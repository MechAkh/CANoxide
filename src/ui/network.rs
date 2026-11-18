//! Network tab: NMT, node list, node scan, SYNC and the raw frame sender.

use super::App;
use crate::bus::BusCommand;
use crate::can::CanFrame;
use crate::canopen::nmt::NmtCommand;
use crate::canopen::types::{parse_text, DataType};
use crate::settings::{AdapterKind, SavedFrame};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;

/// Cyclic key of the frame editor; saved frames use a hash of their name.
const EDITOR_KEY: u64 = 0;

pub(crate) struct NetView {
    pub(crate) scanning: bool,
    pub(crate) scan_results: Vec<(u8, Result<u32, String>)>,
    pub(crate) cyclic_active: HashSet<u64>,
    edit: SavedFrame,
    id_text: String,
    emcy_code: String,
    emcy_reg: String,
}

impl Default for NetView {
    fn default() -> Self {
        NetView {
            scanning: false,
            scan_results: Vec::new(),
            cyclic_active: HashSet::new(),
            edit: SavedFrame { name: "frame".to_string(), id: 0x123, extended: false, rtr: false, dlc: 2, data: "01 02".to_string(), period_ms: 100 },
            id_text: "123".to_string(),
            emcy_code: "8130".to_string(),
            emcy_reg: "11".to_string(),
        }
    }
}

fn name_key(name: &str) -> u64 {
    // FNV-1a; never 0 (the editor key).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h | 1
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

/// Build the CAN frame of a saved frame description.
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

impl App {
    pub(crate) fn network_tab(&mut self, ui: &mut egui::Ui) {
        let can_send = self.can_send();
        let node = self.active_node();
        egui::ScrollArea::vertical().id_salt("net_scroll").show(ui, |ui| {
            if !can_send {
                ui.colored_label(ui.visuals().warn_fg_color, if self.listen_only_active { "Listen only: every send control is disabled." } else { "Not connected: send controls are disabled." });
            }
            ui.heading("NMT");
            egui::Grid::new("nmt").num_columns(6).show(ui, |ui| {
                ui.label(format!("Node {node}"));
                for c in NmtCommand::ALL {
                    if ui.add_enabled(can_send, egui::Button::new(c.label())).clicked() {
                        self.bus.send(BusCommand::Nmt { cmd: c, node });
                        self.set_status(format!("NMT {} node {node} sent", c.label()), false);
                    }
                }
                ui.end_row();
                ui.label("All nodes");
                for c in NmtCommand::ALL {
                    if ui.add_enabled(can_send, egui::Button::new(c.label())).clicked() {
                        self.bus.send(BusCommand::Nmt { cmd: c, node: 0 });
                        self.set_status(format!("NMT {} all nodes sent", c.label()), false);
                    }
                }
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(can_send, egui::Button::new("Send SYNC")).clicked() {
                    self.bus.send(BusCommand::Sync);
                    self.set_status("SYNC sent", false);
                }
                if self.settings.adapter == AdapterKind::Simulator {
                    ui.separator();
                    ui.label("Simulator EMCY code 0x");
                    ui.add(egui::TextEdit::singleline(&mut self.net.emcy_code).desired_width(48.0));
                    ui.label("reg 0x");
                    ui.add(egui::TextEdit::singleline(&mut self.net.emcy_reg).desired_width(28.0));
                    let code = parse_hex_u32(&self.net.emcy_code).and_then(|v| u16::try_from(v).ok());
                    let reg = parse_hex_u32(&self.net.emcy_reg).and_then(|v| u8::try_from(v).ok());
                    if ui.add_enabled(self.connected && code.is_some() && reg.is_some(), egui::Button::new("Trigger EMCY")).on_hover_text("The simulated node sends an EMCY").clicked() {
                        if let (Some(code), Some(reg)) = (code, reg) {
                            self.bus.send(BusCommand::SimEmcy { code, reg });
                        }
                    }
                }
            });

            ui.add_space(6.0);
            ui.heading("Nodes seen");
            self.node_table(ui);

            ui.add_space(6.0);
            ui.heading("Scan nodes");
            self.scan_section(ui, can_send);

            ui.add_space(6.0);
            ui.heading("Raw frame");
            self.frame_sender(ui, can_send);
        });
    }

    fn node_table(&mut self, ui: &mut egui::Ui) {
        let (nodes, now) = {
            let s = self.bus.shared.lock().unwrap_or_else(|p| p.into_inner());
            (s.nodes.nodes.clone(), s.now_us)
        };
        if nodes.is_empty() {
            ui.label("No heartbeat or boot-up seen yet.");
            return;
        }
        let devs: Vec<(u8, String, String)> = match &self.lib {
            Some(l) => l.devices.iter().map(|d| (d.node_id, d.name.clone(), d.eds_file.clone().unwrap_or_default())).collect(),
            None => Vec::new(),
        };
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        TableBuilder::new(ui)
            .id_salt("nodes")
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(50.0))
            .column(Column::initial(120.0))
            .column(Column::initial(90.0))
            .column(Column::initial(70.0))
            .column(Column::initial(140.0).clip(true))
            .column(Column::remainder().clip(true))
            .vscroll(false)
            .header(row_h, |mut h| {
                for t in ["Node", "State", "Last seen", "Count", "Device", "EDS"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for (id, e) in &nodes {
                    body.row(row_h, |mut row| {
                        row.col(|ui| {
                            ui.label(id.to_string());
                        });
                        row.col(|ui| {
                            ui.label(e.state.label());
                        });
                        row.col(|ui| {
                            ui.label(format!("{:.1} s ago", now.saturating_sub(e.last_seen_us) as f64 / 1e6));
                        });
                        row.col(|ui| {
                            ui.label(e.count.to_string());
                        });
                        let d = devs.iter().find(|d| d.0 == *id);
                        row.col(|ui| {
                            ui.label(d.map(|d| d.1.as_str()).unwrap_or(""));
                        });
                        row.col(|ui| {
                            ui.label(d.map(|d| d.2.as_str()).unwrap_or(""));
                        });
                    });
                }
            });
    }

    fn scan_section(&mut self, ui: &mut egui::Ui, can_send: bool) {
        let progress = self.bus.shared.lock().map(|s| s.scan).unwrap_or(None);
        ui.horizontal(|ui| {
            ui.label("SDO read of 0x1000 on nodes 1..127, one at a time, timeout");
            if ui.add(egui::DragValue::new(&mut self.settings.scan_timeout_ms).range(20..=2000).suffix(" ms")).changed() {
                self.mark_settings();
            }
            if self.net.scanning || progress.is_some() {
                if ui.button("Cancel").clicked() {
                    self.bus.send(BusCommand::CancelScan);
                }
            } else if ui.add_enabled(can_send, egui::Button::new("Scan nodes")).clicked() {
                self.net.scan_results.clear();
                self.net.scanning = true;
                self.bus.send(BusCommand::Scan { from: 1, to: 127, timeout_ms: self.settings.scan_timeout_ms });
                self.set_status("Scanning nodes 1..127", false);
            }
        });
        if let Some((done, total)) = progress {
            let frac = if total == 0 { 0.0 } else { done as f32 / total as f32 };
            ui.add(egui::ProgressBar::new(frac).text(format!("{done} / {total}")));
        }
        if !self.net.scan_results.is_empty() {
            let mut res = self.net.scan_results.clone();
            res.sort_by_key(|r| r.0);
            egui::Grid::new("scan_res").striped(true).num_columns(2).show(ui, |ui| {
                for (n, r) in &res {
                    ui.label(format!("Node {n}"));
                    match r {
                        Ok(v) => ui.label(format!("device type 0x{v:08X} (profile {})", v & 0xFFFF)),
                        Err(e) => ui.label(format!("answered: {e}")),
                    };
                    ui.end_row();
                }
            });
        }
    }

    fn frame_sender(&mut self, ui: &mut egui::Ui, can_send: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.label("ID 0x");
            if ui.add(egui::TextEdit::singleline(&mut self.net.id_text).desired_width(80.0)).changed() {
                if let Some(v) = parse_hex_u32(&self.net.id_text) {
                    self.net.edit.id = v;
                }
            }
            ui.checkbox(&mut self.net.edit.extended, "Ext");
            ui.checkbox(&mut self.net.edit.rtr, "RTR");
            ui.label("DLC");
            ui.add(egui::DragValue::new(&mut self.net.edit.dlc).range(0..=8));
            ui.label("Data");
            ui.add(egui::TextEdit::singleline(&mut self.net.edit.data).desired_width(170.0).hint_text("01 02 03"));
            ui.label("Period");
            ui.add(egui::DragValue::new(&mut self.net.edit.period_ms).range(1..=60000).suffix(" ms"));
        });
        let id_ok = parse_hex_u32(&self.net.id_text).is_some();
        let built = if id_ok { build_frame(&self.net.edit) } else { Err("ID: hex value".to_string()) };
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(can_send && built.is_ok(), egui::Button::new("Send once")).clicked() {
                if let Ok(f) = &built {
                    self.bus.send(BusCommand::Send(*f));
                }
            }
            let running = self.net.cyclic_active.contains(&EDITOR_KEY);
            if running {
                if ui.button("Stop cyclic").clicked() {
                    self.bus.send(BusCommand::StopCyclic(EDITOR_KEY));
                    self.net.cyclic_active.remove(&EDITOR_KEY);
                }
            } else if ui.add_enabled(can_send && built.is_ok(), egui::Button::new("Start cyclic")).clicked() {
                if let Ok(f) = &built {
                    self.bus.send(BusCommand::StartCyclic { key: EDITOR_KEY, frame: *f, period_ms: self.net.edit.period_ms as u64 });
                    self.net.cyclic_active.insert(EDITOR_KEY);
                }
            }
            ui.separator();
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut self.net.edit.name).desired_width(110.0));
            if ui.add_enabled(built.is_ok() && !self.net.edit.name.trim().is_empty(), egui::Button::new("Save frame")).clicked() {
                let mut s = self.net.edit.clone();
                s.name = s.name.trim().to_string();
                self.settings.saved_frames.retain(|x| x.name != s.name);
                self.settings.saved_frames.push(s);
                self.mark_settings();
            }
            if let Err(e) = &built {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
        });

        if self.settings.saved_frames.is_empty() {
            return;
        }
        ui.label("Saved frames");
        let mut load: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut send: Option<CanFrame> = None;
        let mut start: Option<(u64, CanFrame, u64)> = None;
        let mut stop: Option<u64> = None;
        egui::Grid::new("saved_frames").striped(true).num_columns(5).show(ui, |ui| {
            for (i, s) in self.settings.saved_frames.iter().enumerate() {
                let key = name_key(&s.name);
                ui.label(&s.name);
                ui.monospace(format!("{}{:X} [{}] {}{}", if s.extended { "x" } else { "" }, s.id, s.dlc, s.data, if s.rtr { " RTR" } else { "" }));
                ui.label(format!("{} ms", s.period_ms));
                let f = build_frame(s);
                ui.horizontal(|ui| {
                    if ui.small_button("Load").clicked() {
                        load = Some(i);
                    }
                    if ui.add_enabled(can_send && f.is_ok(), egui::Button::new("Send").small()).clicked() {
                        send = f.clone().ok();
                    }
                    if self.net.cyclic_active.contains(&key) {
                        if ui.small_button("Stop").clicked() {
                            stop = Some(key);
                        }
                    } else if ui.add_enabled(can_send && f.is_ok(), egui::Button::new("Cyclic").small()).clicked() {
                        if let Ok(f) = &f {
                            start = Some((key, *f, s.period_ms as u64));
                        }
                    }
                    if ui.small_button("Delete").clicked() {
                        delete = Some(i);
                    }
                });
                ui.end_row();
            }
        });
        if let Some(i) = load {
            self.net.edit = self.settings.saved_frames[i].clone();
            self.net.id_text = format!("{:X}", self.net.edit.id);
        }
        if let Some(f) = send {
            self.bus.send(BusCommand::Send(f));
        }
        if let Some((key, frame, period_ms)) = start {
            self.bus.send(BusCommand::StartCyclic { key, frame, period_ms });
            self.net.cyclic_active.insert(key);
        }
        if let Some(key) = stop {
            self.bus.send(BusCommand::StopCyclic(key));
            self.net.cyclic_active.remove(&key);
        }
        if let Some(i) = delete {
            let key = name_key(&self.settings.saved_frames[i].name);
            if self.net.cyclic_active.remove(&key) {
                self.bus.send(BusCommand::StopCyclic(key));
            }
            self.settings.saved_frames.remove(i);
            self.mark_settings();
        }
    }
}
