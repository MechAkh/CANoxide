//! Messages view (0.3): one row per COB-ID with count, cycle time, DLC, one
//! cell per data byte and age. A byte that changes gets a tint (blue: the
//! value went up, red: down) that fades over the fade time. The detail pane
//! shows a bit grid with the toggle heat and the mapped PDO signals; a byte
//! or bit range goes to the Graph as a raw-frame-bits signal. The
//! statistics live in `crate::msgstats`; this module only draws.

use super::{analysis_view, graph, theme, App};
use crate::analysis::BitRef;
use crate::can::CanFrame;
use crate::usersig::{fmt_value, UserSignal};
use crate::canopen::pdo::MapEntry;
use crate::msgstats::{tint_strength, Change, MsgKey};
use crate::settings::DetailPos;
use crate::signals::{RawKind, SignalDef, SignalSource};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum MsgSort {
    /// COB-ID ascending (the default).
    #[default]
    CobId,
    /// Most frames first.
    Count,
    /// Newest frame first.
    Age,
}

impl MsgSort {
    const ALL: [MsgSort; 3] = [MsgSort::CobId, MsgSort::Count, MsgSort::Age];
    fn label(self) -> &'static str {
        match self {
            MsgSort::CobId => "COB-ID",
            MsgSort::Count => "Count",
            MsgSort::Age => "Age",
        }
    }
}

/// The node and the mapping of a PDO.
type NodeMapping = (u8, Vec<MapEntry>);

/// A mapped object of the PDO of the selected row.
pub(crate) struct MappedSig {
    offset: u32,
    bits: u32,
    text: String,
    /// The Graph signal of this object (as the Add signal picker makes it).
    def: SignalDef,
}

#[derive(Default)]
pub(crate) struct MessagesView {
    pub filter: String,
    filter_lower: String,
    pub sort: MsgSort,
    pub selected: Option<MsgKey>,
    /// The drawn rows (reused; sorted again only when the statistics, the
    /// filter or the sort change).
    order: Vec<MsgKey>,
    order_gen: Option<u64>,
    dirty: bool,
    /// The newest frame time stamp and when it arrived: the clock of the
    /// age and of the tint fade.
    clock: Option<(u64, Instant)>,
    fixed_clock: Option<u64>,
    /// The selected bits of the detail pane (anchor, end; bit k = byte
    /// k / 8, bit k % 8).
    bit_sel: Option<(u32, u32)>,
    /// The selection is a big-endian field (grid reading order).
    pub sel_be: bool,
    /// A drag over the bit grid is running.
    bit_drag: bool,
    /// The PDO mapping that `mapped` was built from.
    pdo_cache: Option<(MsgKey, Option<NodeMapping>)>,
    mapped: Vec<MappedSig>,
}

impl MessagesView {
    /// The statistics were reset (by the user or on connect).
    pub(crate) fn on_reset(&mut self) {
        self.clock = None;
        self.dirty = true;
    }

    /// Frames arrived; `latest_us` is the newest time stamp.
    pub(crate) fn on_frames(&mut self, latest_us: u64) {
        if self.clock.is_none_or(|(t, _)| latest_us != t) {
            self.clock = Some((latest_us, Instant::now()));
        }
    }

    /// Replay: the clock is the replay position (it does not run on while
    /// the replay is paused). `None`: the live clock.
    pub(crate) fn set_fixed_clock(&mut self, us: Option<u64>) {
        self.fixed_clock = us;
    }

    fn now_us(&self) -> u64 {
        if let Some(t) = self.fixed_clock {
            return t;
        }
        self.clock.map_or(0, |(t, at)| t + at.elapsed().as_micros() as u64)
    }

    /// The selected field (start bit, length). Little endian: the bits
    /// a..b in bit order (start = the lowest). Big endian: the cells a..b
    /// in grid reading order (bytes down, bits 7..0 across; start = the
    /// first cell, the MSB).
    fn selection(&self) -> Option<(u32, u32)> {
        let (a, b) = self.bit_sel?;
        if self.sel_be {
            let (ga, gb) = (grid_pos(a), grid_pos(b));
            Some((if ga <= gb { a } else { b }, ga.max(gb) - ga.min(gb) + 1))
        } else {
            Some((a.min(b), a.max(b) - a.min(b) + 1))
        }
    }

    /// Bit k is in the selection.
    fn in_selection(&self, k: u32) -> bool {
        let Some((a, b)) = self.bit_sel else { return false };
        if self.sel_be {
            let (ga, gb, g) = (grid_pos(a), grid_pos(b), grid_pos(k));
            g >= ga.min(gb) && g <= ga.max(gb)
        } else {
            k >= a.min(b) && k <= a.max(b)
        }
    }
}

/// The reading-order position of bit k in the grid (byte rows, bit 7 left).
fn grid_pos(k: u32) -> u32 {
    (k / 8) * 8 + 7 - k % 8
}

/// "00".."FF" without an allocation per cell.
const HEX: [u8; 512] = {
    let digits = b"0123456789ABCDEF";
    let mut t = [0u8; 512];
    let mut i = 0;
    while i < 256 {
        t[2 * i] = digits[i >> 4];
        t[2 * i + 1] = digits[i & 15];
        i += 1;
    }
    t
};

/// A data byte as text: two hex digits, or decimal.
fn byte_text(b: u8, dec: bool) -> String {
    if dec {
        b.to_string()
    } else {
        hex2(b).to_string()
    }
}

fn hex2(b: u8) -> &'static str {
    let i = b as usize * 2;
    std::str::from_utf8(&HEX[i..i + 2]).unwrap_or("??")
}

const BYTE_HEAD: [&str; 8] = ["B0", "B1", "B2", "B3", "B4", "B5", "B6", "B7"];
const BYTE_NAME: [&str; 8] = ["Byte 0", "Byte 1", "Byte 2", "Byte 3", "Byte 4", "Byte 5", "Byte 6", "Byte 7"];
const BIT_HEAD: [&str; 8] = ["7", "6", "5", "4", "3", "2", "1", "0"];
/// The hover text on the selection that names Shift + click.
const SHIFT_TIP: &str = "Shift + click on a bit extends the selection from its first bit to that bit; Shift + click on a Byte label extends it to whole bytes";

fn key_text(k: MsgKey) -> String {
    k.to_string()
}

fn cycle_text(period_us: Option<f64>) -> String {
    match period_us {
        Some(p) if p < 1_000_000.0 => format!("{:.1} ms", p / 1000.0),
        Some(p) => format!("{:.2} s", p / 1e6),
        None => "-".to_string(),
    }
}

fn age_text(age_us: u64) -> String {
    let s = age_us as f64 / 1e6;
    if s < 60.0 {
        format!("{s:.1} s")
    } else {
        format!("{:.0} min", s / 60.0)
    }
}

enum DetailAction {
    /// Close the detail pane (deselect the row).
    Close,
    ResetRow,
    AddToGraph(u32, u32),
    Define(u32, u32),
    Similar(u32),
    EditUser(usize),
    DeleteUser(usize),
    /// Show the bit view on this side.
    SetPos(DetailPos),
    /// A signal of the value list to the Graph, or to a dashboard widget.
    SigToGraph(SignalDef),
    SigToDash(SignalDef),
}

/// The selection (anchor, end) of whole bytes from the anchor byte `ab` to
/// byte `b`, in either direction; the anchor stays in byte `ab`. LE: bit
/// order; BE: grid reading order (bit 7 of a byte first).
pub(crate) fn byte_range(ab: u32, b: u32, be: bool) -> (u32, u32) {
    match (be, b >= ab) {
        (false, true) => (ab * 8, b * 8 + 7),
        (false, false) => (ab * 8 + 7, b * 8),
        (true, true) => (ab * 8 + 7, b * 8),
        (true, false) => (ab * 8, b * 8 + 7),
    }
}

/// The indices (into `signals`) of the user signals of `key` with a bit in
/// the selection (a, z) (`be`: in grid reading order).
pub(crate) fn signals_overlapping_selection(signals: &[UserSignal], key: MsgKey, sel: Option<(u32, u32)>, be: bool, j1939: bool) -> Vec<usize> {
    let Some((a, z)) = sel else { return Vec::new() };
    signals.iter().enumerate().filter(|(_, s)| s.matches_frame(key.id, key.extended, j1939) && s.bit_list().iter().any(|k| sel_contains(be, a, z, *k))).map(|(i, _)| i).collect()
}

/// The small icon buttons "to Graph" and "to Dashboard" of a signal row
/// (also on the Find signal candidates): (Graph clicked, Dashboard clicked).
pub(crate) fn signal_buttons(ui: &mut egui::Ui, name: &str) -> (bool, bool) {
    let g = theme::icon_button(ui, true, icon::CHART_LINE, &format!("Add {name} to the Graph"), "Add this signal to the Graph").clicked();
    let d = theme::icon_button(ui, true, icon::GAUGE, &format!("Add {name} to the Dashboard"), "Add this signal to the Dashboard (a new widget)").clicked();
    (g, d)
}

fn signal_action(ui: &mut egui::Ui, def: &SignalDef, action: &mut Option<DetailAction>) {
    match signal_buttons(ui, &def.name) {
        (true, _) => *action = Some(DetailAction::SigToGraph(def.clone())),
        (_, true) => *action = Some(DetailAction::SigToDash(def.clone())),
        _ => {}
    }
}

impl App {
    /// The clock of the Messages view (tests).
    #[cfg(test)]
    pub(crate) fn msgv_now_us(&self) -> u64 {
        self.msgv.now_us()
    }

    pub(crate) fn messages_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        // Toolbar.
        theme::toolbar(ui, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.msgv.filter).hint_text("Filter: COB-ID, type, node").desired_width(200.0));
            if r.changed() {
                self.msgv.filter_lower = self.msgv.filter.trim().to_lowercase();
                self.msgv.dirty = true;
            }
            let before = self.msgv.sort;
            theme::toolbar_combo(ui, "Sort by", Some("Sort by"), self.msgv.sort.label(), None, |ui| {
                for s in MsgSort::ALL {
                    ui.selectable_value(&mut self.msgv.sort, s, s.label());
                }
            });
            if self.msgv.sort != before {
                self.msgv.dirty = true;
            }
            ui.separator();
            let mut dec = self.settings.msg_decimal;
            let hex = ui.selectable_value(&mut dec, false, "Hex").on_hover_text("Data bytes in hex");
            let d = ui.selectable_value(&mut dec, true, "Decimal").on_hover_text("Data bytes in decimal");
            if hex.changed() || d.changed() {
                self.settings.msg_decimal = dec;
                self.mark_settings();
            }
            let mut mute = self.settings.msg_mute_noisy;
            if ui.checkbox(&mut mute, "Mute noisy bytes").on_hover_text("No tint on a byte that changes in nearly every frame (for example a counter)").changed() {
                self.settings.msg_mute_noisy = mute;
                self.mark_settings();
            }
            if ui.button("Reset all statistics").on_hover_text("Forget every row; rows come back with the next frames").clicked() {
                self.msgs.reset();
                self.msgv.on_reset();
            }
            if ui.button("Import DBC...").on_hover_text("Import the signals of a DBC file as user signals").clicked() {
                self.import_dbc_dialog();
            }
            ui.label(egui::RichText::new(format!("{} IDs", self.msgs.len())).color(p.text_secondary));
            if self.msgs.ignored() > 0 {
                ui.label(egui::RichText::new(format!("{} frames of further IDs not counted (limit {})", self.msgs.ignored(), crate::msgstats::MAX_IDS)).color(p.warning));
            }
        });
        ui.add_space(4.0);
        if self.msgs.is_empty() {
            ui.label(egui::RichText::new("No frames yet. Connect to see one row per COB-ID.").color(p.text_secondary));
            return;
        }
        // Rows: sorted again only when something changed.
        let gen = self.msgs.generation();
        if self.msgv.dirty || self.msgv.order_gen != Some(gen) {
            let v = &mut self.msgv;
            v.order.clear();
            v.order.extend(self.msgs.entries().filter(|e| e.matches(&v.filter_lower)).map(|e| e.key));
            let m = &self.msgs;
            match v.sort {
                MsgSort::CobId => v.order.sort_unstable(),
                MsgSort::Count => v.order.sort_unstable_by(|a, b| {
                    let (x, y) = (m.get(*a).map_or(0, |e| e.count), m.get(*b).map_or(0, |e| e.count));
                    y.cmp(&x).then(a.cmp(b))
                }),
                MsgSort::Age => v.order.sort_unstable_by(|a, b| {
                    let (x, y) = (m.get(*a).map_or(0, |e| e.last_us), m.get(*b).map_or(0, |e| e.last_us));
                    y.cmp(&x).then(a.cmp(b))
                }),
            }
            v.order_gen = Some(gen);
            v.dirty = false;
        }
        if self.msgv.selected.is_some_and(|k| self.msgs.get(k).is_none()) {
            self.msgv.selected = None;
        }
        let now = self.msgv.now_us();
        let fade_us = (self.settings.msg_fade_s.max(0.0) * 1e6) as u64;
        let mut fading = false;
        // Detail pane of the selected row.
        self.an.sparks.tick();
        // The bit view is a resizable panel of the tab (own id per side, so
        // that each side keeps its size); the table takes the rest.
        let pos = self.settings.msg_detail_pos;
        if let Some(key) = self.msgv.selected {
            let f = match pos {
                DetailPos::Bottom => egui::TopBottomPanel::bottom("msg_detail").resizable(true).default_height(460.0).min_height(120.0).show_inside(ui, |ui| self.message_detail(ui, key, now, fade_us)).inner,
                DetailPos::Top => egui::TopBottomPanel::top("msg_detail_top").resizable(true).default_height(460.0).min_height(120.0).show_inside(ui, |ui| self.message_detail(ui, key, now, fade_us)).inner,
                DetailPos::Left => {
                    let max = (ui.available_width() - 200.0).max(260.0);
                    egui::SidePanel::left("msg_detail_left").resizable(true).default_width(560.0).min_width(260.0).max_width(max).show_inside(ui, |ui| self.message_detail(ui, key, now, fade_us)).inner
                }
                DetailPos::Right => {
                    let max = (ui.available_width() - 200.0).max(260.0);
                    egui::SidePanel::right("msg_detail_right").resizable(true).default_width(560.0).min_width(260.0).max_width(max).show_inside(ui, |ui| self.message_detail(ui, key, now, fade_us)).inner
                }
            };
            fading |= f;
        }
        let side = self.msgv.selected.is_some() && matches!(pos, DetailPos::Left | DetailPos::Right);
        let row_h = self.settings.row_density.row_height();
        let mute = self.settings.msg_mute_noisy;
        let dec = self.settings.msg_decimal;
        let reduce = self.settings.reduce_colour;
        let zebra = self.settings.zebra;
        let mut clicked: Option<MsgKey> = None;
        let selected = self.msgv.selected;
        let msgs = &self.msgs;
        let order = &self.msgv.order;
        let dctx = &self.decode_ctx;
        // A click on a cell selects the row (a selectable label would take
        // the click for text selection).
        ui.style_mut().interaction.selectable_labels = false;
        // The column lines of the table stop above the detail pane.
        let clip = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        ui.set_clip_rect(clip);
        let mut table = |ui: &mut egui::Ui| {
        let mut tb = TableBuilder::new(ui)
            .id_salt("messages_table")
            .striped(zebra)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(76.0).at_least(56.0))
            .column(Column::initial(110.0).at_least(60.0).clip(true))
            .column(Column::initial(40.0).at_least(30.0))
            .column(Column::initial(70.0).at_least(44.0))
            .column(Column::initial(72.0).at_least(50.0))
            .column(Column::initial(34.0).at_least(28.0));
        for _ in 0..8 {
            tb = tb.column(Column::exact(32.0));
        }
        tb.column(Column::initial(64.0).at_least(50.0))
            .column(Column::remainder().at_least(80.0).clip(true))
            .min_scrolled_height(0.0)
            .header(row_h, |mut h| {
                for t in ["COB-ID", "Type", "Node", "Count", "Cycle", "DLC"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
                for t in BYTE_HEAD {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
                h.col(|ui| {
                    ui.strong("Age");
                });
                h.col(|ui| {
                    ui.strong("Decoded");
                });
            })
            .body(|body| {
                body.rows(row_h, order.len(), |mut row| {
                    let i = row.index();
                    let Some(e) = order.get(i).and_then(|k| msgs.get(*k)) else { return };
                    let sel = selected == Some(e.key);
                    row.set_selected(sel);
                    let row_bg = if sel {
                        p.bg_selection
                    } else if zebra && i % 2 == 1 {
                        p.bg_row_alt
                    } else {
                        p.bg_panel
                    };
                    row.col(|ui| {
                        ui.monospace(key_text(e.key));
                    });
                    row.col(|ui| {
                        let (fg, fill) = theme::class_colours(p, e.class);
                        theme::pill(ui, e.label.as_str(), fg, fill, reduce);
                    });
                    row.col(|ui| {
                        if let Some(n) = e.node {
                            ui.label(n.to_string());
                        }
                    });
                    row.col(|ui| {
                        ui.monospace(e.count.to_string());
                    });
                    row.col(|ui| {
                        ui.monospace(cycle_text(e.period_us));
                    });
                    row.col(|ui| {
                        ui.monospace(if e.rtr { "RTR".to_string() } else { e.dlc.to_string() });
                    });
                    let n = if e.rtr { 0 } else { e.dlc.min(8) as usize };
                    for b in 0..8 {
                        row.col(|ui| {
                            if b >= n {
                                return;
                            }
                            let s = &e.bytes[b];
                            let mut k = tint_strength(s.last_change_us, now, fade_us);
                            if mute && e.is_noisy(b) {
                                k = 0.0;
                            }
                            let up = s.dir == Some(Change::Up);
                            if k > 0.0 {
                                fading = true;
                                ui.painter().rect_filled(ui.max_rect(), 0.0, theme::mix(row_bg, theme::change_tint(p, up), k));
                            }
                            ui.label(egui::RichText::new(byte_text(e.data[b], dec)).monospace().color(p.text_primary));
                            if reduce && k > 0.0 {
                                ui.label(egui::RichText::new(if up { icon::CARET_UP } else { icon::CARET_DOWN }).small().color(p.text_primary));
                            }
                        });
                    }
                    row.col(|ui| {
                        ui.monospace(age_text(now.saturating_sub(e.last_us)));
                    });
                    // Decoded text of the newest frame (visible rows only).
                    row.col(|ui| {
                        let f = CanFrame { id: e.key.id, extended: e.key.extended, rtr: e.rtr, dlc: e.dlc, data: e.data, timestamp_us: e.last_us, dir: e.dir };
                        ui.add(egui::Label::new(crate::canopen::decode::row_text(&f, dctx)).truncate());
                    });
                    if row.response().clicked() {
                        clicked = Some(e.key);
                    }
                });
            });
        };
        // Beside a side panel the table may be narrower than its columns:
        // it scrolls sideways.
        if side {
            egui::ScrollArea::horizontal().id_salt("messages_hscroll").show(ui, |ui| table(ui));
        } else {
            table(ui);
        }
        if let Some(k) = clicked {
            // A click on the selected row closes the detail pane.
            self.msgv.bit_sel = None;
            self.msgv.selected = if self.msgv.selected == Some(k) { None } else { Some(k) };
        }
        // Repaint only while a tint fades (frames repaint on their own).
        if fading {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }

    /// Rebuild the mapped signals of the selected row when its PDO mapping
    /// changed (no allocation while it stays the same).
    fn refresh_mapped(&mut self, key: MsgKey) {
        let cur = if key.extended { None } else { self.decode_ctx.pdo(key.id).map(|(n, c)| (*n, c.mapping.as_slice())) };
        let same = match &self.msgv.pdo_cache {
            Some((k, c)) if *k == key => match (c, cur) {
                (None, None) => true,
                (Some((n, m)), Some((cn, cm))) => *n == cn && m.as_slice() == cm,
                _ => false,
            },
            _ => false,
        };
        if same {
            return;
        }
        let owned = cur.map(|(n, m)| (n, m.to_vec()));
        let pdo_label = if key.extended { None } else { self.decode_ctx.pdo(key.id).map(|(_, c)| c.label()) };
        self.msgv.mapped.clear();
        if let Some((node, map)) = &owned {
            let od = self.od_of_node(*node);
            let mut off = 0u32;
            for m in map {
                let bits = m.bits as u32;
                if !m.is_dummy() && bits > 0 {
                    let name = od.as_ref().and_then(|o| o.name_of(m.index, m.sub)).unwrap_or_default();
                    let text = format!("{:04X}:{:02X} {name}", m.index, m.sub).trim_end().to_string();
                    // The name that the "Mapped object" choice of the Add
                    // signal picker gives ("N1 TPDO1 6000:01 ...").
                    let sig_name = match &pdo_label {
                        Some(l) => format!("N{node} {l} {text}"),
                        None => format!("N{node} {text}"),
                    };
                    let def = SignalDef::new(sig_name, SignalSource::Pdo { node: *node, index: m.index, sub: m.sub });
                    self.msgv.mapped.push(MappedSig { offset: off, bits, text, def });
                }
                off += bits;
            }
        }
        self.msgv.pdo_cache = Some((key, owned));
    }

    /// The definition of the Graph signal with the source of `def`, when
    /// the Graph shows one.
    fn graph_signal_like(&self, def: &SignalDef) -> Option<SignalDef> {
        let s = self.bus.signals.lock().ok()?;
        let d = s.graph_signals().find(|g| g.def.source == def.source).map(|g| g.def.clone());
        d
    }

    /// Add a signal to the Graph, unless the Graph shows it already.
    pub(crate) fn signal_to_graph(&mut self, def: SignalDef) {
        if let Some(g) = self.graph_signal_like(&def) {
            self.set_status(format!("Signal {} is already in the Graph", g.name), false);
            return;
        }
        let name = def.name.clone();
        if self.add_signal(def).is_some() {
            self.set_status(format!("Signal {name} added to the Graph"), false);
        }
    }

    /// A dashboard widget for a signal: the signal goes to the Graph first
    /// (the dashboard shares the Graph signals), then the widget dialog
    /// opens as after Add signal, and the Dashboard comes to the front.
    pub(crate) fn signal_to_dashboard(&mut self, def: SignalDef) {
        let def = match self.graph_signal_like(&def) {
            Some(g) => g,
            None => {
                self.add_signal(def.clone());
                def
            }
        };
        self.dash_add_widget(def);
        self.open_tab(super::dock::Tab::Dashboard);
    }

    /// Select a message and one bit of it (from Find similar bits).
    pub(crate) fn select_message_bit(&mut self, key: MsgKey, bit: u32) {
        if self.msgs.get(key).is_none() {
            self.set_status(format!("{} is not in the Messages view", key_text(key)), true);
            return;
        }
        self.msgv.selected = Some(key);
        self.msgv.bit_sel = Some((bit, bit));
        self.open_tab(super::dock::Tab::Messages);
    }

    /// The detail pane: bit grid with toggle heat (a drag selects bits),
    /// the user signals and the mapped PDO signals with sparklines, the
    /// history. Returns true while a tint fades.
    fn message_detail(&mut self, ui: &mut egui::Ui, key: MsgKey, now: u64, fade_us: u64) -> bool {
        self.refresh_mapped(key);
        let p = theme::pal(ui);
        let Some(e) = self.msgs.get(key) else { return false };
        let mut action: Option<DetailAction> = None;
        let sel = self.msgv.selection();
        let n = if e.rtr { 0 } else { e.dlc.min(8) as usize };
        let window_s = self.settings.sparkline_s as f64;
        let now_s = now as f64 / 1e6;
        // The user signals of this COB-ID: (index, bits, colour).
        let usr: Vec<(usize, Vec<u32>, egui::Color32)> = self.an.usigs.iter().enumerate().filter(|(_, s)| s.matches_frame(key.id, key.extended, self.decode_ctx.j1939())).map(|(i, s)| (i, s.bit_list(), analysis_view::colour32(s.colour))).collect();
        ui.add_space(4.0);
        theme::toolbar(ui, |ui| {
            if ui.button(format!("{}  Close", icon::X)).on_hover_text("Close the bit view (also: click the selected row again)").clicked() {
                action = Some(DetailAction::Close);
            }
            let cur = self.settings.msg_detail_pos;
            for (pos, glyph) in [(DetailPos::Bottom, icon::ALIGN_BOTTOM), (DetailPos::Left, icon::ALIGN_LEFT), (DetailPos::Right, icon::ALIGN_RIGHT), (DetailPos::Top, icon::ALIGN_TOP)] {
                let r = ui.add(egui::SelectableLabel::new(cur == pos, egui::RichText::new(glyph).size(16.0))).on_hover_text(format!("Where the bit view is shown: {}", pos.label().to_lowercase()));
                let name = format!("Bit view {}", pos.label());
                r.widget_info(move || egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, cur == pos, &name));
                if r.clicked() && cur != pos {
                    action = Some(DetailAction::SetPos(pos));
                }
            }
            let node = e.node.map(|n| format!(" node {n}")).unwrap_or_default();
            ui.strong(format!("{} {}{node}", key_text(key), e.label));
            ui.label(egui::RichText::new(format!("{} frames, {} compared", e.count, e.compared)).color(p.text_secondary));
            if ui.button("Reset statistics").on_hover_text("Zero the count, cycle time, change and toggle counts of this row").clicked() {
                action = Some(DetailAction::ResetRow);
            }
            ui.separator();
            let txt = match sel {
                Some((s, l)) if self.msgv.sel_be => format!("Selected: bit {s} (MSB), {l} bit, big endian"),
                Some((s, l)) => format!("Selected: bit {s}, {l} bit"),
                None => "Drag over bits or click a byte or a bit (Shift: extend) to select".to_string(),
            };
            ui.label(egui::RichText::new(txt).color(p.text_secondary)).on_hover_text(SHIFT_TIP);
            ui.selectable_value(&mut self.msgv.sel_be, false, "LE").on_hover_text("Little endian: the selection runs in bit order (start = the lowest bit)");
            ui.selectable_value(&mut self.msgv.sel_be, true, "BE").on_hover_text("Big endian: the selection runs in grid reading order (start = the first cell, the MSB)");
            if ui.add_enabled(sel.is_some(), egui::Button::new("Add to Graph")).on_hover_text("Add the selected bits as a raw-frame-bits signal (unsigned)").clicked() {
                if let Some((s, l)) = sel {
                    action = Some(DetailAction::AddToGraph(s, l));
                }
            }
            if ui.add_enabled(sel.is_some(), egui::Button::new("Define signal...")).on_hover_text("Name the selected bits as a signal of this COB-ID").clicked() {
                if let Some((s, l)) = sel {
                    action = Some(DetailAction::Define(s, l));
                }
            }
            let one = sel.filter(|(_, l)| *l == 1).map(|(s, _)| s);
            if ui.add_enabled(one.is_some(), egui::Button::new("Find similar bits")).on_hover_text("Rank the bits of all messages by how well they follow the selected bit").on_disabled_hover_text("Select one bit").clicked() {
                if let Some(s) = one {
                    action = Some(DetailAction::Similar(s));
                }
            }
        });
        ui.separator();
        let mut new_sel = self.msgv.bit_sel;
        let mut drag = self.msgv.bit_drag;
        let mapped = &self.msgv.mapped;
        let sig_of = |k: u32| mapped.iter().position(|s| k >= s.offset && k < s.offset + s.bits);
        let msgv = &self.msgv;
        let shift = ui.input(|i| i.modifiers.shift);
        let (ptr, down) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down()));
        let sparks = &mut self.an.sparks;
        let usigs = &self.an.usigs;
        // The user signals with a bit in the selection (marked in the list).
        let in_sel = signals_overlapping_selection(usigs, key, self.msgv.bit_sel, self.msgv.sel_be, self.decode_ctx.j1939());
        // The grid and the value list scroll (both ways) inside the pane;
        // the history below keeps the rest of the height.
        let avail_h = ui.available_height();
        let max_h = (avail_h - 100.0).max(avail_h * 0.5).max(80.0);
        egui::ScrollArea::both().id_salt("msg_bits_scroll").max_height(max_h).auto_shrink([true, true]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                if n == 0 {
                    ui.label(egui::RichText::new("No data bytes").color(p.text_secondary));
                    return;
                }
                let cell = egui::vec2(24.0, 20.0);
                ui.horizontal(|ui| {
                    ui.add_sized(egui::vec2(56.0, 16.0), egui::Label::new(egui::RichText::new("Bit").small().color(p.text_secondary)));
                    for t in BIT_HEAD {
                        ui.add_sized(egui::vec2(cell.x, 16.0), egui::Label::new(egui::RichText::new(t).small().color(p.text_secondary)));
                    }
                    ui.add_sized(egui::vec2(40.0, 16.0), egui::Label::new(egui::RichText::new(if self.settings.msg_decimal { "Dec" } else { "Hex" }).small().color(p.text_secondary)));
                });
                for b in 0..n {
                    ui.horizontal(|ui| {
                        let r = ui.add_sized(egui::vec2(56.0, cell.y), egui::Button::new(BYTE_NAME[b]).frame(false)).on_hover_text("Select the whole byte (Shift + click: whole bytes from the first selected byte to this one)");
                        if r.clicked() {
                            let bu = b as u32;
                            new_sel = Some(match (shift, new_sel) {
                                (true, Some((a, _))) => byte_range(a / 8, bu, msgv.sel_be),
                                _ => byte_range(bu, bu, msgv.sel_be),
                            });
                        }
                        for bit in (0..8u32).rev() {
                            let k = b as u32 * 8 + bit;
                            let (r, resp) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());
                            let rate = e.toggle_rate(k as usize);
                            // Toggle heat, and over it the change flash of
                            // the last change of this bit.
                            let fk = e.bit_flash(k as usize, now);
                            let v = e.bit(k as usize);
                            let (fill, mut fg) = theme::bit_flash(p, theme::bit_heat(p, rate), fk);
                            if fk <= 0.5 && !v {
                                fg = p.text_secondary;
                            }
                            ui.painter().rect_filled(r.shrink(1.0), 2.0, fill);
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, if v { "1" } else { "0" }, egui::FontId::monospace(12.0), fg);
                            if let Some(si) = sig_of(k) {
                                ui.painter().rect_stroke(r.shrink(1.5), 2.0, egui::Stroke::new(1.5_f32, theme::series_colour(p, si)), egui::StrokeKind::Inside);
                            }
                            // User signals: an inner outline in their colour
                            // (a PDO-mapped outline stays visible outside it).
                            for (_, bits, c) in usr.iter().filter(|(_, bits, _)| bits.contains(&k)) {
                                let _ = bits;
                                ui.painter().rect_stroke(r.shrink(3.5), 1.0, egui::Stroke::new(1.5_f32, *c), egui::StrokeKind::Inside);
                            }
                            if msgv.in_selection(k) || (drag && new_sel.is_some_and(|(a, z)| sel_contains(msgv.sel_be, a, z, k))) {
                                ui.painter().rect_stroke(r.shrink(0.5), 2.0, egui::Stroke::new(2.0_f32, p.text_accent), egui::StrokeKind::Inside);
                            }
                            let resp = resp.on_hover_ui(|ui| {
                                let t = e.bit_toggles[k as usize];
                                ui.label(format!("Byte {b} bit {bit} (bit {k}): {}", u8::from(v)));
                                ui.label(egui::RichText::new("Shift + click: extend the selection to this bit").small());
                                ui.label(format!("Toggled {t} times in {} compared frames ({:.0} %)", e.compared, rate * 100.0));
                                if let Some(si) = sig_of(k) {
                                    ui.label(format!("Mapped: {}", mapped[si].text));
                                }
                                for (i, _, _) in usr.iter().filter(|(_, bits, _)| bits.contains(&k)) {
                                    ui.label(format!("Signal: {}", usigs[*i].name));
                                }
                            });
                            if resp.drag_started() {
                                new_sel = Some((k, k));
                                drag = true;
                            } else if drag && down && ptr.is_some_and(|q| r.contains(q)) {
                                if let Some((a, _)) = new_sel {
                                    new_sel = Some((a, k));
                                }
                            }
                            if resp.clicked() {
                                new_sel = match (shift, new_sel) {
                                    (true, Some((a, _))) => Some((a, k)),
                                    _ => Some((k, k)),
                                };
                            }
                        }
                        let s = &e.bytes[b];
                        let mut kt = tint_strength(s.last_change_us, now, fade_us);
                        if self.settings.msg_mute_noisy && e.is_noisy(b) {
                            kt = 0.0;
                        }
                        let (r, _) = ui.allocate_exact_size(egui::vec2(40.0, cell.y), egui::Sense::hover());
                        // The change flash of the byte (its last changed
                        // bit) first; after it the up / down tint.
                        let fb = e.byte_flash(b, now);
                        let mut fg = p.text_primary;
                        if fb > 0.0 {
                            let (fill, t) = theme::bit_flash(p, p.bg_panel, fb);
                            fg = t;
                            ui.painter().rect_filled(r.shrink(1.0), 2.0, fill);
                        } else if kt > 0.0 {
                            ui.painter().rect_filled(r.shrink(1.0), 2.0, theme::mix(p.bg_panel, theme::change_tint(p, s.dir == Some(Change::Up)), kt));
                        }
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, byte_text(e.data[b], self.settings.msg_decimal), egui::FontId::monospace(12.0), fg);
                        // The names of the user signals in this byte.
                        for (i, _, c) in usr.iter().filter(|(_, bits, _)| bits.iter().any(|k| *k / 8 == b as u32)) {
                            ui.label(egui::RichText::new(&usigs[*i].name).small().color(*c));
                        }
                    });
                }
            });
            // A space, not a separator: a vertical separator takes the full
            // pane height, and the history below would grow the pane.
            ui.add_space(16.0);
            ui.vertical(|ui| {
                let data = &e.data[..n];
                if !usr.is_empty() {
                    ui.label(egui::RichText::new("User signals").small().color(p.text_secondary));
                    for (i, _, c) in &usr {
                        let s = &usigs[*i];
                        ui.horizontal(|ui| {
                            // An accent bar when the signal has a bit in the
                            // selection.
                            let hit = in_sel.contains(i);
                            let (bar, br) = ui.allocate_exact_size(egui::vec2(3.0, 14.0), egui::Sense::hover());
                            if hit {
                                ui.painter().rect_filled(bar, 1.0, p.text_accent);
                                br.on_hover_text("This signal has bits in the selection").widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "In the selection"));
                            }
                            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 2.0, *c);
                            let vt = s.value(data).map(fmt_value).unwrap_or_else(|| "-".to_string());
                            let unit = if s.unit.is_empty() { String::new() } else { format!(" {}", s.unit) };
                            let kind = match s.kind {
                                RawKind::Unsigned => "unsigned",
                                RawKind::Signed => "signed",
                                RawKind::Float => "float32",
                            };
                            ui.label(format!("{} = {vt}{unit}", s.name)).on_hover_text(format!("bit {}, {} bit, {}, {kind}, factor {}, offset {}", s.start_bit, s.bits, if s.big_endian { "big endian" } else { "little endian" }, fmt_value(s.factor), fmt_value(s.offset)));
                            analysis_view::sparkline(ui, sparks.ensure(key, s, window_s), now_s, *c);
                            if ui.small_button("Edit").clicked() {
                                action = Some(DetailAction::EditUser(*i));
                            }
                            if ui.small_button("Delete").clicked() {
                                action = Some(DetailAction::DeleteUser(*i));
                            }
                            if s.bits == 1 && ui.small_button("Similar bits").on_hover_text("Find the bits that follow this 1-bit signal").clicked() {
                                action = Some(DetailAction::Similar(s.start_bit));
                            }
                            signal_action(ui, &s.to_def(), &mut action);
                        });
                    }
                    ui.add_space(4.0);
                }
                if mapped.is_empty() {
                    ui.label(egui::RichText::new("No known PDO mapping for this COB-ID.").color(p.text_secondary));
                } else {
                    ui.label(egui::RichText::new(format!("{} mapping", e.label)).small().color(p.text_secondary));
                    for (si, s) in mapped.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let c = theme::series_colour(p, si);
                            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(2.0_f32, c), egui::StrokeKind::Inside);
                            let v = crate::canopen::pdo::extract_bits(data, s.offset, s.bits);
                            let vt = v.map_or("-".to_string(), |v| format!("{v} (0x{v:X})"));
                            ui.label(format!("{} = {vt}", s.text)).on_hover_text(format!("bit {}, {} bit", s.offset, s.bits));
                            if s.bits <= 64 {
                                let us = UserSignal::new(s.text.clone(), key.id, key.extended, s.offset, s.bits);
                                analysis_view::sparkline(ui, sparks.ensure(key, &us, window_s), now_s, c);
                            }
                            signal_action(ui, &s.def, &mut action);
                        });
                    }
                }
            });
        });
        });
        if !down {
            drag = false;
        }
        // Repaints while a tint or a change flash fades (none when all are out).
        let fading = (0..n).any(|b| tint_strength(e.bytes[b].last_change_us, now, fade_us) > 0.0 || e.byte_flash(b, now) > 0.0);
        // The history columns: the mapped PDO signals, then the user signals.
        let mut cols: Vec<UserSignal> = self.msgv.mapped.iter().filter(|s| s.bits <= 64).map(|s| UserSignal::new(s.text.clone(), key.id, key.extended, s.offset, s.bits)).collect();
        cols.extend(usr.iter().map(|(i, _, _)| self.an.usigs[*i].clone()));
        self.msgv.bit_sel = new_sel;
        self.msgv.bit_drag = drag;
        if drag {
            ui.ctx().request_repaint();
        }
        ui.separator();
        self.history_section(ui, key, cols);
        let be = self.msgv.sel_be;
        match action {
            Some(DetailAction::Close) => {
                self.msgv.selected = None;
                self.msgv.bit_sel = None;
            }
            Some(DetailAction::ResetRow) => self.msgs.reset_id(key),
            Some(DetailAction::AddToGraph(start_bit, bits)) => {
                let source = SignalSource::Raw { cob_id: key.id, extended: key.extended, start_bit, bits, raw_kind: RawKind::Unsigned, big_endian: be };
                let name = graph::default_name(&source);
                if self.add_signal(SignalDef::new(name.clone(), source)).is_some() {
                    self.set_status(format!("Signal {name} added to the Graph"), false);
                }
            }
            Some(DetailAction::Define(start_bit, bits)) => self.define_signal(key, start_bit, bits, be, false),
            Some(DetailAction::Similar(bit)) => self.start_similar(BitRef { key, bit }),
            Some(DetailAction::EditUser(i)) => self.edit_user_signal(i),
            Some(DetailAction::DeleteUser(i)) => self.delete_user_signal(i),
            Some(DetailAction::SetPos(pos)) => {
                self.settings.msg_detail_pos = pos;
                self.mark_settings();
            }
            Some(DetailAction::SigToGraph(def)) => self.signal_to_graph(def),
            Some(DetailAction::SigToDash(def)) => self.signal_to_dashboard(def),
            None => {}
        }
        fading
    }
}

/// Bit k lies in the selection (a, z) (`be`: in grid reading order).
fn sel_contains(be: bool, a: u32, z: u32, k: u32) -> bool {
    if be {
        let (ga, gz, g) = (grid_pos(a), grid_pos(z), grid_pos(k));
        g >= ga.min(gz) && g <= ga.max(gz)
    } else {
        k >= a.min(z) && k <= a.max(z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The (start bit, length) that the Messages view shows for (a, z).
    fn start_len(a: u32, z: u32, be: bool) -> (u32, u32) {
        let v = MessagesView { bit_sel: Some((a, z)), sel_be: be, ..Default::default() };
        v.selection().unwrap()
    }

    #[test]
    fn whole_bytes_both_directions() {
        // LE: bit order, the start is the lowest bit.
        assert_eq!(byte_range(1, 2, false), (8, 23));
        assert_eq!(byte_range(2, 1, false), (23, 8));
        assert_eq!(start_len(8, 23, false), (8, 16));
        assert_eq!(start_len(23, 8, false), (8, 16));
        // BE: grid reading order, the start is the MSB of the first byte.
        assert_eq!(byte_range(1, 2, true), (15, 16));
        assert_eq!(byte_range(2, 1, true), (16, 15));
        assert_eq!(start_len(15, 16, true), (15, 16));
        assert_eq!(start_len(16, 15, true), (15, 16));
        // One byte.
        assert_eq!(start_len(byte_range(3, 3, false).0, byte_range(3, 3, false).1, false), (24, 8));
        assert_eq!(start_len(byte_range(3, 3, true).0, byte_range(3, 3, true).1, true), (31, 8));
    }

    #[test]
    fn overlapping_signals() {
        let key = MsgKey { id: 0x181, extended: false };
        let a = UserSignal::new("Battery voltage", 0x181, false, 0, 16);
        let b = UserSignal::new("Motor speed", 0x181, false, 16, 16);
        let other = UserSignal::new("Other", 0x281, false, 0, 16);
        let mut be = UserSignal::new("Pressure", 0x181, false, 39, 16);
        be.big_endian = true;
        let sigs = vec![a, b, other, be];
        assert!(signals_overlapping_selection(&sigs, key, None, false, true).is_empty());
        // Byte 1 (LE): only the first signal.
        assert_eq!(signals_overlapping_selection(&sigs, key, Some((8, 15)), false, true), vec![0]);
        // Bits 15..16 (LE, either direction): the first two.
        assert_eq!(signals_overlapping_selection(&sigs, key, Some((16, 15)), false, true), vec![0, 1]);
        // Byte 5 (BE): the big-endian signal (bytes 4 and 5).
        let (s, z) = byte_range(5, 5, true);
        assert_eq!(signals_overlapping_selection(&sigs, key, Some((s, z)), true, true), vec![3]);
        // Another COB-ID: nothing.
        assert!(signals_overlapping_selection(&sigs, MsgKey { id: 0x182, extended: false }, Some((0, 63)), false, true).is_empty());
    }
}
