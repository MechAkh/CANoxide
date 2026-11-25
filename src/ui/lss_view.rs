//! LSS view (CiA 305): switch state, inquire, configure node-ID and bit
//! timing, activate, store, fastscan and identify. Each button sends one
//! LSS service; configure, activate and store ask first when the Safety
//! setting says so.

use super::theme::{self, Tone};
use super::App;
use crate::bus::BusCommand;
use crate::canopen::lss::{bit_timing_kbit, LssCommand, LssError, LssIdentity, LssResult};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;

/// One row of the LSS log (spec 8.8).
pub(crate) struct LssLogRow {
    pub time: String,
    pub service: String,
    pub request: String,
    pub result: &'static str,
    pub tone: Tone,
    pub detail: String,
}

/// The running service: tag, service label, request text.
pub(crate) struct LssRunning {
    pub tag: u64,
    pub label: String,
    pub request: String,
    /// The service has no answer (switch global, activate).
    pub unconfirmed: bool,
}

pub(crate) struct LssView {
    pub identity: LssIdentity,
    pub id_text: [String; 4],
    pub node_id: u8,
    pub bt_index: u8,
    pub delay_ms: u16,
    pub running: Option<LssRunning>,
    /// Newest last.
    pub log: Vec<LssLogRow>,
    pub last_identity: Option<LssIdentity>,
    /// Node-ID from the last Inquire node-ID (255 = not configured).
    pub current_node: Option<u8>,
}

impl Default for LssView {
    fn default() -> Self {
        LssView {
            identity: LssIdentity::default(),
            id_text: std::array::from_fn(|_| hex8(0)),
            node_id: 1,
            bt_index: 3,
            delay_ms: 1000,
            running: None,
            log: Vec::new(),
            last_identity: None,
            current_node: None,
        }
    }
}

fn hex8(v: u32) -> String {
    format!("0x{v:08X}")
}

fn id_text(i: &LssIdentity) -> String {
    format!("vendor 0x{:08X}, product 0x{:08X}, revision 0x{:08X}, serial 0x{:08X}", i.vendor, i.product, i.revision, i.serial)
}

/// Request text of a service for the log.
fn request_text(cmd: &LssCommand) -> String {
    match cmd {
        LssCommand::SwitchGlobal { configuration } => (if *configuration { "configuration" } else { "waiting" }).to_string(),
        LssCommand::SwitchSelective { identity } => format!("{:08X} {:08X} {:08X} {:08X}", identity.vendor, identity.product, identity.revision, identity.serial),
        LssCommand::ConfigureNodeId { node_id } => format!("node-ID {node_id}"),
        LssCommand::ConfigureBitTiming { index, .. } => format!("index {index}"),
        LssCommand::ActivateBitTiming { delay_ms } => format!("delay {delay_ms} ms"),
        _ => String::new(),
    }
}

/// Capitalised service label ("Configure node-ID").
fn service_text(cmd: &LssCommand) -> String {
    let l = cmd.label();
    let mut c = l.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl App {
    /// Send one LSS service (asks first for the changing ones).
    pub(crate) fn lss_send(&mut self, cmd: LssCommand) {
        let changes = matches!(cmd, LssCommand::ConfigureNodeId { .. } | LssCommand::ConfigureBitTiming { .. } | LssCommand::ActivateBitTiming { .. } | LssCommand::StoreConfiguration);
        let ask = changes && self.settings.confirm_lss;
        let text = match &cmd {
            LssCommand::ConfigureNodeId { node_id } => format!("Set node-ID {node_id} on the device in configuration state? It may leave this network."),
            LssCommand::ConfigureBitTiming { index, .. } => format!("Set the pending bit rate of the device in configuration state to {} kbit/s?", bit_timing_kbit(*index).map(|k| k.to_string()).unwrap_or_else(|| "auto".to_string())),
            LssCommand::ActivateBitTiming { delay_ms } => format!("Every LSS slave switches to the new bit rate after {delay_ms} ms. Nodes at the old bit rate stop talking to this tool."),
            LssCommand::StoreConfiguration => "Store the node-ID and bit timing in the slave's non-volatile memory.".to_string(),
            _ => String::new(),
        };
        let danger = matches!(cmd, LssCommand::ActivateBitTiming { .. });
        let ok = match &cmd {
            LssCommand::ConfigureNodeId { .. } => "Set node-ID",
            LssCommand::ConfigureBitTiming { .. } => "Set bit rate",
            LssCommand::ActivateBitTiming { .. } => "Activate",
            _ => "Send",
        };
        let label = service_text(&cmd);
        let request = request_text(&cmd);
        let unconfirmed = matches!(cmd, LssCommand::SwitchGlobal { .. } | LssCommand::ActivateBitTiming { .. });
        self.confirm_then(ask, "LSS", text, ok, danger, move |app| {
            let tag = app.tag();
            app.lssv.running = Some(LssRunning { tag, label, request, unconfirmed });
            app.bus.send(BusCommand::Lss { tag, cmd });
        });
    }

    pub(crate) fn on_lss_done(&mut self, tag: u64, result: Result<LssResult, LssError>) {
        let run = match self.lssv.running.take() {
            Some(r) if r.tag == tag => Some(r),
            Some(other) => {
                self.lssv.running = Some(other);
                None
            }
            None => None,
        };
        let (label, request, unconfirmed) = match run {
            Some(r) => (r.label, r.request, r.unconfirmed),
            None => ("LSS".to_string(), String::new(), false),
        };
        let (text, err) = match &result {
            Ok(LssResult::Done) => ("done".to_string(), false),
            Ok(LssResult::Identity(i)) => (id_text(i), false),
            Ok(LssResult::NodeId(n)) => (if *n == 255 { "node-ID 255 (not configured)".to_string() } else { format!("node-ID {n}") }, false),
            Ok(LssResult::Found(Some(i))) => (format!("found {}; it is in configuration state", id_text(i)), false),
            Ok(LssResult::Found(None)) => ("no unconfigured slave answered".to_string(), false),
            Ok(LssResult::NonConfigured(n)) => (format!("{n} non-configured slave(s) answered"), false),
            Err(e) => (e.to_string(), true),
        };
        // Inquire identity and Fastscan fill the identity fields.
        match &result {
            Ok(LssResult::Identity(i)) | Ok(LssResult::Found(Some(i))) => self.lss_fill_identity(*i),
            Ok(LssResult::NodeId(n)) => self.lssv.current_node = Some(*n),
            _ => {}
        }
        let (res, tone) = match &result {
            Ok(LssResult::Done) if unconfirmed => ("Sent", Tone::Info),
            Ok(LssResult::Found(None)) => ("No answer", Tone::Warning),
            Ok(LssResult::NonConfigured(0)) => ("No answer", Tone::Warning),
            Ok(_) => ("OK", Tone::Success),
            Err(LssError::Timeout) => ("Timeout", Tone::Error),
            Err(LssError::Refused { .. }) => ("Error", Tone::Error),
            Err(LssError::Cancelled) => ("Cancelled", Tone::Neutral),
            Err(_) => ("Error", Tone::Error),
        };
        let mut lower = label.clone();
        if let Some(f) = lower.get_mut(0..1) {
            f.make_ascii_lowercase();
        }
        self.set_status(format!("LSS {lower}: {text}"), err);
        let now = self.bus.shared.lock().map(|s| s.now_us).unwrap_or(0);
        self.lssv.log.push(LssLogRow { time: format!("{:.3}", now as f64 / 1e6), service: label, request, result: res, tone, detail: text });
        if self.lssv.log.len() > 200 {
            self.lssv.log.remove(0);
        }
    }

    fn lss_fill_identity(&mut self, i: LssIdentity) {
        self.lssv.last_identity = Some(i);
        self.lssv.identity = i;
        self.lssv.id_text = [hex8(i.vendor), hex8(i.product), hex8(i.revision), hex8(i.serial)];
    }

    pub(crate) fn lss_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let can = self.can_send();
        let idle = self.lssv.running.is_none();
        let ok = idle;
        if !can {
            ui.colored_label(p.warning, if self.listen_only_active { "Listen only: every LSS service is disabled." } else { "Not connected: LSS services are disabled." });
        }
        egui::ScrollArea::vertical().id_salt("lss_scroll").show(ui, |ui| {
            ui.strong("State");
            theme::toolbar(ui, |ui| {
                if self.send_button(ui, ok, "Switch all to configuration").clicked() {
                    self.lss_send(LssCommand::SwitchGlobal { configuration: true });
                }
                if self.send_button(ui, ok, "Switch all to waiting").clicked() {
                    self.lss_send(LssCommand::SwitchGlobal { configuration: false });
                }
            });
            ui.separator();
            ui.strong("Identity");
            let mut valid = true;
            egui::Grid::new("lss_identity").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                let names = ["Vendor ID", "Product code", "Revision", "Serial"];
                for (i, n) in names.iter().enumerate() {
                    ui.label(*n);
                    let t = self.lssv.id_text[i].trim().trim_start_matches("0x").trim_start_matches("0X").to_string();
                    let parsed = if t.len() <= 8 { u32::from_str_radix(&t, 16).ok() } else { None };
                    let mut edit = egui::TextEdit::singleline(&mut self.lssv.id_text[i]).desired_width(120.0).font(egui::TextStyle::Monospace).hint_text("0x00000000");
                    if parsed.is_none() {
                        edit = edit.text_color(p.error);
                    }
                    ui.add(edit).on_hover_text("8 hex digits");
                    match parsed {
                        Some(v) => match i {
                            0 => self.lssv.identity.vendor = v,
                            1 => self.lssv.identity.product = v,
                            2 => self.lssv.identity.revision = v,
                            _ => self.lssv.identity.serial = v,
                        },
                        None => valid = false,
                    }
                    ui.end_row();
                }
                ui.label("Current node-ID");
                let cur = match self.lssv.current_node {
                    Some(255) => "255 (not configured)".to_string(),
                    Some(n) => n.to_string(),
                    None => "\u{2014}".to_string(),
                };
                ui.label(egui::RichText::new(cur).monospace()).on_hover_text("From Inquire node-ID");
                ui.end_row();
            });
            theme::toolbar(ui, |ui| {
                if self.send_button(ui, ok && valid, "Switch selective").on_hover_text("Switch state selective: only the device with this identity enters configuration").clicked() {
                    self.lss_send(LssCommand::SwitchSelective { identity: self.lssv.identity });
                }
                if let Some(i) = self.lssv.last_identity {
                    if ui.button("Use last identity").clicked() {
                        self.lss_fill_identity(i);
                    }
                }
            });
            ui.separator();
            ui.strong("Inquire and find");
            theme::toolbar(ui, |ui| {
                if self.send_button(ui, ok, "Inquire identity").on_hover_text("Read the identity of the device in configuration state; fills the identity fields").clicked() {
                    self.lss_send(LssCommand::InquireIdentity);
                }
                if self.send_button(ui, ok, "Inquire node-ID").on_hover_text("Read the node-ID of the device in configuration state").clicked() {
                    self.lss_send(LssCommand::InquireNodeId);
                }
                if self.send_button(ui, ok, "Fastscan").on_hover_text("Find one unconfigured slave; fills the identity fields").clicked() {
                    self.lss_send(LssCommand::Fastscan);
                }
                if self.send_button(ui, ok, "Identify non-configured").clicked() {
                    self.lss_send(LssCommand::IdentifyNonConfigured);
                }
            });
            ui.separator();
            ui.strong("Configure (slave in configuration state)");
            theme::toolbar(ui, |ui| {
                ui.label("New node-ID");
                ui.add(egui::DragValue::new(&mut self.lssv.node_id).range(1..=127));
                if self.styled_send_button(ui, ok, false, "Configure node-ID", "Set the node-ID of the device in configuration state").clicked() {
                    self.lss_send(LssCommand::ConfigureNodeId { node_id: self.lssv.node_id });
                }
            });
            theme::caution_note(ui, "The device answers on the new node-ID after a reset; it may leave this network configuration.");
            ui.add_space(4.0);
            theme::toolbar(ui, |ui| {
                ui.label("New bit rate");
                let cur = bit_timing_kbit(self.lssv.bt_index).map(|k| format!("{k} kbit/s")).unwrap_or_else(|| "auto".to_string());
                theme::toolbar_combo(ui, "lss_bt", None, &cur, None, |ui| {
                    for i in 0..=8u8 {
                        if let Some(k) = bit_timing_kbit(i) {
                            ui.selectable_value(&mut self.lssv.bt_index, i, format!("{k} kbit/s"));
                        }
                    }
                });
                ui.label(egui::RichText::new(format!("index {}", self.lssv.bt_index)).small().color(p.text_secondary));
                if self.styled_send_button(ui, ok, false, "Configure bit timing", "Set the pending bit rate of the device in configuration state").clicked() {
                    self.lss_send(LssCommand::ConfigureBitTiming { table: 0, index: self.lssv.bt_index });
                }
            });
            theme::toolbar(ui, |ui| {
                ui.label("Switch delay");
                ui.add(egui::DragValue::new(&mut self.lssv.delay_ms).range(1..=10_000).suffix(" ms"));
                if self.styled_send_button(ui, ok, true, "Activate bit timing", "All devices in configuration state switch to the new bit rate after the delay").clicked() {
                    self.lss_send(LssCommand::ActivateBitTiming { delay_ms: self.lssv.delay_ms });
                }
            });
            theme::caution_note(ui, "Devices on another bit rate leave this bus. Set the adapter bit rate to match afterwards.");
            ui.add_space(4.0);
            theme::toolbar(ui, |ui| {
                let r = self.send_button(ui, ok, format!("{}  Store configuration", icon::FLOPPY_DISK)).on_hover_text("Device stores node-ID and bit rate in non-volatile memory");
                let en = r.enabled();
                r.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, en, "Store configuration"));
                if r.clicked() {
                    self.lss_send(LssCommand::StoreConfiguration);
                }
            });
            ui.separator();
            if let Some(r) = &self.lssv.running {
                let l = r.label.clone();
                let prog = self.bus.shared.lock().ok().and_then(|s| s.lss);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{l}{}", prog.map(|(d, t)| format!(": {d} / {t}")).unwrap_or_default()));
                    if ui.button("Cancel").clicked() {
                        self.bus.send(BusCommand::CancelLss);
                    }
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            ui.horizontal(|ui| {
                ui.strong("Log");
                if !self.lssv.log.is_empty() && theme::icon_button(ui, true, icon::TRASH, "Clear log", "Clear the LSS log").clicked() {
                    self.lssv.log.clear();
                }
            });
            if self.lssv.log.is_empty() {
                ui.label(egui::RichText::new("No LSS service run yet.").color(p.text_secondary));
                return;
            }
            self.lss_log_table(ui);
        });
    }

    fn lss_log_table(&self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let row_h = self.settings.row_density.row_height().max(ui.spacing().interact_size.y);
        let outlined = self.settings.reduce_colour;
        egui::ScrollArea::horizontal().id_salt("lss_log_h").show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt("lss_log")
                .striped(self.settings.zebra)
                .vscroll(false)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(80.0))
                .column(Column::exact(170.0).clip(true))
                .column(Column::exact(150.0).clip(true))
                .column(Column::exact(100.0))
                .column(Column::remainder().at_least(160.0).clip(true))
                .header(row_h, |mut h| {
                    for t in ["Time", "Service", "Request", "Result", "Detail"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|mut body| {
                    for r in &self.lssv.log {
                        body.row(row_h, |mut row| {
                            row.col(|ui| {
                                ui.monospace(&r.time);
                            });
                            row.col(|ui| {
                                ui.label(&r.service);
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(egui::RichText::new(&r.request).monospace()).truncate());
                            });
                            row.col(|ui| {
                                theme::tone_pill(ui, r.tone, r.result, outlined);
                            });
                            row.col(|ui| {
                                let c = if r.tone == Tone::Error { p.error } else { p.text_primary };
                                ui.add(egui::Label::new(egui::RichText::new(&r.detail).color(c)).truncate()).on_hover_text(&r.detail);
                            });
                        });
                    }
                });
        });
    }
}
