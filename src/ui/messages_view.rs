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

fn hex2(b: u8) -> &'static str {
    let i = b as usize * 2;
    std::str::from_utf8(&HEX[i..i + 2]).unwrap_or("??")
}

const BYTE_HEAD: [&str; 8] = ["B0", "B1", "B2", "B3", "B4", "B5", "B6", "B7"];
const BYTE_NAME: [&str; 8] = ["Byte 0", "Byte 1", "Byte 2", "Byte 3", "Byte 4", "Byte 5", "Byte 6", "Byte 7"];
const BIT_HEAD: [&str; 8] = ["7", "6", "5", "4", "3", "2", "1", "0"];

fn key_text(k: MsgKey) -> String {
    if k.extended {
        format!("{:08X}", k.id)
    } else {
        format!("{:03X}", k.id)
    }
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
    ResetRow,
    AddToGraph(u32, u32),
    Define(u32, u32),
    Similar(u32),
    EditUser(usize),
    DeleteUser(usize),
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
        ui.horizontal_wrapped(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.msgv.filter).hint_text("Filter: COB-ID, type, node").desired_width(200.0));
            if r.changed() {
                self.msgv.filter_lower = self.msgv.filter.trim().to_lowercase();
                self.msgv.dirty = true;
            }
            let before = self.msgv.sort;
            egui::ComboBox::from_label("Sort by").selected_text(self.msgv.sort.label()).show_ui(ui, |ui| {
                for s in MsgSort::ALL {
                    ui.selectable_value(&mut self.msgv.sort, s, s.label());
                }
            });
            if self.msgv.sort != before {
                self.msgv.dirty = true;
            }
            ui.separator();
            let mut mute = self.settings.msg_mute_noisy;
            if ui.checkbox(&mut mute, "Mute noisy bytes").on_hover_text("No tint on a byte that changes in nearly every frame (for example a counter)").changed() {
                self.settings.msg_mute_noisy = mute;
                self.mark_settings();
            }
            if ui.button("Reset all statistics").on_hover_text("Forget every row; rows come back with the next frames").clicked() {
                self.msgs.reset();
                self.msgv.on_reset();
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
        if let Some(key) = self.msgv.selected {
            egui::TopBottomPanel::bottom("msg_detail").resizable(true).default_height(460.0).min_height(120.0).show_inside(ui, |ui| {
                fading |= self.message_detail(ui, key, now, fade_us);
            });
        }
        let row_h = self.settings.row_density.row_height();
        let mute = self.settings.msg_mute_noisy;
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
                            ui.label(egui::RichText::new(hex2(e.data[b])).monospace().color(p.text_primary));
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
                        ui.add(egui::Label::new(crate::canopen::decode::decode(&f, dctx).text).truncate());
                    });
                    if row.response().clicked() {
                        clicked = Some(e.key);
                    }
                });
            });
        if let Some(k) = clicked {
            if self.msgv.selected != Some(k) {
                self.msgv.bit_sel = None;
            }
            self.msgv.selected = Some(k);
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
        self.msgv.mapped.clear();
        if let Some((node, map)) = &owned {
            let od = self.od_of_node(*node);
            let mut off = 0u32;
            for m in map {
                let bits = m.bits as u32;
                if !m.is_dummy() && bits > 0 {
                    let name = od.as_ref().and_then(|o| o.name_of(m.index, m.sub)).unwrap_or_default();
                    let text = format!("{:04X}:{:02X} {name}", m.index, m.sub).trim_end().to_string();
                    self.msgv.mapped.push(MappedSig { offset: off, bits, text });
                }
                off += bits;
            }
        }
        self.msgv.pdo_cache = Some((key, owned));
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
        let usr: Vec<(usize, Vec<u32>, egui::Color32)> = self.an.usigs.iter().enumerate().filter(|(_, s)| s.matches(key.id, key.extended)).map(|(i, s)| (i, s.bit_list(), analysis_view::colour32(s.colour))).collect();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
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
            ui.label(egui::RichText::new(txt).color(p.text_secondary));
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
                    ui.add_sized(egui::vec2(40.0, 16.0), egui::Label::new(egui::RichText::new("Hex").small().color(p.text_secondary)));
                });
                for b in 0..n {
                    ui.horizontal(|ui| {
                        let r = ui.add_sized(egui::vec2(56.0, cell.y), egui::Button::new(BYTE_NAME[b]).frame(false)).on_hover_text("Select the whole byte");
                        if r.clicked() {
                            new_sel = Some(if msgv.sel_be { (b as u32 * 8 + 7, b as u32 * 8) } else { (b as u32 * 8, b as u32 * 8 + 7) });
                        }
                        for bit in (0..8u32).rev() {
                            let k = b as u32 * 8 + bit;
                            let (r, resp) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());
                            let rate = e.toggle_rate(k as usize);
                            ui.painter().rect_filled(r.shrink(1.0), 2.0, theme::bit_heat(p, rate));
                            let v = e.bit(k as usize);
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, if v { "1" } else { "0" }, egui::FontId::monospace(12.0), if v { p.text_primary } else { p.text_secondary });
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
                        if kt > 0.0 {
                            ui.painter().rect_filled(r.shrink(1.0), 2.0, theme::mix(p.bg_panel, theme::change_tint(p, s.dir == Some(Change::Up)), kt));
                        }
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, hex2(e.data[b]), egui::FontId::monospace(12.0), p.text_primary);
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
                        });
                    }
                }
            });
        });
        if !down {
            drag = false;
        }
        let fading = (0..n).any(|b| tint_strength(e.bytes[b].last_change_us, now, fade_us) > 0.0);
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
