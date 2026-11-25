//! Program download wizard (CiA 302-3): node, program number and file
//! (size, SHA-256), the steps to run, a warning, the danger button behind a
//! question, then the live step list with progress and Cancel, then the
//! result. The sequence itself is `progdl::ProgDlTask`.

use super::theme::{self, Tone};
use super::{App, TaskOwner};
use crate::bus::BusCommand;
use crate::canopen::sdo::TransferMode;
use crate::canopen::task::TaskResult;
use crate::progdl::{sha256_hex, ProgDlOptions, ProgDlReport, ProgDlTask, ProgStep, StepState};
use eframe::egui;
use std::sync::{Arc, Mutex};

/// The warning of the wizard (also in the question before the start).
pub(crate) const PROGDL_WARNING: &str = "The node stops its program for the download. A wrong file can leave the device without a working program. Never download to a machine in operation.";

/// The loaded file.
pub(crate) struct ProgFile {
    pub path: String,
    pub data: Arc<Vec<u8>>,
    pub sha256: String,
}

pub(crate) struct ProgDlView {
    pub open: bool,
    pub opts: ProgDlOptions,
    /// The path in the text field.
    pub path: String,
    /// The file of `path` (or why it is not read).
    pub file: Option<Result<ProgFile, String>>,
    /// The running download: its tag and its live report.
    pub tag: Option<u64>,
    pub live: Option<Arc<Mutex<ProgDlReport>>>,
    /// The report of the last download.
    pub result: Option<ProgDlReport>,
}

impl Default for ProgDlView {
    fn default() -> Self {
        ProgDlView { open: false, opts: ProgDlOptions::new(1), path: String::new(), file: None, tag: None, live: None, result: None }
    }
}

fn read_file(path: &str) -> Result<ProgFile, String> {
    let p = path.trim();
    if p.is_empty() {
        return Err("no file chosen".to_string());
    }
    let data = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
    if data.is_empty() {
        return Err(format!("{p}: the file is empty"));
    }
    Ok(ProgFile { path: p.to_string(), sha256: sha256_hex(&data), data: Arc::new(data) })
}

fn state_tone(s: StepState) -> Tone {
    match s {
        StepState::Ok => Tone::Success,
        StepState::Running => Tone::Info,
        StepState::Failed => Tone::Error,
        StepState::Cancelled => Tone::Warning,
        StepState::Pending | StepState::Skipped | StepState::NotRun => Tone::Neutral,
    }
}

impl App {
    /// Open the wizard for the active node (File menu, Network view).
    pub(crate) fn open_program_download(&mut self) {
        self.pdl.open = true;
        if self.pdl.tag.is_none() {
            self.pdl.opts.node = self.active_node();
            self.pdl.opts.mode = self.settings.sdo_transfer_mode;
        }
    }

    pub(crate) fn progdl_running(&self) -> bool {
        self.pdl.tag.is_some()
    }

    fn progdl_file_check(&mut self) {
        let stale = match &self.pdl.file {
            Some(Ok(f)) => f.path != self.pdl.path.trim(),
            Some(Err(_)) | None => true,
        };
        if stale && !self.pdl.path.trim().is_empty() {
            let r = read_file(&self.pdl.path);
            // Keep the error of a path that is still typed, without reading
            // it again each frame.
            let same = matches!(&self.pdl.file, Some(Err(e)) if e.starts_with(self.pdl.path.trim()));
            if r.is_ok() || !same {
                self.pdl.file = Some(r);
            }
        } else if self.pdl.path.trim().is_empty() {
            self.pdl.file = None;
        }
    }

    fn start_program_download(&mut self) {
        let Some(Ok(f)) = &self.pdl.file else { return };
        let data = f.data.as_ref().clone();
        let opts = self.pdl.opts.clone();
        let node = opts.node;
        let size = data.len();
        let mut task = ProgDlTask::new(opts, data);
        self.pdl.live = Some(task.shared());
        self.pdl.result = None;
        let tag = self.start_task(TaskOwner::ProgramDownload, Box::new(task));
        self.pdl.tag = Some(tag);
        self.set_status(format!("Program download to node {node} started ({size} bytes)"), false);
    }

    fn ask_program_download(&mut self) {
        let Some(Ok(f)) = &self.pdl.file else { return };
        let o = &self.pdl.opts;
        let plan = o.plan(f.data.len()).join("; ");
        let text = format!("Download {} ({} bytes) as program {} to node {}?\n\nSteps: {plan}.\n\n{PROGDL_WARNING}", f.path, f.data.len(), o.program, o.node);
        // Always asks, as Restore defaults does.
        self.confirm_then(true, "Download program", text, "Download", true, |app| app.start_program_download());
    }

    pub(crate) fn on_progdl_task(&mut self, result: TaskResult) {
        self.pdl.tag = None;
        self.pdl.live = None;
        if let TaskResult::ProgramDownload(r) = result {
            let t = if r.ok {
                format!("Program download to node {} done", r.node)
            } else if r.cancelled {
                format!("Program download to node {} cancelled", r.node)
            } else {
                let f = r.failed().map(|s| format!("{}: {}", s.step.name(), s.detail.clone().unwrap_or_default())).unwrap_or_default();
                format!("Program download to node {} failed at {f}", r.node)
            };
            self.set_status(t, !r.ok);
            self.pdl.result = Some(r);
        }
    }

    pub(crate) fn progdl_dialog(&mut self, ctx: &egui::Context) {
        if !self.pdl.open {
            return;
        }
        self.progdl_file_check();
        let mut open = true;
        let running = self.progdl_running();
        let report: Option<ProgDlReport> = match &self.pdl.live {
            Some(l) => l.lock().ok().map(|g| g.clone()),
            None => self.pdl.result.clone(),
        };
        egui::Window::new("Program download")
            .id(egui::Id::new("progdl"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .constrain(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.set_max_width(520.0);
                let p = theme::pal(ui);
                ui.add_enabled_ui(!running, |ui| {
                    egui::Grid::new("progdl_grid").num_columns(2).show(ui, |ui| {
                        ui.label("Node");
                        ui.add(egui::DragValue::new(&mut self.pdl.opts.node).range(1..=127));
                        ui.end_row();
                        ui.label("Program");
                        ui.add(egui::DragValue::new(&mut self.pdl.opts.program).range(1..=254)).on_hover_text("The sub-index of 0x1F50, 0x1F51, 0x1F56 and 0x1F57");
                        ui.end_row();
                        ui.label("File");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.pdl.path).desired_width(300.0).hint_text("program file"));
                            if ui.button("Browse...").clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("All files", &["*"]).pick_file() {
                                    self.pdl.path = path.display().to_string();
                                }
                            }
                        });
                        ui.end_row();
                        match &self.pdl.file {
                            Some(Ok(f)) => {
                                ui.label("Size");
                                ui.monospace(format!("{} bytes", f.data.len()));
                                ui.end_row();
                                ui.label("SHA-256");
                                ui.add(egui::Label::new(egui::RichText::new(&f.sha256).monospace().small()).wrap());
                                ui.end_row();
                            }
                            Some(Err(e)) => {
                                ui.label("");
                                ui.colored_label(p.error, e);
                                ui.end_row();
                            }
                            None => {}
                        }
                        ui.label("Transfer");
                        egui::ComboBox::from_id_salt("progdl_mode").selected_text(self.pdl.opts.mode.label()).show_ui(ui, |ui| {
                            for m in TransferMode::ALL {
                                ui.selectable_value(&mut self.pdl.opts.mode, m, m.label());
                            }
                        });
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Steps").strong());
                    for s in ProgStep::ALL {
                        let mut on = match s {
                            ProgStep::WaitBoot => self.pdl.opts.wait_boot,
                            _ => self.pdl.opts.enabled(s),
                        };
                        let enabled = s != ProgStep::WaitBoot || self.pdl.opts.start;
                        if ui.add_enabled(enabled, egui::Checkbox::new(&mut on, s.name())).changed() {
                            self.pdl.opts.set(s, on);
                        }
                    }
                });
                ui.add_space(4.0);
                theme::caution_note(ui, PROGDL_WARNING);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let ok = !running && matches!(self.pdl.file, Some(Ok(_)));
                    if self.styled_send_button(ui, ok, true, "Download program", "Run the enabled steps (asks first)").clicked() {
                        self.ask_program_download();
                    }
                    if let Some(t) = self.pdl.tag {
                        if ui.button("Cancel").clicked() {
                            self.bus.send(BusCommand::CancelTask(t));
                        }
                    }
                    if !running && ui.button("Close").clicked() {
                        self.pdl.open = false;
                    }
                });
                if let Some(r) = &report {
                    ui.separator();
                    let frac = if r.size == 0 { 0.0 } else { r.bytes_done as f32 / r.size as f32 };
                    ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).text(format!("{} / {} bytes", r.bytes_done, r.size)));
                    egui::Grid::new("progdl_steps").num_columns(3).show(ui, |ui| {
                        for s in &r.steps {
                            ui.label(s.step.name());
                            theme::tone_pill(ui, state_tone(s.state), s.state.name(), self.settings.reduce_colour);
                            ui.label(egui::RichText::new(s.detail.clone().unwrap_or_default()).small().color(p.text_secondary));
                            ui.end_row();
                        }
                    });
                    if r.finished {
                        let (tone, t) = if r.ok {
                            (Tone::Success, "Download done".to_string())
                        } else if r.cancelled {
                            (Tone::Warning, "Download cancelled".to_string())
                        } else {
                            (Tone::Error, format!("Download failed: {}", r.failed().map(|s| s.step.name()).unwrap_or("?")))
                        };
                        theme::tone_pill(ui, tone, t, self.settings.reduce_colour);
                        if let (Some(a), Some(b)) = (r.old_id, r.new_id) {
                            ui.label(format!("Program identification 0x{a:08X} -> 0x{b:08X} (CRC-32 of the file 0x{:08X})", r.crc32));
                        } else if let Some(b) = r.new_id {
                            ui.label(format!("Program identification 0x{b:08X} (CRC-32 of the file 0x{:08X})", r.crc32));
                        }
                    }
                }
            });
        if !open && !running {
            self.pdl.open = false;
        }
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
