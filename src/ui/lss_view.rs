//! LSS view (CiA 305): switch state, inquire, configure node-ID and bit
//! timing, activate, store, fastscan and identify. Each button sends one
//! LSS service; configure, activate and store ask first when the Safety
//! setting says so.

use super::{theme, App};
use crate::bus::BusCommand;
use crate::canopen::lss::{bit_timing_kbit, LssCommand, LssError, LssIdentity, LssResult};
use eframe::egui;

pub(crate) struct LssView {
    pub identity: LssIdentity,
    pub id_text: [String; 4],
    pub node_id: u8,
    pub bt_index: u8,
    pub delay_ms: u16,
    /// Tag and label of the running service.
    pub running: Option<(u64, String)>,
    /// (service, result text, error) log, newest last.
    pub log: Vec<(String, String, bool)>,
    pub last_identity: Option<LssIdentity>,
}

impl Default for LssView {
    fn default() -> Self {
        LssView {
            identity: LssIdentity::default(),
            id_text: [String::from("0"), String::from("0"), String::from("0"), String::from("0")],
            node_id: 1,
            bt_index: 3,
            delay_ms: 1000,
            running: None,
            log: Vec::new(),
            last_identity: None,
        }
    }
}

fn id_text(i: &LssIdentity) -> String {
    format!("vendor 0x{:08X}, product 0x{:08X}, revision 0x{:08X}, serial 0x{:08X}", i.vendor, i.product, i.revision, i.serial)
}

impl App {
    /// Send one LSS service (asks first for the changing ones).
    pub(crate) fn lss_send(&mut self, cmd: LssCommand) {
        let changes = matches!(cmd, LssCommand::ConfigureNodeId { .. } | LssCommand::ConfigureBitTiming { .. } | LssCommand::ActivateBitTiming { .. } | LssCommand::StoreConfiguration);
        let ask = changes && self.settings.confirm_lss;
        let text = match &cmd {
            LssCommand::ConfigureNodeId { node_id } => format!("Set the node-ID of the slave in configuration state to {node_id}."),
            LssCommand::ConfigureBitTiming { index, .. } => format!("Set the pending bit rate of the slave to {} kbit/s.", bit_timing_kbit(*index).map(|k| k.to_string()).unwrap_or_else(|| "auto".to_string())),
            LssCommand::ActivateBitTiming { delay_ms } => format!("Every LSS slave switches to the new bit rate after {delay_ms} ms. Nodes at the old bit rate stop talking to this tool."),
            LssCommand::StoreConfiguration => "Store the node-ID and bit timing in the slave's non-volatile memory.".to_string(),
            _ => String::new(),
        };
        let label = cmd.label().to_string();
        self.confirm_then(ask, "LSS", text, "Send", true, move |app| {
            let tag = app.tag();
            app.lssv.running = Some((tag, label));
            app.bus.send(BusCommand::Lss { tag, cmd });
        });
    }

    pub(crate) fn on_lss_done(&mut self, tag: u64, result: Result<LssResult, LssError>) {
        let label = match self.lssv.running.take() {
            Some((t, l)) if t == tag => l,
            Some(other) => {
                self.lssv.running = Some(other);
                "LSS".to_string()
            }
            None => "LSS".to_string(),
        };
        let (text, err) = match result {
            Ok(LssResult::Done) => ("done".to_string(), false),
            Ok(LssResult::Identity(i)) => {
                self.lssv.last_identity = Some(i);
                (id_text(&i), false)
            }
            Ok(LssResult::NodeId(n)) => (if n == 255 { "node-ID 255 (not configured)".to_string() } else { format!("node-ID {n}") }, false),
            Ok(LssResult::Found(Some(i))) => {
                self.lssv.last_identity = Some(i);
                (format!("found {}; it is in configuration state", id_text(&i)), false)
            }
            Ok(LssResult::Found(None)) => ("no unconfigured slave answered".to_string(), false),
            Ok(LssResult::NonConfigured(n)) => (format!("{n} non-configured slave(s) answered"), false),
            Err(e) => (e.to_string(), true),
        };
        self.set_status(format!("LSS {label}: {text}"), err);
        self.lssv.log.push((label, text, err));
        if self.lssv.log.len() > 200 {
            self.lssv.log.remove(0);
        }
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
            ui.horizontal_wrapped(|ui| {
                if self.send_button(ui, ok, "Switch all to configuration").clicked() {
                    self.lss_send(LssCommand::SwitchGlobal { configuration: true });
                }
                if self.send_button(ui, ok, "Switch all to waiting").clicked() {
                    self.lss_send(LssCommand::SwitchGlobal { configuration: false });
                }
            });
            ui.horizontal_wrapped(|ui| {
                let names = ["Vendor", "Product", "Revision", "Serial"];
                let mut valid = true;
                for (i, n) in names.iter().enumerate() {
                    ui.label(*n);
                    if ui.add(egui::TextEdit::singleline(&mut self.lssv.id_text[i]).desired_width(86.0).font(egui::TextStyle::Monospace)).changed() {
                        // parsed below
                    }
                    let t = self.lssv.id_text[i].trim().trim_start_matches("0x").trim_start_matches("0X").to_string();
                    match u32::from_str_radix(&t, 16) {
                        Ok(v) => match i {
                            0 => self.lssv.identity.vendor = v,
                            1 => self.lssv.identity.product = v,
                            2 => self.lssv.identity.revision = v,
                            _ => self.lssv.identity.serial = v,
                        },
                        Err(_) => valid = false,
                    }
                }
                if self.send_button(ui, ok && valid, "Switch selective").on_hover_text("Identity in hex").clicked() {
                    self.lss_send(LssCommand::SwitchSelective { identity: self.lssv.identity });
                }
                if let Some(i) = self.lssv.last_identity {
                    if ui.button("Use last identity").clicked() {
                        self.lssv.identity = i;
                        self.lssv.id_text = [format!("{:X}", i.vendor), format!("{:X}", i.product), format!("{:X}", i.revision), format!("{:X}", i.serial)];
                    }
                }
            });
            ui.separator();
            ui.strong("Inquire and find");
            ui.horizontal_wrapped(|ui| {
                if self.send_button(ui, ok, "Inquire identity").clicked() {
                    self.lss_send(LssCommand::InquireIdentity);
                }
                if self.send_button(ui, ok, "Inquire node-ID").clicked() {
                    self.lss_send(LssCommand::InquireNodeId);
                }
                if self.send_button(ui, ok, "Fastscan").on_hover_text("Find one unconfigured slave").clicked() {
                    self.lss_send(LssCommand::Fastscan);
                }
                if self.send_button(ui, ok, "Identify non-configured").clicked() {
                    self.lss_send(LssCommand::IdentifyNonConfigured);
                }
            });
            ui.separator();
            ui.strong("Configure (slave in configuration state)");
            ui.horizontal_wrapped(|ui| {
                ui.label("Node-ID");
                ui.add(egui::DragValue::new(&mut self.lssv.node_id).range(1..=127));
                if self.send_button(ui, ok, "Configure node-ID").clicked() {
                    self.lss_send(LssCommand::ConfigureNodeId { node_id: self.lssv.node_id });
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Bit rate");
                let cur = bit_timing_kbit(self.lssv.bt_index).map(|k| format!("{k} kbit/s")).unwrap_or_else(|| "auto".to_string());
                egui::ComboBox::from_id_salt("lss_bt").selected_text(cur).show_ui(ui, |ui| {
                    for i in 0..=8u8 {
                        if let Some(k) = bit_timing_kbit(i) {
                            ui.selectable_value(&mut self.lssv.bt_index, i, format!("{k} kbit/s"));
                        }
                    }
                });
                if self.send_button(ui, ok, "Configure bit timing").clicked() {
                    self.lss_send(LssCommand::ConfigureBitTiming { table: 0, index: self.lssv.bt_index });
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Switch delay");
                ui.add(egui::DragValue::new(&mut self.lssv.delay_ms).range(1..=10_000).suffix(" ms"));
                if self.send_button(ui, ok, "Activate bit timing").clicked() {
                    self.lss_send(LssCommand::ActivateBitTiming { delay_ms: self.lssv.delay_ms });
                }
                if self.send_button(ui, ok, "Store configuration").clicked() {
                    self.lss_send(LssCommand::StoreConfiguration);
                }
            });
            ui.separator();
            if let Some((_, l)) = &self.lssv.running {
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
            ui.strong("Results");
            if self.lssv.log.is_empty() {
                ui.label(egui::RichText::new("No LSS service run yet.").color(p.text_secondary));
            }
            for (l, t, e) in self.lssv.log.iter().rev().take(50) {
                let c = if *e { p.error } else { p.text_primary };
                ui.label(egui::RichText::new(format!("{l}: {t}")).color(c));
            }
        });
    }
}
