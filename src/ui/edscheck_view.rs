//! EDS check view: read every readable entry of the EDS from the node and
//! compare it with the EDS (exists, access, size, constant values). Read
//! only: the check sends SDO uploads, never a write.

use super::theme::{self, Tone};
use super::{App, TaskOwner};
use crate::bus::BusCommand;
use crate::canopen::task::TaskResult;
use crate::edscheck::{EdsCheckOptions, EdsCheckReport, EdsCheckTask, Severity};
use crate::eds::ObjectDictionary;
use crate::settings::AdapterKind;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::sync::Arc;

pub(crate) struct EdsCheckView {
    pub skip_domain: bool,
    /// The index range (hex text).
    pub from: String,
    pub to: String,
    pub stop_after: u32,
    pub show_ok: bool,
    pub report: Option<EdsCheckReport>,
    /// The built-in EDS of the simulator (parsed once).
    pub builtin: Option<Arc<ObjectDictionary>>,
}

impl Default for EdsCheckView {
    fn default() -> Self {
        let o = EdsCheckOptions::default();
        EdsCheckView { skip_domain: o.skip_domain, from: format!("{:04X}", o.from), to: format!("{:04X}", o.to), stop_after: o.stop_after_timeouts, show_ok: false, report: None, builtin: None }
    }
}

fn parse_index(s: &str) -> Option<u16> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u16::from_str_radix(t, 16).ok()
}

fn tone(s: Severity) -> Tone {
    match s {
        Severity::Pass => Tone::Success,
        Severity::Warning => Tone::Warning,
        Severity::Fail => Tone::Error,
        Severity::Skipped => Tone::Neutral,
    }
}

impl App {
    fn edscheck_busy(&self) -> bool {
        self.tasks.values().any(|o| matches!(o, TaskOwner::EdsCheck))
    }

    /// The EDS to check against: the one of the active device, else (on the
    /// simulator) the built-in EDS of the simulated node.
    fn edscheck_eds(&mut self) -> Result<(String, Arc<ObjectDictionary>), String> {
        if let Some(file) = self.active_device().and_then(|d| d.eds_file.clone()) {
            return self.od_for(&file).map(|od| (file, od));
        }
        if self.settings.adapter == AdapterKind::Simulator {
            let od = self.ecv.builtin.get_or_insert_with(|| Arc::new(crate::can::sim::builtin_dictionary())).clone();
            return Ok(("sim_builtin.eds".to_string(), od));
        }
        Err("No EDS: select a device with an EDS file".to_string())
    }

    pub(crate) fn start_eds_check(&mut self) {
        let (file, od) = match self.edscheck_eds() {
            Ok(x) => x,
            Err(e) => {
                self.set_status(e, true);
                return;
            }
        };
        let (Some(from), Some(to)) = (parse_index(&self.ecv.from), parse_index(&self.ecv.to)) else {
            self.set_status("The index range must be hex (0000..FFFF)", true);
            return;
        };
        let opts = EdsCheckOptions { skip_domain: self.ecv.skip_domain, from: from.min(to), to: from.max(to), stop_after_timeouts: self.ecv.stop_after };
        let node = self.active_node();
        self.ecv.report = None;
        self.start_task(TaskOwner::EdsCheck, Box::new(EdsCheckTask::new(node, od, file.clone(), opts)));
        self.set_status(format!("Checking node {node} against {file}"), false);
    }

    pub(crate) fn on_edscheck_task(&mut self, result: TaskResult) {
        if let TaskResult::EdsCheck(rep) = result {
            self.set_status(rep.summary_line(), !rep.passed());
            self.ecv.report = Some(rep);
        }
    }

    pub(crate) fn edscheck_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let node = self.active_node();
        let busy = self.edscheck_busy();
        let eds = self.edscheck_eds();
        egui::ScrollArea::vertical().id_salt("edscheck_scroll").show(ui, |ui| {
            match &eds {
                Ok((file, _)) => ui.strong(format!("Check node {node} against {file}")),
                Err(_) => ui.strong(format!("Check node {node} against its EDS")),
            };
            ui.label(egui::RichText::new("Reads each entry by SDO and compares it with the EDS. Nothing is written.").color(p.text_secondary));
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.ecv.skip_domain, "Skip DOMAIN entries");
                ui.separator();
                ui.label("Index from 0x");
                ui.add(egui::TextEdit::singleline(&mut self.ecv.from).desired_width(44.0));
                ui.label("to 0x");
                ui.add(egui::TextEdit::singleline(&mut self.ecv.to).desired_width(44.0));
                ui.separator();
                ui.label("Stop after");
                ui.add(egui::DragValue::new(&mut self.ecv.stop_after).range(0..=100));
                ui.label("time-outs in a row (0 = never)");
            });
            ui.horizontal_wrapped(|ui| {
                let r = self.send_button(ui, eds.is_ok() && !busy, format!("{} Start", egui_phosphor::regular::PLAY));
                let r = match (&eds, self.can_send()) {
                    (Err(e), true) => r.on_disabled_hover_text(e.as_str()),
                    _ => r,
                };
                if r.clicked() {
                    self.start_eds_check();
                }
                if !self.can_send() {
                    ui.label(egui::RichText::new(self.send_hint()).color(p.text_secondary));
                } else if let Err(e) = &eds {
                    ui.label(egui::RichText::new(e.as_str()).color(p.text_secondary));
                }
            });
            let task = self.bus.shared.lock().ok().and_then(|s| s.task.clone());
            if let Some((tag, label, done, total)) = task.filter(|t| self.tasks.get(&t.0).is_some_and(|o| matches!(o, TaskOwner::EdsCheck))) {
                ui.horizontal(|ui| {
                    let frac = total.map(|t| done as f32 / t.max(1) as f32).unwrap_or(0.0);
                    ui.add(egui::ProgressBar::new(frac).desired_width(220.0).text(format!("{label}: {done}{}", total.map(|t| format!(" / {t}")).unwrap_or_default())));
                    if ui.button("Cancel").clicked() {
                        self.bus.send(BusCommand::CancelTask(tag));
                    }
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            let Some(rep) = &self.ecv.report else {
                if !busy {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("No check run yet. Press Start.").color(p.text_secondary));
                }
                return;
            };
            ui.separator();
            let c = if rep.failed() > 0 { p.error } else if rep.warnings() > 0 { p.warning } else { p.success };
            ui.colored_label(c, rep.summary_line());
            if rep.skipped_domain > 0 {
                ui.label(egui::RichText::new(format!("{} DOMAIN entr{} not read", rep.skipped_domain, if rep.skipped_domain == 1 { "y" } else { "ies" })).small().color(p.text_secondary));
            }
            let mut save: Option<(String, &'static str, &'static str, String)> = None;
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.ecv.show_ok, "show ok entries");
                if ui.button("Export CSV...").clicked() {
                    save = Some((format!("node{}_edscheck.csv", rep.node), "CSV", "csv", rep.to_csv(self.ecv.show_ok)));
                }
                if ui.button("Export JUnit XML...").clicked() {
                    save = Some((format!("node{}_edscheck.xml", rep.node), "JUnit XML", "xml", rep.to_junit()));
                }
            });
            let show_ok = self.ecv.show_ok;
            let rows: Vec<&crate::edscheck::CheckEntry> = rep.entries.iter().filter(|e| show_ok || e.severity != Severity::Pass).collect();
            if rows.is_empty() {
                ui.label(egui::RichText::new("Every entry is ok.").color(p.text_secondary));
            } else {
                let row_h = ui.spacing().interact_size.y;
                let outlined = self.settings.reduce_colour;
                egui::ScrollArea::horizontal().id_salt("edscheck_h").show(ui, |ui| {
                    TableBuilder::new(ui)
                        .id_salt("edscheck_table")
                        .striped(self.settings.zebra)
                        .vscroll(false)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(Column::exact(72.0))
                        .column(Column::initial(220.0).at_least(140.0).clip(true))
                        .column(Column::exact(120.0))
                        .column(Column::remainder().at_least(220.0).clip(true))
                        .header(row_h, |mut h| {
                            for t in ["Index:sub", "Name", "Result", "Detail"] {
                                h.col(|ui| {
                                    ui.strong(t);
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, rows.len(), |mut row| {
                                let e = rows[row.index()];
                                row.col(|ui| {
                                    ui.monospace(e.key());
                                });
                                row.col(|ui| {
                                    ui.add(egui::Label::new(e.name.as_str()).truncate()).on_hover_text(e.name.as_str());
                                });
                                row.col(|ui| {
                                    theme::tone_pill(ui, tone(e.severity), e.kind.label(), outlined);
                                });
                                row.col(|ui| {
                                    if !e.detail.is_empty() {
                                        let c = if e.severity == Severity::Fail { p.error } else { p.text_secondary };
                                        ui.add(egui::Label::new(egui::RichText::new(e.detail.as_str()).color(c)).truncate()).on_hover_text(e.detail.as_str());
                                    }
                                });
                            });
                        });
                });
            }
            if let Some((name, filter, ext, text)) = save {
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter(filter, &[ext]).save_file() {
                    match crate::library::write_atomic(&path, text.as_bytes()) {
                        Ok(()) => self.set_status(format!("Saved {}", path.display()), false),
                        Err(e) => self.set_status(format!("Not written: {}", e.0), true),
                    }
                }
            }
        });
    }
}
