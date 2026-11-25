//! Network view: NMT (one node / all nodes), SYNC, the network overview
//! (state, last seen, identity, last EMCY of every node), Refresh node
//! info, heartbeat of all nodes and the node scan.

use super::{theme, App, Dialog, TaskOwner};
use crate::bus::BusCommand;
use crate::library::Device;
use crate::canopen::nmt::NmtCommand;
use crate::canopen::task::{StepsTask, TaskResult};
use crate::params::{caps, query_task, restore_defaults, store_parameters, sub_name, ParamCaps};
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
    /// Parameters group: the sub of 0x1010 / 0x1011 (1 = all parameters).
    pub(crate) param_sub: u8,
    /// The last query of 0x1010 / 0x1011: (node, what it supports).
    pub(crate) param_caps: Option<(u8, ParamCaps)>,
    /// The result of the last store / restore.
    pub(crate) param_note: Option<(String, bool)>,
    /// Tag of a running store / restore / query task.
    pub(crate) param_task: Option<u64>,
}

impl Default for NetView {
    fn default() -> Self {
        NetView {
            scanning: false,
            scan_results: Vec::new(),
            emcy_code: "8130".to_string(),
            emcy_reg: "11".to_string(),
            hb_ms: 1000,
            task: None,
            param_sub: 1,
            param_caps: None,
            param_note: None,
            param_task: None,
        }
    }
}

/// The note after a successful restore of the defaults.
pub(crate) const RESTORE_NOTE: &str = "The defaults are active after a reset node or a power cycle.";

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

    pub(crate) fn on_param_task(&mut self, owner: TaskOwner, result: TaskResult) {
        self.net.param_task = None;
        match (owner, result) {
            (TaskOwner::ParamQuery, TaskResult::Reads(r)) => {
                let c = caps(&r);
                let err = c.store.error.is_some() && c.restore.error.is_some();
                self.set_status(format!("Store / restore parameters of node {}: {} store sub(s), {} restore sub(s)", r.node, c.store.subs.len(), c.restore.subs.len()), err);
                self.net.param_caps = Some((r.node, c));
            }
            (owner @ (TaskOwner::ParamStore(_) | TaskOwner::ParamRestore(_)), TaskResult::Steps(r)) => {
                let restore = matches!(owner, TaskOwner::ParamRestore(_));
                let sub = match owner {
                    TaskOwner::ParamStore(s) | TaskOwner::ParamRestore(s) => s,
                    _ => 0,
                };
                let node = r.done.first().map(|s| s.step.node).unwrap_or(0);
                let what = if restore { "Restore defaults" } else { "Store parameters" };
                let t = match r.done.first() {
                    Some(s) if s.ok => format!("{what} (0x{:04X}:{sub:02}, {}) on node {node} done", if restore { 0x1011 } else { 0x1010 }, sub_name(sub)),
                    Some(s) => format!("{what} on node {node} failed: {}", s.error.clone().unwrap_or_default()),
                    None => format!("{what} cancelled"),
                };
                self.set_status(t.clone(), !r.ok);
                let t = if r.ok && restore { format!("{t}. {RESTORE_NOTE}") } else { t };
                self.net.param_note = Some((t, !r.ok));
            }
            _ => {}
        }
    }

    fn start_param(&mut self, owner: TaskOwner, task: Box<dyn crate::canopen::task::SdoTask>) {
        self.net.param_note = None;
        let tag = self.start_task(owner, task);
        self.net.param_task = Some(tag);
    }

    pub(crate) fn store_params(&mut self, node: u8, sub: u8) {
        let ask = self.settings.confirm_before_write;
        self.confirm_then(ask, "Store parameters", format!("Write \"save\" to 0x1010:{sub:02} ({}) of node {node}? The node saves its parameters to its non-volatile memory.", sub_name(sub)), "Store", false, move |app| {
            app.start_param(TaskOwner::ParamStore(sub), Box::new(StepsTask::new("store parameters", vec![store_parameters(node, sub)], true)));
        });
    }

    pub(crate) fn restore_params(&mut self, node: u8, sub: u8) {
        // Always asks: the saved parameters of the node are lost.
        self.confirm_then(true, "Restore default parameters", format!("Write \"load\" to 0x1011:{sub:02} ({}) of node {node}? The saved parameters are lost. {RESTORE_NOTE}", sub_name(sub)), "Restore defaults", true, move |app| {
            app.start_param(TaskOwner::ParamRestore(sub), Box::new(StepsTask::new("restore default parameters", vec![restore_defaults(node, sub)], true)));
        });
    }

    fn params_section(&mut self, ui: &mut egui::Ui, node: u8) {
        let p = theme::pal(ui);
        let busy = self.net.param_task.is_some();
        // The subs to offer: those the node reported, else 1..3.
        let subs: Vec<u8> = match &self.net.param_caps {
            Some((n, c)) if *n == node && !c.store.subs.is_empty() => {
                let mut v: Vec<u8> = c.store.subs.iter().chain(c.restore.subs.iter()).map(|s| s.sub).collect();
                v.sort_unstable();
                v.dedup();
                v
            }
            _ => vec![1, 2, 3],
        };
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([72.0, ui.spacing().interact_size.y], egui::Label::new(format!("Node {node}")));
            egui::ComboBox::from_id_salt("param_sub").selected_text(sub_name(self.net.param_sub)).width(190.0).show_ui(ui, |ui| {
                for s in &subs {
                    ui.selectable_value(&mut self.net.param_sub, *s, format!("{} (sub {s})", sub_name(*s)));
                }
            });
            let sub = self.net.param_sub;
            if self.styled_send_button(ui, !busy, false, "Store (0x1010)", "Write \"save\" to 0x1010 of the node").clicked() {
                self.store_params(node, sub);
            }
            if self.styled_send_button(ui, !busy, true, "Restore defaults (0x1011)", "Write \"load\" to 0x1011 of the node (asks first)").clicked() {
                self.restore_params(node, sub);
            }
            if self.send_button(ui, !busy, "Read supported").on_hover_text("Read 0x1010 and 0x1011 (sub 0 and each sub) by SDO").clicked() {
                self.start_param(TaskOwner::ParamQuery, Box::new(query_task(node)));
            }
        });
        if let Some((n, c)) = &self.net.param_caps {
            if *n == node {
                let line = |o: &crate::params::ParamObject| match &o.error {
                    Some(e) => e.clone(),
                    None => o
                        .subs
                        .iter()
                        .map(|s| match (s.value, &s.error) {
                            (Some(v), _) => {
                                let how = match (s.on_command(), s.autonomous()) {
                                    (true, true) => "on command + autonomous".to_string(),
                                    (true, false) => "on command".to_string(),
                                    (false, true) => "autonomous".to_string(),
                                    (false, false) => format!("not supported (0x{v:08X})"),
                                };
                                format!("{} {how}", s.sub)
                            }
                            (None, Some(e)) => format!("{} {e}", s.sub),
                            (None, None) => format!("{} ?", s.sub),
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                ui.label(egui::RichText::new(format!("0x1010: {}", line(&c.store))).small().color(p.text_secondary));
                ui.label(egui::RichText::new(format!("0x1011: {}", line(&c.restore))).small().color(p.text_secondary));
            }
        }
        if let Some((t, err)) = &self.net.param_note {
            ui.colored_label(if *err { p.error } else { p.text_secondary }, t.as_str());
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
                ui.separator();
                if ui.button("Program download...").on_hover_text("Open the program download wizard for this node (CiA 302-3); nothing is sent yet").clicked() {
                    self.open_program_download();
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
            ui.label(egui::RichText::new("Parameters").strong());
            self.params_section(ui, node);

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
            let p = theme::pal(ui);
            let have_lib = self.lib.is_some();
            let devs: Vec<(u8, String)> = match &self.lib {
                Some(l) => l.devices.iter().map(|d| (d.node_id, d.name.clone())).collect(),
                None => Vec::new(),
            };
            let names: Vec<(u8, String)> = {
                let s = self.bus.shared.lock().unwrap_or_else(|p| p.into_inner());
                s.network.nodes().into_iter().filter_map(|n| n.device_name.clone().filter(|d| !d.trim().is_empty()).map(|d| (n.node, d))).collect()
            };
            let mut add: Option<u8> = None;
            egui::Grid::new("scan_res").striped(true).num_columns(3).show(ui, |ui| {
                for (n, r) in &res {
                    ui.label(format!("Node {n}"));
                    match r {
                        Ok(v) => ui.label(format!("device type 0x{v:08X} (profile {})", v & 0xFFFF)),
                        Err(e) => ui.label(format!("answered: {e}")),
                    };
                    match devs.iter().find(|d| d.0 == *n) {
                        Some((_, name)) => {
                            ui.label(egui::RichText::new(format!("Device: {name}")).color(p.text_secondary));
                        }
                        None => {
                            if ui.add_enabled(have_lib, egui::Button::new("Add as device").small()).on_hover_text("Open Add device with this node").clicked() {
                                add = Some(*n);
                            }
                        }
                    }
                    ui.end_row();
                }
            });
            if let Some(n) = add {
                let name = names.iter().find(|d| d.0 == n).map(|d| d.1.trim().to_string()).unwrap_or_else(|| format!("Node {n}"));
                let dev = Device { name, node_id: n, eds_file: self.selected_eds.clone(), bitrate_kbit: self.settings.bitrate_kbit, notes: String::new() };
                self.dialog = Some(Dialog::Device { idx: None, dev, error: None });
            }
        }
    }
}
