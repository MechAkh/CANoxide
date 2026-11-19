//! Settings dialog (spec section 9): a left navigation, one scroll list of
//! sections, a search box, a reset icon on each changed setting, and
//! Restore all defaults / Cancel / Save. Appearance previews live from the
//! draft; Cancel drops the draft. Only settings with working code are here.

use super::{theme, App};
use crate::bus::BusCommand;
use crate::can::Bitrate;
use crate::canopen::sdo::TransferMode;
use crate::canopen::types::DisplayMode;
use crate::settings::{AdapterKind, RowDensity, Settings, Theme, TimestampFormat, TraceColour};
use eframe::egui;
use egui_phosphor::regular as icon;

const SECTIONS: [(&str, &str); 9] = [
    ("Appearance", icon::PALETTE),
    ("Connection", icon::PLUGS),
    ("SDO", icon::ARROWS_LEFT_RIGHT),
    ("LSS", icon::IDENTIFICATION_CARD),
    ("Scan", icon::BROADCAST),
    ("Trace", icon::LIST_DASHES),
    ("Graph", icon::CHART_LINE),
    ("Safety", icon::SHIELD_CHECK),
    ("Data folder", icon::FOLDER),
];

const SCALES: [f32; 8] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
const TRACE_SIZES: [usize; 5] = [10_000, 50_000, 100_000, 500_000, 1_000_000];
const GRAPH_SIZES: [usize; 3] = [10_000, 100_000, 1_000_000];
const WINDOWS: [(f32, &str); 5] = [(5.0, "5 s"), (10.0, "10 s"), (30.0, "30 s"), (60.0, "1 min"), (300.0, "5 min")];

pub(crate) struct SettingsDlg {
    pub draft: Settings,
    pub search: String,
    /// Section to scroll to (clicked in the navigation).
    pub goto: Option<usize>,
    pub current: usize,
}

/// A navigation item (spec section 9): full width, icon then label,
/// left-aligned; the selected item has the selection fill.
fn nav_item(ui: &mut egui::Ui, selected: bool, glyph: &str, name: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(168.0, 32.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact_selectable(&r, selected);
        if selected || r.hovered() {
            ui.painter().rect_filled(rect, 4.0, v.weak_bg_fill);
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let x = rect.left() + 10.0;
        ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, glyph, font.clone(), v.text_color());
        ui.painter().text(egui::pos2(x + 26.0, rect.center().y), egui::Align2::LEFT_CENTER, name, font, v.text_color());
    }
    let label = format!("{glyph}  {name}");
    r.widget_info(move || egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &label));
    r
}

/// One setting row: label and help on the left, the control on the right,
/// and a reset icon when the value is not the default. Returns true when
/// the reset icon was clicked. Hidden (false) when the search does not match.
fn row(ui: &mut egui::Ui, search: &str, label: &str, help: &str, changed: bool, control: impl FnOnce(&mut egui::Ui)) -> bool {
    if !search.is_empty() {
        let s = search.to_lowercase();
        if !label.to_lowercase().contains(&s) && !help.to_lowercase().contains(&s) {
            return false;
        }
    }
    let p = theme::pal(ui);
    let mut reset = false;
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        ui.allocate_ui_with_layout(egui::vec2(260.0, 40.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(260.0);
            ui.label(egui::RichText::new(label).color(p.text_primary));
            if !help.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(help).small().color(p.text_secondary)).wrap());
            }
        });
        ui.add_space(12.0);
        control(ui);
        if changed && ui.add(egui::Button::new(icon::ARROW_COUNTER_CLOCKWISE).frame(false)).on_hover_text("Reset to default").clicked() {
            reset = true;
        }
    });
    reset
}

fn section_title(ui: &mut egui::Ui, title: &str, ic: &str) -> egui::Response {
    ui.add_space(16.0);
    let r = ui.heading(format!("{ic}  {title}"));
    ui.separator();
    r
}

impl App {
    pub(crate) fn open_settings(&mut self) {
        if self.settings_dlg.is_none() {
            self.settings_dlg = Some(SettingsDlg { draft: self.settings.clone(), search: String::new(), goto: None, current: 0 });
        }
    }

    /// Take the draft as the settings, apply them to the worker and the
    /// stores, and save.
    pub(crate) fn apply_settings(&mut self, mut new: Settings) {
        new.sanitize();
        let old = std::mem::replace(&mut self.settings, new);
        let s = &self.settings;
        if s.trace_capacity != old.trace_capacity {
            self.trace.set_capacity(s.trace_capacity);
        }
        if s.signal_capacity != old.signal_capacity {
            if let Ok(mut st) = self.bus.signals.lock() {
                st.set_capacity(s.signal_capacity);
            }
        }
        if s.sdo_timeout_ms != old.sdo_timeout_ms {
            self.bus.send(BusCommand::SetSdoTimeout(s.sdo_timeout_ms));
        }
        if s.sdo_block_size != old.sdo_block_size {
            self.bus.send(BusCommand::SetSdoBlockSize(s.sdo_block_size));
        }
        if s.lss_timeout_ms != old.lss_timeout_ms {
            self.bus.send(BusCommand::SetLssTimeout(s.lss_timeout_ms));
        }
        if s.lss_switch_delay_ms != old.lss_switch_delay_ms {
            self.lssv.delay_ms = s.lss_switch_delay_ms;
        }
        self.style_key = None;
        self.save_settings();
        self.set_status("Settings saved", false);
    }

    pub(crate) fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.settings_dlg.take() else { return };
        let mut save = false;
        let mut cancel = false;
        let def = Settings::default();
        let data_dir = self.data_dir.clone();
        let data_mode = self.data_mode.map(|m| m.label()).unwrap_or("not available");
        let data_error = self.data_error.clone();
        let mut open_dir = false;
        let modal = egui::Modal::new(egui::Id::new("settings_dialog")).show(ctx, |ui| {
            let p = theme::pal(ui);
            ui.set_width(760.0_f32.min(ctx.screen_rect().width() - 40.0).max(560.0));
            let height = 560.0_f32.min(ctx.screen_rect().height() - 80.0).max(380.0);
            // Header.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Settings").text_style(egui::TextStyle::Name("title".into())));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(icon::X).frame(false)).on_hover_text("Close without saving").clicked() {
                        cancel = true;
                    }
                    ui.add(egui::TextEdit::singleline(&mut dlg.search).desired_width(240.0).hint_text(format!("{} Search settings", icon::MAGNIFYING_GLASS)));
                });
            });
            ui.separator();
            let search = dlg.search.trim().to_string();
            ui.horizontal_top(|ui| {
                // Navigation.
                egui::Frame::new().fill(p.bg_bar).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                    ui.set_width(168.0);
                    ui.set_min_height(height - 70.0);
                    ui.vertical(|ui| {
                        for (i, (name, ic)) in SECTIONS.iter().enumerate() {
                            let r = nav_item(ui, dlg.current == i, ic, name);
                            if r.clicked() {
                                dlg.goto = Some(i);
                                dlg.current = i;
                            }
                        }
                    });
                });
                // Content (a column: the scroll area takes the layout of its parent).
                ui.vertical(|ui| egui::ScrollArea::vertical().id_salt("settings_scroll").max_height(height - 70.0).auto_shrink([false, false]).show(ui, |ui| {
                    let d = &mut dlg.draft;
                    let goto = dlg.goto.take();
                    let mut tops = [0.0f32; 9];
                    let mut sec = |ui: &mut egui::Ui, i: usize| {
                        let r = section_title(ui, SECTIONS[i].0, SECTIONS[i].1);
                        tops[i] = r.rect.top();
                        if goto == Some(i) {
                            r.scroll_to_me(Some(egui::Align::TOP));
                        }
                    };
                    // ---- Appearance
                    sec(ui, 0);
                    if row(ui, &search, "Theme", "System follows Windows.", d.theme != def.theme, |ui| {
                        for (t, l) in [(Theme::System, "System"), (Theme::Dark, "Dark"), (Theme::Light, "Light")] {
                            ui.selectable_value(&mut d.theme, t, l);
                        }
                    }) {
                        d.theme = def.theme;
                    }
                    if row(ui, &search, "UI scale", "Size of the whole window content.", d.ui_scale != def.ui_scale, |ui| {
                        egui::ComboBox::from_id_salt("set_scale").selected_text(format!("{:.0} %", d.ui_scale * 100.0)).show_ui(ui, |ui| {
                            for s in SCALES {
                                ui.selectable_value(&mut d.ui_scale, s, format!("{:.0} %", s * 100.0));
                            }
                        });
                    }) {
                        d.ui_scale = def.ui_scale;
                    }
                    if row(ui, &search, "Font size", "Body text size in points.", d.font_offset != def.font_offset, |ui| {
                        let mut pt = 14 + d.font_offset as i32;
                        if ui.add(egui::DragValue::new(&mut pt).range(12..=18).suffix(" pt")).changed() {
                            d.font_offset = (pt - 14) as i8;
                        }
                    }) {
                        d.font_offset = def.font_offset;
                    }
                    if row(ui, &search, "Row density", "Height of table rows.", d.row_density != def.row_density, |ui| {
                        ui.selectable_value(&mut d.row_density, RowDensity::Compact, "Compact");
                        ui.selectable_value(&mut d.row_density, RowDensity::Comfortable, "Comfortable");
                    }) {
                        d.row_density = def.row_density;
                    }
                    if row(ui, &search, "Zebra stripes in tables", "", d.zebra != def.zebra, |ui| {
                        ui.checkbox(&mut d.zebra, "");
                    }) {
                        d.zebra = def.zebra;
                    }
                    if row(ui, &search, "Reduce colour", "Chips and pills outlined, no fill.", d.reduce_colour != def.reduce_colour, |ui| {
                        ui.checkbox(&mut d.reduce_colour, "");
                    }) {
                        d.reduce_colour = def.reduce_colour;
                    }
                    // ---- Connection
                    sec(ui, 1);
                    if row(ui, &search, "Default adapter", "Used by Connect.", d.adapter != def.adapter, |ui| {
                        egui::ComboBox::from_id_salt("set_adapter").selected_text(d.adapter.label()).show_ui(ui, |ui| {
                            for a in [AdapterKind::Simulator, AdapterKind::Peak] {
                                ui.selectable_value(&mut d.adapter, a, a.label());
                            }
                        });
                    }) {
                        d.adapter = def.adapter;
                    }
                    if row(ui, &search, "Default bitrate", "", d.bitrate_kbit != def.bitrate_kbit, |ui| {
                        let cur = Bitrate::from_kbit(d.bitrate_kbit).map(|b| b.label()).unwrap_or_else(|| format!("{} kbit/s", d.bitrate_kbit));
                        egui::ComboBox::from_id_salt("set_br").selected_text(cur).show_ui(ui, |ui| {
                            for b in Bitrate::ALL {
                                ui.selectable_value(&mut d.bitrate_kbit, b.kbit(), b.label());
                            }
                        });
                    }) {
                        d.bitrate_kbit = def.bitrate_kbit;
                    }
                    if row(ui, &search, "Listen-only on start", "Connect without sending anything.", d.listen_only != def.listen_only, |ui| {
                        ui.checkbox(&mut d.listen_only, "");
                    }) {
                        d.listen_only = def.listen_only;
                    }
                    if row(ui, &search, "Connect automatically on start", "Connects with the defaults above; sends nothing by itself.", d.connect_on_start != def.connect_on_start, |ui| {
                        ui.checkbox(&mut d.connect_on_start, "");
                    }) {
                        d.connect_on_start = def.connect_on_start;
                    }
                    // ---- SDO
                    sec(ui, 2);
                    if row(ui, &search, "Timeout", "Wait for an SDO answer.", d.sdo_timeout_ms != def.sdo_timeout_ms, |ui| {
                        ui.add(egui::DragValue::new(&mut d.sdo_timeout_ms).range(50..=10_000).suffix(" ms"));
                    }) {
                        d.sdo_timeout_ms = def.sdo_timeout_ms;
                    }
                    if row(ui, &search, "Block size (block transfer)", "Segments per block the client offers.", d.sdo_block_size != def.sdo_block_size, |ui| {
                        ui.add(egui::DragValue::new(&mut d.sdo_block_size).range(1..=127));
                    }) {
                        d.sdo_block_size = def.sdo_block_size;
                    }
                    if row(ui, &search, "Transfer mode", "Default of the object dictionary view.", d.sdo_transfer_mode != def.sdo_transfer_mode, |ui| {
                        egui::ComboBox::from_id_salt("set_tm").selected_text(d.sdo_transfer_mode.label()).show_ui(ui, |ui| {
                            for m in TransferMode::ALL {
                                ui.selectable_value(&mut d.sdo_transfer_mode, m, m.label());
                            }
                        });
                    }) {
                        d.sdo_transfer_mode = def.sdo_transfer_mode;
                    }
                    // ---- LSS
                    sec(ui, 3);
                    if row(ui, &search, "LSS timeout", "Wait for a confirmed LSS service.", d.lss_timeout_ms != def.lss_timeout_ms, |ui| {
                        ui.add(egui::DragValue::new(&mut d.lss_timeout_ms).range(50..=10_000).suffix(" ms"));
                    }) {
                        d.lss_timeout_ms = def.lss_timeout_ms;
                    }
                    if row(ui, &search, "Default switch delay", "Activate bit timing.", d.lss_switch_delay_ms != def.lss_switch_delay_ms, |ui| {
                        ui.add(egui::DragValue::new(&mut d.lss_switch_delay_ms).range(0..=65_535).suffix(" ms"));
                    }) {
                        d.lss_switch_delay_ms = def.lss_switch_delay_ms;
                    }
                    // ---- Scan
                    sec(ui, 4);
                    if row(ui, &search, "Node ID range", "First and last node of a scan.", d.scan_first != def.scan_first || d.scan_last != def.scan_last, |ui| {
                        ui.add(egui::DragValue::new(&mut d.scan_first).range(1..=127));
                        ui.label("to");
                        ui.add(egui::DragValue::new(&mut d.scan_last).range(1..=127));
                    }) {
                        d.scan_first = def.scan_first;
                        d.scan_last = def.scan_last;
                    }
                    if row(ui, &search, "Timeout per node", "", d.scan_timeout_ms != def.scan_timeout_ms, |ui| {
                        ui.add(egui::DragValue::new(&mut d.scan_timeout_ms).range(10..=2_000).suffix(" ms"));
                    }) {
                        d.scan_timeout_ms = def.scan_timeout_ms;
                    }
                    if row(ui, &search, "PDO read stops after", "Missing PDOs in a row that end a PDO read.", d.pdo_scan_stop != def.pdo_scan_stop, |ui| {
                        ui.add(egui::DragValue::new(&mut d.pdo_scan_stop).range(1..=512));
                    }) {
                        d.pdo_scan_stop = def.pdo_scan_stop;
                    }
                    // ---- Trace
                    sec(ui, 5);
                    if row(ui, &search, "Buffer size", "Frames kept in memory.", d.trace_capacity != def.trace_capacity, |ui| {
                        egui::ComboBox::from_id_salt("set_tcap").selected_text(format!("{} frames", d.trace_capacity)).show_ui(ui, |ui| {
                            for n in TRACE_SIZES {
                                ui.selectable_value(&mut d.trace_capacity, n, format!("{n} frames"));
                            }
                        });
                    }) {
                        d.trace_capacity = def.trace_capacity;
                    }
                    if row(ui, &search, "Timestamp format", "", d.timestamp_format != def.timestamp_format, |ui| {
                        ui.selectable_value(&mut d.timestamp_format, TimestampFormat::Relative, "Relative");
                        ui.selectable_value(&mut d.timestamp_format, TimestampFormat::Delta, "Delta");
                    }) {
                        d.timestamp_format = def.timestamp_format;
                    }
                    if row(ui, &search, "Timestamp resolution", "", d.timestamp_us != def.timestamp_us, |ui| {
                        ui.selectable_value(&mut d.timestamp_us, false, "ms");
                        ui.selectable_value(&mut d.timestamp_us, true, "µs");
                    }) {
                        d.timestamp_us = def.timestamp_us;
                    }
                    if row(ui, &search, "Default data view", "Data column of a new trace tab.", d.trace_display != def.trace_display, |ui| {
                        for m in DisplayMode::ALL {
                            ui.selectable_value(&mut d.trace_display, m, m.label());
                        }
                    }) {
                        d.trace_display = def.trace_display;
                    }
                    if row(ui, &search, "Colour by type", "", d.trace_colour != def.trace_colour, |ui| {
                        egui::ComboBox::from_id_salt("set_tcol").selected_text(d.trace_colour.label()).show_ui(ui, |ui| {
                            for c in TraceColour::ALL {
                                ui.selectable_value(&mut d.trace_colour, c, c.label());
                            }
                        });
                    }) {
                        d.trace_colour = def.trace_colour;
                    }
                    if row(ui, &search, "Autoscroll by default", "", d.trace_autoscroll != def.trace_autoscroll, |ui| {
                        ui.checkbox(&mut d.trace_autoscroll, "");
                    }) {
                        d.trace_autoscroll = def.trace_autoscroll;
                    }
                    if row(ui, &search, "Messages: change fade time", "Seconds over which the tint of a changed data byte fades.", d.msg_fade_s != def.msg_fade_s, |ui| {
                        ui.add(egui::DragValue::new(&mut d.msg_fade_s).range(0.2..=10.0).speed(0.1).suffix(" s"));
                    }) {
                        d.msg_fade_s = def.msg_fade_s;
                    }
                    if row(ui, &search, "Messages: mute noisy bytes", "No tint on a byte that changes in nearly every frame (for example a counter).", d.msg_mute_noisy != def.msg_mute_noisy, |ui| {
                        ui.checkbox(&mut d.msg_mute_noisy, "");
                    }) {
                        d.msg_mute_noisy = def.msg_mute_noisy;
                    }
                    // ---- Graph
                    sec(ui, 6);
                    if row(ui, &search, "Buffer size", "Points kept per signal.", d.signal_capacity != def.signal_capacity, |ui| {
                        egui::ComboBox::from_id_salt("set_gcap").selected_text(format!("{} points", d.signal_capacity)).show_ui(ui, |ui| {
                            for n in GRAPH_SIZES {
                                ui.selectable_value(&mut d.signal_capacity, n, format!("{n} points"));
                            }
                        });
                    }) {
                        d.signal_capacity = def.signal_capacity;
                    }
                    if row(ui, &search, "Default time window", "", d.graph_window_s != def.graph_window_s, |ui| {
                        let cur = WINDOWS.iter().find(|w| w.0 == d.graph_window_s).map(|w| w.1.to_string()).unwrap_or_else(|| format!("{} s", d.graph_window_s));
                        egui::ComboBox::from_id_salt("set_gwin").selected_text(cur).show_ui(ui, |ui| {
                            for (v, l) in WINDOWS {
                                ui.selectable_value(&mut d.graph_window_s, v, l);
                            }
                        });
                    }) {
                        d.graph_window_s = def.graph_window_s;
                    }
                    if row(ui, &search, "Line width", "", d.graph_line_width != def.graph_line_width, |ui| {
                        ui.add(egui::DragValue::new(&mut d.graph_line_width).range(1.0..=3.0).speed(0.1));
                    }) {
                        d.graph_line_width = def.graph_line_width;
                    }
                    if row(ui, &search, "Show cursor readout", "", d.graph_cursor_readout != def.graph_cursor_readout, |ui| {
                        ui.checkbox(&mut d.graph_cursor_readout, "");
                    }) {
                        d.graph_cursor_readout = def.graph_cursor_readout;
                    }
                    // ---- Safety
                    sec(ui, 7);
                    if row(ui, &search, "Confirm before SDO write", "", d.confirm_before_write != def.confirm_before_write, |ui| {
                        ui.checkbox(&mut d.confirm_before_write, "");
                    }) {
                        d.confirm_before_write = def.confirm_before_write;
                    }
                    if row(ui, &search, "Confirm before NMT to all nodes", "", d.confirm_nmt_all != def.confirm_nmt_all, |ui| {
                        ui.checkbox(&mut d.confirm_nmt_all, "");
                    }) {
                        d.confirm_nmt_all = def.confirm_nmt_all;
                    }
                    if row(ui, &search, "Confirm before PDO apply", "Also enable / disable.", d.confirm_pdo_apply != def.confirm_pdo_apply, |ui| {
                        ui.checkbox(&mut d.confirm_pdo_apply, "");
                    }) {
                        d.confirm_pdo_apply = def.confirm_pdo_apply;
                    }
                    if row(ui, &search, "Confirm before LSS change", "Node-ID, bit rate, activate and store.", d.confirm_lss != def.confirm_lss, |ui| {
                        ui.checkbox(&mut d.confirm_lss, "");
                    }) {
                        d.confirm_lss = def.confirm_lss;
                    }
                    if row(ui, &search, "Confirm before DCF restore", "", d.confirm_dcf_restore != def.confirm_dcf_restore, |ui| {
                        ui.checkbox(&mut d.confirm_dcf_restore, "");
                    }) {
                        d.confirm_dcf_restore = def.confirm_dcf_restore;
                    }
                    row(ui, &search, "Block all transmit in listen-only", "Always on.", false, |ui| {
                        let mut on = true;
                        ui.add_enabled(false, egui::Checkbox::new(&mut on, ""));
                    });
                    // ---- Data folder
                    sec(ui, 8);
                    row(ui, &search, "Path", "Fixed by the portable data folder rule.", false, |ui| {
                        let t = match (&data_dir, &data_error) {
                            (_, Some(e)) => e.clone(),
                            (Some(d), None) => format!("{} ({data_mode})", d.display()),
                            (None, None) => "-".to_string(),
                        };
                        ui.add(egui::Label::new(egui::RichText::new(t).monospace()).wrap());
                        if ui.add_enabled(data_dir.is_some(), egui::Button::new("Open")).clicked() {
                            open_dir = true;
                        }
                    });
                    // The nav follows the scroll position.
                    let clip_top = ui.clip_rect().top();
                    if goto.is_none() {
                        let mut cur = 0;
                        for (i, t) in tops.iter().enumerate() {
                            if *t <= clip_top + 40.0 {
                                cur = i;
                            }
                        }
                        dlg.current = cur;
                    }
                }));
            });
            ui.separator();
            // Footer.
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Restore all defaults").frame(false)).clicked() {
                    let keep = (dlg.draft.window_width, dlg.draft.window_height, dlg.draft.active_device.clone(), dlg.draft.saved_frames.clone());
                    dlg.draft = Settings::default();
                    (dlg.draft.window_width, dlg.draft.window_height, dlg.draft.active_device, dlg.draft.saved_frames) = keep;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("Save").color(p.on_accent)).fill(p.accent_fill)).clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        });
        if modal.should_close() {
            cancel = true;
        }
        if open_dir {
            if let Some(d) = &self.data_dir {
                super::open_folder(d);
            }
        }
        if save {
            self.apply_settings(dlg.draft);
        } else if cancel {
            self.style_key = None;
        } else {
            self.settings_dlg = Some(dlg);
        }
    }
}
