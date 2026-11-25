//! Transmit list view: a frame editor (send once, add to the list) and the
//! list of named frames with a trigger (manual, periodic, on reception).
//! The bus worker runs the list; an entry sends only while it is started
//! or when Send is pressed. The list is saved in `txlist.json`.

use super::{theme, App};
use crate::bus::{BusCommand, BusHandle};
use crate::can::CanFrame;
use crate::canopen::types::{parse_text, DataType};
use crate::library::Library;
use crate::settings::SavedFrame;
use crate::txlist::{Trigger, TxEntry, TxList};
use eframe::egui;
use egui_extras::{Column, TableBuilder};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrigKind {
    Manual,
    Periodic,
    OnRx,
}

pub(crate) struct TxView {
    entries: Vec<TxEntry>,
    edit: SavedFrame,
    id_text: String,
    trig: TrigKind,
    rx_text: String,
    /// Key of the entry in the editor (Update instead of Add).
    editing: Option<u64>,
    error: Option<String>,
}

impl Default for TxView {
    fn default() -> Self {
        TxView {
            entries: Vec::new(),
            edit: SavedFrame { name: "frame".to_string(), id: 0x123, extended: false, rtr: false, dlc: 2, data: "01 02".to_string(), period_ms: 100 },
            id_text: "123".to_string(),
            trig: TrigKind::Periodic,
            rx_text: "181".to_string(),
            editing: None,
            error: None,
        }
    }
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

/// Build the CAN frame of a frame description.
pub(crate) fn build_frame(s: &SavedFrame) -> Result<CanFrame, String> {
    let max = if s.extended { 0x1FFF_FFFF } else { 0x7FF };
    if s.id > max {
        return Err(format!("ID 0x{:X} is above 0x{max:X}", s.id));
    }
    if s.dlc > 8 {
        return Err("DLC must be 0..8".to_string());
    }
    let data = if s.data.trim().is_empty() || s.rtr { Vec::new() } else { parse_text(DataType::OctetString, &s.data, false).map_err(|e| format!("data: {e}"))? };
    if data.len() > 8 {
        return Err("at most 8 data bytes".to_string());
    }
    if !s.rtr && data.len() > s.dlc as usize {
        return Err(format!("{} data bytes but DLC {}", data.len(), s.dlc));
    }
    let mut f = CanFrame::new(s.id, &data);
    f.extended = s.extended;
    f.rtr = s.rtr;
    f.dlc = s.dlc;
    f.validate().map_err(|e| e.to_string())?;
    Ok(f)
}

fn trigger_text(t: &Trigger) -> String {
    match t {
        Trigger::Manual => "manual".to_string(),
        Trigger::Periodic { period_ms } => format!("every {period_ms} ms"),
        Trigger::OnRx { cob_id, extended } => format!("on RX {}{cob_id:X}", if *extended { "x" } else { "" }),
    }
}

impl TxView {
    /// Load `txlist.json` (or the v0.1 saved frames once) and hand the list
    /// to the worker. Nothing is sent: every entry is stopped.
    pub(crate) fn load(&mut self, lib: Option<&Library>, saved: &[SavedFrame], bus: &BusHandle) -> Option<String> {
        let lib = lib?;
        let mut note = None;
        match TxList::load(&lib.txlist_path()) {
            Ok(l) => self.entries = l.entries,
            Err(e) => note = Some(format!("Transmit list not read: {e}")),
        }
        if self.entries.is_empty() && !lib.txlist_path().exists() {
            for (i, s) in saved.iter().enumerate() {
                if let Ok(f) = build_frame(s) {
                    self.entries.push(TxEntry::from_frame(i as u64 + 1, s.name.clone(), &f, Trigger::Periodic { period_ms: s.period_ms as u64 }));
                }
            }
        }
        bus.send(BusCommand::TxLoad(self.entries.clone()));
        note
    }

    fn free_key(&self) -> u64 {
        self.entries.iter().map(|e| e.key).max().map(|k| k + 1).unwrap_or(1)
    }
}

impl App {
    fn save_txlist(&mut self) {
        if self.data_error.is_some() {
            return;
        }
        let Some(lib) = &self.lib else { return };
        if let Err(e) = TxList::new(self.txv.entries.clone()).save(&lib.txlist_path()) {
            self.set_status(format!("Transmit list not saved: {e}"), true);
        }
    }

    pub(crate) fn txlist_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let can_send = self.can_send();
        if !can_send {
            ui.colored_label(p.warning, if self.listen_only_active { "Listen only: every send control is disabled." } else { "Not connected: send controls are disabled." });
        }
        // Editor.
        theme::toolbar(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut self.txv.edit.name).desired_width(100.0));
            ui.label("ID 0x");
            if ui.add(egui::TextEdit::singleline(&mut self.txv.id_text).desired_width(80.0)).changed() {
                if let Some(v) = parse_hex_u32(&self.txv.id_text) {
                    self.txv.edit.id = v;
                }
            }
            ui.checkbox(&mut self.txv.edit.extended, "Ext");
            ui.checkbox(&mut self.txv.edit.rtr, "RTR");
            ui.label("DLC");
            ui.add(egui::DragValue::new(&mut self.txv.edit.dlc).range(0..=8));
            ui.label("Data");
            ui.add(egui::TextEdit::singleline(&mut self.txv.edit.data).desired_width(170.0).hint_text("01 02 03"));
        });
        theme::toolbar(ui, |ui| {
            ui.label("Trigger");
            ui.selectable_value(&mut self.txv.trig, TrigKind::Manual, "Manual");
            ui.selectable_value(&mut self.txv.trig, TrigKind::Periodic, "Periodic");
            ui.selectable_value(&mut self.txv.trig, TrigKind::OnRx, "On receive");
            match self.txv.trig {
                TrigKind::Periodic => {
                    ui.add(egui::DragValue::new(&mut self.txv.edit.period_ms).range(1..=60000).suffix(" ms"));
                }
                TrigKind::OnRx => {
                    ui.label("of COB-ID 0x");
                    ui.add(egui::TextEdit::singleline(&mut self.txv.rx_text).desired_width(70.0));
                }
                TrigKind::Manual => {}
            }
        });
        let id_ok = parse_hex_u32(&self.txv.id_text).is_some();
        let built = if id_ok { build_frame(&self.txv.edit) } else { Err("ID: hex value".to_string()) };
        let trigger = match self.txv.trig {
            TrigKind::Manual => Ok(Trigger::Manual),
            TrigKind::Periodic => Ok(Trigger::Periodic { period_ms: self.txv.edit.period_ms.max(1) as u64 }),
            TrigKind::OnRx => parse_hex_u32(&self.txv.rx_text).map(|c| Trigger::OnRx { cob_id: c, extended: c > 0x7FF }).ok_or_else(|| "on receive: COB-ID in hex".to_string()),
        };
        theme::toolbar(ui, |ui| {
            if self.send_button(ui, built.is_ok(), "Send once").clicked() {
                if let Ok(f) = &built {
                    self.bus.send(BusCommand::Send(*f));
                }
            }
            let name_ok = !self.txv.edit.name.trim().is_empty();
            let label = if self.txv.editing.is_some() { "Update entry" } else { "Add to list" };
            if ui.add_enabled(built.is_ok() && trigger.is_ok() && name_ok, egui::Button::new(label)).on_hover_text("Adds the frame; it sends only when you press Send or Start").clicked() {
                if let (Ok(f), Ok(t)) = (&built, &trigger) {
                    let key = self.txv.editing.unwrap_or_else(|| self.txv.free_key());
                    let e = TxEntry::from_frame(key, self.txv.edit.name.trim(), f, t.clone());
                    match self.txv.entries.iter_mut().find(|x| x.key == key) {
                        Some(x) => *x = e.clone(),
                        None => self.txv.entries.push(e.clone()),
                    }
                    self.bus.send(BusCommand::TxUpsert(e));
                    self.txv.editing = None;
                    self.save_txlist();
                }
            }
            if self.txv.editing.is_some() && ui.button("Cancel edit").clicked() {
                self.txv.editing = None;
            }
            if let Err(e) = built.as_ref().map(|_| ()).and(trigger.as_ref().map(|_| ())) {
                ui.colored_label(p.error, e);
            }
        });
        ui.separator();

        // The list.
        let running: Vec<u64> = self.bus.shared.lock().map(|s| s.txlist.iter().filter(|e| e.enabled).map(|e| e.key).collect()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} entr{}, {} running", self.txv.entries.len(), if self.txv.entries.len() == 1 { "y" } else { "ies" }, running.len())).color(p.text_secondary));
            if ui.add_enabled(!running.is_empty(), egui::Button::new("Stop all")).clicked() {
                self.bus.send(BusCommand::TxStopAll);
            }
        });
        if self.txv.entries.is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::PAPER_PLANE_TILT).size(28.0).color(p.text_secondary));
                ui.label("No frames in the transmit list. Build one above and press Add to list.");
            });
            return;
        }
        let row_h = self.settings.row_density.row_height().max(24.0);
        let mut fire = None;
        let mut start = None;
        let mut stop = None;
        let mut edit = None;
        let mut remove = None;
        let entries = self.txv.entries.clone();
        let hint = self.send_hint();
        TableBuilder::new(ui)
            .id_salt("txlist")
            .striped(self.settings.zebra)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(120.0).clip(true))
            .column(Column::initial(80.0))
            .column(Column::initial(34.0))
            .column(Column::initial(170.0).clip(true))
            .column(Column::initial(110.0).clip(true))
            .column(Column::initial(80.0))
            .column(Column::remainder().at_least(200.0))
            .min_scrolled_height(0.0)
            .header(row_h, |mut h| {
                for t in ["Name", "ID", "DLC", "Data", "Trigger", "State", ""] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for e in &entries {
                    let on = running.contains(&e.key);
                    body.row(row_h, |mut row| {
                        row.col(|ui| {
                            ui.add(egui::Label::new(&e.name).truncate());
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{}{:X}", if e.extended { "x" } else { "" }, e.id));
                        });
                        row.col(|ui| {
                            ui.monospace(e.dlc.to_string());
                        });
                        row.col(|ui| {
                            let d = if e.rtr { "RTR".to_string() } else { e.data[..e.dlc.min(8) as usize].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ") };
                            ui.monospace(d);
                        });
                        row.col(|ui| {
                            ui.label(trigger_text(&e.trigger));
                        });
                        row.col(|ui| {
                            if on {
                                theme::pill(ui, "running", p.success, p.success_tint, self.settings.reduce_colour);
                            } else {
                                ui.label(egui::RichText::new("stopped").color(p.text_secondary));
                            }
                        });
                        row.col(|ui| {
                            let r = ui.add_enabled(can_send, egui::Button::new("Send"));
                            if r.clicked() {
                                fire = Some(e.key);
                            }
                            if !can_send {
                                r.on_disabled_hover_text(hint);
                            }
                            if e.trigger != Trigger::Manual {
                                if on {
                                    if ui.button("Stop").clicked() {
                                        stop = Some(e.key);
                                    }
                                } else if ui.add_enabled(can_send, egui::Button::new("Start")).clicked() {
                                    start = Some(e.key);
                                }
                            }
                            if ui.button("Edit").clicked() {
                                edit = Some(e.clone());
                            }
                            if ui.button("Remove").clicked() {
                                remove = Some(e.key);
                            }
                        });
                    });
                }
            });
        if let Some(k) = fire {
            self.bus.send(BusCommand::TxFire(k));
        }
        if let Some(k) = start {
            self.bus.send(BusCommand::TxStart(k));
        }
        if let Some(k) = stop {
            self.bus.send(BusCommand::TxStop(k));
        }
        if let Some(e) = edit {
            self.txv.editing = Some(e.key);
            self.txv.edit = SavedFrame {
                name: e.name.clone(),
                id: e.id,
                extended: e.extended,
                rtr: e.rtr,
                dlc: e.dlc,
                data: e.data[..e.dlc.min(8) as usize].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
                period_ms: match e.trigger {
                    Trigger::Periodic { period_ms } => period_ms as u32,
                    _ => self.txv.edit.period_ms,
                },
            };
            self.txv.id_text = format!("{:X}", e.id);
            match e.trigger {
                Trigger::Manual => self.txv.trig = TrigKind::Manual,
                Trigger::Periodic { .. } => self.txv.trig = TrigKind::Periodic,
                Trigger::OnRx { cob_id, .. } => {
                    self.txv.trig = TrigKind::OnRx;
                    self.txv.rx_text = format!("{cob_id:X}");
                }
            }
        }
        if let Some(k) = remove {
            self.bus.send(BusCommand::TxRemove(k));
            self.txv.entries.retain(|e| e.key != k);
            if self.txv.editing == Some(k) {
                self.txv.editing = None;
            }
            self.save_txlist();
        }
        if !running.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        let _ = &self.txv.error;
    }
}
