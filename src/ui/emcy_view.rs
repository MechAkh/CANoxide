//! Emergencies view: the history of the received EMCY frames (newest
//! first, node filter, clear, CSV export) and the error history object
//! 0x1003 of a node (read; clear with a write of 0 to 0x1003:00).

use super::theme::{self, Tone};
use super::{App, TaskOwner};
use crate::bus::BusCommand;
use crate::canopen::task::{StepsTask, TaskResult};
use crate::emcy::{clear_history_step, error_history, error_history_task, history_csv, ErrorField};
use crate::settings::AdapterKind;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

pub(crate) struct EmcyView {
    /// Node filter of the table (None = all nodes).
    pub node: Option<u8>,
    /// The last read of 0x1003: (node, entries or the error text).
    pub history: Option<(u8, Result<Vec<ErrorField>, String>)>,
    /// The text of the last clear of 0x1003.
    pub clear_note: Option<String>,
    /// Simulator controls (hex).
    pub sim_code: String,
    pub sim_reg: String,
}

impl Default for EmcyView {
    fn default() -> Self {
        EmcyView { node: None, history: None, clear_note: None, sim_code: "8130".to_string(), sim_reg: "11".to_string() }
    }
}

fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

impl App {
    fn emcy_busy(&self) -> bool {
        self.tasks.values().any(|o| matches!(o, TaskOwner::ErrorHistory | TaskOwner::ClearHistory))
    }

    /// The node of the 0x1003 commands: the filter node, else the active node.
    fn emcy_target(&self) -> u8 {
        self.emv.node.unwrap_or_else(|| self.active_node())
    }

    fn device_name(&self, node: u8) -> Option<String> {
        self.lib.as_ref()?.devices.iter().find(|d| d.node_id == node).map(|d| d.name.clone())
    }

    pub(crate) fn read_error_history(&mut self, node: u8) {
        self.emv.history = None;
        self.emv.clear_note = None;
        self.start_task(TaskOwner::ErrorHistory, Box::new(error_history_task(node)));
        self.set_status(format!("Reading the error history (0x1003) of node {node}"), false);
    }

    pub(crate) fn clear_error_history(&mut self, node: u8) {
        let ask = self.settings.confirm_before_write;
        self.confirm_then(ask, "Clear error history", format!("Write 0 to 0x1003:00 of node {node}? This clears the error history of the node."), "Clear", true, move |app| {
            app.emv.clear_note = None;
            app.start_task(TaskOwner::ClearHistory, Box::new(StepsTask::new("clear error history", vec![clear_history_step(node)], true)));
        });
    }

    pub(crate) fn on_emcy_task(&mut self, owner: TaskOwner, result: TaskResult) {
        match (owner, result) {
            (TaskOwner::ErrorHistory, TaskResult::Reads(r)) => {
                let h = error_history(&r);
                match &h {
                    Ok(v) => self.set_status(format!("Error history of node {}: {} entr{}", r.node, v.len(), if v.len() == 1 { "y" } else { "ies" }), false),
                    Err(e) => self.set_status(format!("Error history of node {} not read: {e}", r.node), true),
                }
                self.emv.history = Some((r.node, h));
            }
            (TaskOwner::ClearHistory, TaskResult::Steps(r)) => {
                let node = r.done.first().map(|s| s.step.node).unwrap_or(0);
                let t = match r.done.first() {
                    Some(s) if s.ok => format!("Error history of node {node} cleared"),
                    Some(s) => format!("Error history of node {node} not cleared: {}", s.error.clone().unwrap_or_default()),
                    None => "Error history not cleared (cancelled)".to_string(),
                };
                self.set_status(t.clone(), !r.ok);
                self.emv.clear_note = Some(t);
                if r.ok {
                    self.emv.history = Some((node, Ok(Vec::new())));
                }
            }
            _ => {}
        }
    }

    pub(crate) fn emcy_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let outlined = self.settings.reduce_colour;
        let (entries, nodes, total, dropped) = {
            let s = self.bus.shared.lock().unwrap_or_else(|p| p.into_inner());
            (s.network.emcy.newest_first(self.emv.node), s.network.emcy.nodes(), s.network.emcy.len(), s.network.emcy.dropped)
        };
        egui::ScrollArea::vertical().id_salt("emcy_scroll").show(ui, |ui| {
            theme::toolbar(ui, |ui| {
                ui.label("Node");
                let sel = match self.emv.node {
                    None => "All nodes".to_string(),
                    Some(n) => format!("Node {n}"),
                };
                let mut choices: Vec<u8> = nodes.clone();
                if let Some(n) = self.emv.node {
                    if !choices.contains(&n) {
                        choices.push(n);
                        choices.sort_unstable();
                    }
                }
                theme::toolbar_combo(ui, "emcy_node", None, &sel, None, |ui| {
                    ui.selectable_value(&mut self.emv.node, None, "All nodes");
                    for n in &choices {
                        ui.selectable_value(&mut self.emv.node, Some(*n), format!("Node {n}"));
                    }
                });
                if ui.add_enabled(total > 0, egui::Button::new(format!("{} Clear", egui_phosphor::regular::TRASH))).on_hover_text("Clear the list (nothing is sent)").clicked() {
                    self.bus.shared.lock().unwrap_or_else(|p| p.into_inner()).network.emcy.clear();
                }
                if ui.add_enabled(!entries.is_empty(), egui::Button::new("Export CSV...")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().set_file_name("emergencies.csv").add_filter("CSV", &["csv"]).save_file() {
                        let csv = history_csv(&entries, |n| self.device_name(n));
                        match crate::library::write_atomic(&path, csv.as_bytes()) {
                            Ok(()) => self.set_status(format!("Saved {} EMCY entr{} to {}", entries.len(), if entries.len() == 1 { "y" } else { "ies" }, path.display()), false),
                            Err(e) => self.set_status(format!("CSV not written: {}", e.0), true),
                        }
                    }
                }
                let count = if self.emv.node.is_some() { format!("{} of {total} EMCY", entries.len()) } else { format!("{total} EMCY") };
                ui.label(egui::RichText::new(count).color(p.text_secondary));
                if dropped > 0 {
                    ui.label(egui::RichText::new(format!("({dropped} oldest dropped)")).color(p.text_secondary));
                }
            });
            if entries.is_empty() {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(egui_phosphor::regular::WARNING).size(28.0).color(p.text_secondary));
                    ui.label("No emergency received yet.");
                    ui.label(egui::RichText::new("Each EMCY frame on the bus (COB-ID 0x081..0x0FF) is listed here, newest first.").color(p.text_secondary));
                });
            } else {
                self.emcy_table(ui, &entries, outlined);
            }

            ui.separator();
            let node = self.emcy_target();
            let busy = self.emcy_busy();
            ui.strong(format!("Error history (0x1003) of node {node}"));
            theme::toolbar(ui, |ui| {
                if self.send_button(ui, !busy, "Read error history (0x1003)").on_hover_text("Read 0x1003:00 and then each entry by SDO").clicked() {
                    self.read_error_history(node);
                }
                if self.styled_send_button(ui, !busy, false, "Clear error history", "Write 0 to 0x1003:00").clicked() {
                    self.clear_error_history(node);
                }
            });
            if let Some(t) = &self.emv.clear_note {
                ui.label(egui::RichText::new(t).color(p.text_secondary));
            }
            match &self.emv.history {
                None => {}
                Some((n, Err(e))) => {
                    ui.colored_label(p.error, format!("Node {n}: {e}"));
                }
                Some((n, Ok(v))) if v.is_empty() => {
                    ui.label(egui::RichText::new(format!("Node {n}: the error history is empty.")).color(p.text_secondary));
                }
                Some((n, Ok(v))) => {
                    ui.label(egui::RichText::new(format!("Node {n}: {} entr{} (1 = newest)", v.len(), if v.len() == 1 { "y" } else { "ies" })).color(p.text_secondary));
                    let row_h = ui.spacing().interact_size.y;
                    TableBuilder::new(ui)
                        .id_salt("emcy_1003")
                        .striped(self.settings.zebra)
                        .vscroll(false)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(Column::exact(44.0))
                        .column(Column::exact(64.0))
                        .column(Column::remainder().at_least(200.0).clip(true))
                        .column(Column::exact(64.0))
                        .header(row_h, |mut h| {
                            for t in ["No.", "Code", "Text", "Info"] {
                                h.col(|ui| {
                                    ui.strong(t);
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, v.len(), |mut row| {
                                let e = &v[row.index()];
                                row.col(|ui| {
                                    ui.monospace(e.number.to_string());
                                });
                                row.col(|ui| {
                                    ui.monospace(format!("{:04X}", e.code()));
                                });
                                row.col(|ui| {
                                    ui.add(egui::Label::new(e.text()).truncate());
                                });
                                row.col(|ui| {
                                    ui.monospace(format!("{:04X}", e.info()));
                                });
                            });
                        });
                }
            }

            if self.settings.adapter == AdapterKind::Simulator {
                ui.separator();
                ui.strong("Simulator");
                theme::toolbar(ui, |ui| {
                    ui.label("EMCY code 0x");
                    ui.add(egui::TextEdit::singleline(&mut self.emv.sim_code).desired_width(48.0));
                    ui.label("reg 0x");
                    ui.add(egui::TextEdit::singleline(&mut self.emv.sim_reg).desired_width(28.0));
                    let code = parse_hex(&self.emv.sim_code).and_then(|v| u16::try_from(v).ok());
                    let reg = parse_hex(&self.emv.sim_reg).and_then(|v| u8::try_from(v).ok());
                    if ui.add_enabled(self.connected && code.is_some() && reg.is_some(), egui::Button::new("Raise EMCY")).on_hover_text("The simulated node sends this EMCY and adds it to its 0x1003").clicked() {
                        if let (Some(code), Some(reg)) = (code, reg) {
                            self.bus.send(BusCommand::SimEmcy { code, reg });
                        }
                    }
                    if ui.add_enabled(self.connected, egui::Button::new("Reset EMCY")).on_hover_text("The simulated node sends EMCY 0x0000 (error reset)").clicked() {
                        self.bus.send(BusCommand::SimEmcy { code: 0, reg: 0 });
                    }
                });
            }
        });
    }

    fn emcy_table(&self, ui: &mut egui::Ui, entries: &[crate::emcy::EmcyEntry], outlined: bool) {
        let p = theme::pal(ui);
        let row_h = self.settings.row_density.row_height();
        let names: Vec<String> = entries.iter().map(|e| self.device_name(e.node).unwrap_or_default()).collect();
        egui::ScrollArea::horizontal().id_salt("emcy_h").show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt("emcy_table")
                .striped(self.settings.zebra)
                .vscroll(true)
                .max_scroll_height(360.0)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(96.0))
                .column(Column::initial(120.0).clip(true))
                .column(Column::initial(260.0).at_least(160.0).clip(true))
                .column(Column::initial(200.0).clip(true))
                .column(Column::exact(130.0))
                .header(row_h, |mut h| {
                    for t in ["Time", "Node", "Code", "Error register", "Manufacturer"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, entries.len(), |mut row| {
                        let i = row.index();
                        let e = &entries[i];
                        row.col(|ui| {
                            ui.monospace(format!("{:.3} s", e.time_us as f64 / 1e6));
                        });
                        row.col(|ui| {
                            let t = if names[i].is_empty() { e.node.to_string() } else { format!("{} {}", e.node, names[i]) };
                            ui.add(egui::Label::new(t).truncate());
                        });
                        row.col(|ui| {
                            let tone = if e.is_reset() { Tone::Success } else { Tone::Warning };
                            theme::tone_pill(ui, tone, format!("{:04X}", e.code), outlined);
                            let c = if e.is_reset() { p.success } else { p.warning };
                            ui.add(egui::Label::new(egui::RichText::new(e.text()).color(c)).truncate()).on_hover_text(e.text());
                        });
                        row.col(|ui| {
                            let w = e.register_words();
                            let t = if w.is_empty() { format!("{:02X}", e.register) } else { format!("{:02X} {w}", e.register) };
                            ui.add(egui::Label::new(egui::RichText::new(t).monospace()).truncate()).on_hover_text(w);
                        });
                        row.col(|ui| {
                            ui.monospace(e.mfr_hex());
                        });
                    });
                });
        });
    }
}
