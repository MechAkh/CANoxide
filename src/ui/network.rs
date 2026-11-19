//! Network view: NMT (one node / all nodes), SYNC, the network overview
//! (state, last seen, identity, last EMCY of every node), Refresh node
//! info, heartbeat of all nodes and the node scan.

use super::{theme, App, TaskOwner};
use crate::bus::BusCommand;
use crate::canopen::nmt::NmtCommand;
use crate::canopen::task::TaskResult;
use crate::settings::AdapterKind;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

pub(crate) struct NetView {
    pub(crate) scanning: bool,
    pub(crate) scan_results: Vec<(u8, Result<u32, String>)>,
    emcy_code: String,
    emcy_reg: String,
    hb_ms: u16,
    /// Tag of a running node info / heartbeat task.
    pub(crate) task: Option<u64>,
}

impl Default for NetView {
    fn default() -> Self {
        NetView { scanning: false, scan_results: Vec::new(), emcy_code: "8130".to_string(), emcy_reg: "11".to_string(), hb_ms: 1000, task: None }
    }
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

impl App {
    pub(crate) fn on_net_task(&mut self, owner: TaskOwner, result: TaskResult) {
        self.net.task = None;
        match (owner, result) {
            (TaskOwner::NodeInfo, TaskResult::NodeInfo(v)) => {
                let errs: usize = v.iter().filter(|i| !i.read_errors.is_empty()).count();
                let msg = format!("Node info read from {} node(s){}", v.len(), if errs > 0 { format!(", {errs} with errors") } else { String::new() });
                self.set_status(msg, errs > 0);
            }
            (TaskOwner::Heartbeat, TaskResult::Steps(r)) => {
                let ok = r.done.iter().filter(|s| s.ok).count();
                let msg = format!("Heartbeat set on {ok} of {} node(s){}", r.done.len() + r.not_run.len(), if r.cancelled { " (cancelled)" } else { "" });
                self.set_status(msg, !r.ok);
            }
            _ => {}
        }
    }

    fn nmt_all(&mut self, c: NmtCommand) {
        let ask = self.settings.confirm_nmt_all;
        self.confirm_then(ask, "NMT to all nodes", format!("Send NMT {} to all nodes (node 0)?", c.label()), c.label(), c == NmtCommand::ResetNode || c == NmtCommand::Stop, move |app| {
            app.bus.send(BusCommand::Nmt { cmd: c, node: 0 });
            app.set_status(format!("NMT {} all nodes sent", c.label()), false);
        });
    }

    pub(crate) fn network_tab(&mut self, ui: &mut egui::Ui) {
        let can_send = self.can_send();
        let node = self.active_node();
        let p = theme::pal(ui);
        egui::ScrollArea::vertical().id_salt("net_scroll").show(ui, |ui| {
            if !can_send {
                ui.colored_label(p.warning, if self.listen_only_active { "Listen only: every send control is disabled." } else { "Not connected: send controls are disabled." });
            }
            ui.label(egui::RichText::new("NMT").strong());
            // Each group wraps onto rows that fit the width of the view.
            ui.horizontal_wrapped(|ui| {
                ui.add_sized([72.0, ui.spacing().interact_size.y], egui::Label::new(format!("Node {node}")));
                for c in NmtCommand::ALL {
                    if self.send_button(ui, true, c.label()).clicked() {
                        self.bus.send(BusCommand::Nmt { cmd: c, node });
                        self.set_status(format!("NMT {} node {node} sent", c.label()), false);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_sized([72.0, ui.spacing().interact_size.y], egui::Label::new("All nodes"));
                for c in NmtCommand::ALL {
                    if self.send_button(ui, true, c.label()).clicked() {
                        self.nmt_all(c);
                    }
                }
            });
            // Rarely used controls.
            egui::CollapsingHeader::new("More").id_salt("net_more").default_open(false).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.send_button(ui, true, "Send SYNC").clicked() {
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
            });

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Network overview").strong());
                ui.separator();
                let busy = self.net.task.is_some();
                if self.send_button(ui, !busy, "Refresh node info").on_hover_text("Read 0x1000, 0x1001, 0x1018 and 0x1008 of every known node by SDO").clicked() {
                    let tag = self.tag();
                    self.tasks.insert(tag, TaskOwner::NodeInfo);
                    self.net.task = Some(tag);
                    self.bus.send(BusCommand::RefreshNodeInfo { tag, nodes: None });
                    self.set_status("Reading node info", false);
                }
                ui.separator();
                ui.label("Heartbeat of all nodes");
                ui.add(egui::DragValue::new(&mut self.net.hb_ms).range(0..=65000).suffix(" ms"));
                if self.send_button(ui, !busy, "Set heartbeat").on_hover_text("Write 0x1017 on every known node").clicked() {
                    let ms = self.net.hb_ms;
                    let ask = self.settings.confirm_nmt_all;
                    self.confirm_then(ask, "Heartbeat of all nodes", format!("Write the producer heartbeat time 0x1017 = {ms} ms on every known node?"), "Write", false, move |app| {
                        let tag = app.tag();
                        app.tasks.insert(tag, TaskOwner::Heartbeat);
                        app.net.task = Some(tag);
                        app.bus.send(BusCommand::SetHeartbeatAll { tag, ms, nodes: None });
                    });
                }
                if let Some(t) = self.net.task {
                    if ui.button("Cancel").clicked() {
                        self.bus.send(BusCommand::CancelTask(t));
                    }
                }
            });
            // The table scrolls sideways in a narrow view.
            egui::ScrollArea::horizontal().id_salt("nodes_h").show(ui, |ui| self.node_table(ui));

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Scan nodes").strong());
            self.scan_section(ui);
        });
    }

    fn node_table(&mut self, ui: &mut egui::Ui) {
        let (infos, now) = {
            let s = self.bus.shared.lock().unwrap_or_else(|p| p.into_inner());
            (s.network.nodes().into_iter().cloned().collect::<Vec<_>>(), s.now_us)
        };
        let p = theme::pal(ui);
        if infos.is_empty() {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::GRAPH).size(28.0).color(p.text_secondary));
                ui.label("No node seen yet. Nodes appear when they send a heartbeat, an EMCY or an SDO answer.");
                ui.label(egui::RichText::new("Scan nodes or Refresh node info read them by SDO.").color(p.text_secondary));
            });
            return;
        }
        let devs: Vec<(u8, String)> = match &self.lib {
            Some(l) => l.devices.iter().map(|d| (d.node_id, d.name.clone())).collect(),
            None => Vec::new(),
        };
        let row_h = self.settings.row_density.row_height();
        let outlined = self.settings.reduce_colour;
        TableBuilder::new(ui)
            .id_salt("nodes")
            .striped(self.settings.zebra)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(44.0))
            .column(Column::initial(150.0))
            .column(Column::initial(80.0))
            .column(Column::initial(110.0))
            .column(Column::initial(60.0))
            .column(Column::initial(170.0).clip(true))
            .column(Column::initial(120.0).clip(true))
            .column(Column::remainder().clip(true))
            .vscroll(false)
            .header(row_h, |mut h| {
                for t in ["Node", "State", "Last seen", "Device type", "Err reg", "Identity (vendor / product / rev / serial)", "Name", "Last EMCY"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for n in &infos {
                    body.row(row_h, |mut row| {
                        row.col(|ui| {
                            ui.monospace(n.node.to_string());
                        });
                        row.col(|ui| {
                            let label = n.state.clone().unwrap_or_else(|| "unknown".to_string());
                            let (fg, bg, icon) = theme::state_colours(p, &label);
                            theme::pill(ui, format!("{icon} {label}"), fg, bg, outlined);
                        });
                        row.col(|ui| {
                            let t = n.last_seen_us.map(|t| format!("{:.1} s ago", now.saturating_sub(t) as f64 / 1e6)).unwrap_or_default();
                            ui.label(t);
                        });
                        row.col(|ui| {
                            ui.monospace(n.device_type.map(|v| format!("0x{v:08X}")).unwrap_or_default());
                        });
                        row.col(|ui| {
                            let t = n.error_register.map(|v| format!("0x{v:02X}")).unwrap_or_default();
                            if n.error_register.is_some_and(|v| v != 0) {
                                ui.colored_label(p.error, egui::RichText::new(t).monospace());
                            } else {
                                ui.monospace(t);
                            }
                        });
                        row.col(|ui| {
                            let f = |v: Option<u32>| v.map(|v| format!("{v:X}")).unwrap_or_else(|| "-".to_string());
                            let id = if n.vendor_id.is_some() { format!("{} / {} / {} / {}", f(n.vendor_id), f(n.product_code), f(n.revision), f(n.serial)) } else { String::new() };
                            ui.monospace(id);
                        });
                        row.col(|ui| {
                            let name = n.device_name.clone().or_else(|| devs.iter().find(|d| d.0 == n.node).map(|d| d.1.clone())).unwrap_or_default();
                            let r = ui.add(egui::Label::new(name).truncate());
                            if !n.read_errors.is_empty() {
                                r.on_hover_text(n.read_errors.join("\n"));
                            }
                        });
                        row.col(|ui| {
                            if let Some(e) = &n.last_emcy {
                                ui.colored_label(p.warning, format!("0x{:04X} {}", e.code, e.text));
                            }
                        });
                    });
                }
            });
    }

    /// The progress of a running node scan (done, total).
    pub(crate) fn scan_progress(&self) -> Option<(usize, usize)> {
        self.bus.shared.lock().map(|s| s.scan).unwrap_or(None)
    }

    /// A node scan runs (started here, or the worker reports progress).
    pub(crate) fn scan_running(&self) -> bool {
        self.net.scanning || self.scan_progress().is_some()
    }

    /// Start the node scan with the settings (the one scan of the app: the
    /// Network view and the connection bar both call this). The caller
    /// enables it only when sending is allowed (`send_button`).
    pub(crate) fn start_scan(&mut self) {
        let (a, b) = (self.settings.scan_first, self.settings.scan_last);
        self.net.scan_results.clear();
        self.net.scanning = true;
        self.bus.send(BusCommand::Scan { from: a, to: b, timeout_ms: self.settings.scan_timeout_ms });
        self.set_status(format!("Scanning nodes {a}..{b}"), false);
    }

    pub(crate) fn cancel_scan(&mut self) {
        self.bus.send(BusCommand::CancelScan);
    }

    fn scan_section(&mut self, ui: &mut egui::Ui) {
        let progress = self.scan_progress();
        ui.horizontal_wrapped(|ui| {
            let (a, b) = (self.settings.scan_first, self.settings.scan_last);
            ui.label(format!("SDO read of 0x1000 on nodes {a}..{b}, one at a time, timeout"));
            if ui.add(egui::DragValue::new(&mut self.settings.scan_timeout_ms).range(20..=2000).suffix(" ms")).changed() {
                self.mark_settings();
            }
            if self.net.scanning || progress.is_some() {
                if ui.button("Cancel").clicked() {
                    self.cancel_scan();
                }
            } else if self.send_button(ui, true, "Scan nodes").clicked() {
                self.start_scan();
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
}
