//! DCF view: save the node's values to a DCF file, compare a DCF with the
//! node, and restore a DCF (with a dry run first). Each one is an SDO task
//! that starts only when the user presses its button.

use super::theme::{self, Tone};
use super::{App, TaskOwner};
use crate::bus::BusCommand;
use crate::dcf::{DcfCommissioning, DcfCompareReport, DcfCompareTask, DcfRestoreReport, DcfRestoreTask, DcfSaveTask, ObjectAction, RestoreOptions};
use crate::eds::ObjectDictionary;
use crate::canopen::task::TaskResult;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Result class of a table row (the filter chips of spec 8.9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RowKind {
    Written,
    WouldWrite,
    Unchanged,
    Skipped,
    Failed,
    NotRun,
    Different,
}

impl RowKind {
    fn label(self) -> &'static str {
        match self {
            RowKind::Written => "Written",
            RowKind::WouldWrite => "Would write",
            RowKind::Unchanged => "Unchanged",
            RowKind::Skipped => "Skipped",
            RowKind::Failed => "Failed",
            RowKind::NotRun => "Not run",
            RowKind::Different => "Different",
        }
    }

    fn tone(self) -> Tone {
        match self {
            RowKind::Written => Tone::Success,
            RowKind::WouldWrite | RowKind::Different => Tone::Warning,
            RowKind::Unchanged | RowKind::NotRun => Tone::Neutral,
            RowKind::Skipped => Tone::Info,
            RowKind::Failed => Tone::Error,
        }
    }
}

/// One row of the result table (spec 8.9).
struct ResultRow<'a> {
    index: u16,
    sub: u8,
    name: &'a str,
    kind: RowKind,
    file: &'a str,
    device: &'a str,
    detail: &'a str,
}

fn restore_rows(rep: &DcfRestoreReport) -> Vec<ResultRow<'_>> {
    rep.objects
        .iter()
        .map(|o| {
            let (kind, detail) = match &o.action {
                ObjectAction::Written => (RowKind::Written, ""),
                ObjectAction::WouldWrite => (RowKind::WouldWrite, "dry run"),
                ObjectAction::Unchanged => (RowKind::Unchanged, ""),
                ObjectAction::Skipped { reason } => (RowKind::Skipped, reason.as_str()),
                ObjectAction::Failed { error, .. } => (RowKind::Failed, error.as_str()),
                ObjectAction::NotRun => (RowKind::NotRun, "not run after a stop"),
            };
            ResultRow { index: o.index, sub: o.sub, name: &o.name, kind, file: o.dcf_value.as_deref().unwrap_or(""), device: o.node_value.as_deref().unwrap_or(""), detail }
        })
        .collect()
}

fn compare_rows(rep: &DcfCompareReport) -> Vec<ResultRow<'_>> {
    rep.differences
        .iter()
        .map(|d| {
            let kind = if d.error.is_some() { RowKind::Failed } else { RowKind::Different };
            ResultRow { index: d.index, sub: d.sub, name: &d.name, kind, file: &d.dcf_value, device: d.node_value.as_deref().unwrap_or(""), detail: d.error.as_deref().unwrap_or("") }
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct DcfView {
    /// The DCF to compare or restore.
    pub file: Option<PathBuf>,
    pub dcf: Option<Arc<ObjectDictionary>>,
    pub load_error: Option<String>,
    pub use_default: bool,
    pub continue_on_error: bool,
    pub store: bool,
    pub node_name: String,
    pub compare: Option<DcfCompareReport>,
    pub restore: Option<DcfRestoreReport>,
    pub last: Option<String>,
    /// Result filter chip (None = All).
    pub filter: Option<RowKind>,
}

impl App {
    fn dcf_busy(&self) -> bool {
        self.tasks.values().any(|o| matches!(o, TaskOwner::DcfSave(_) | TaskOwner::DcfCompare | TaskOwner::DcfRestore))
    }

    /// Load the DCF file for compare / restore.
    pub(crate) fn dcf_load(&mut self, path: &Path) {
        match crate::eds::load(path) {
            Ok(od) => {
                self.dcfv.dcf = Some(Arc::new(od));
                self.dcfv.load_error = None;
            }
            Err(e) => {
                self.dcfv.dcf = None;
                self.dcfv.load_error = Some(format!("{}: {e}", path.display()));
            }
        }
        self.dcfv.file = Some(path.to_path_buf());
        self.dcfv.compare = None;
        self.dcfv.restore = None;
    }

    /// Read every readable object of the active node into a DCF at `path`.
    pub(crate) fn dcf_save_to(&mut self, path: PathBuf) -> Result<(), String> {
        let node = self.active_node();
        let dev = self.active_device().cloned().ok_or("Select a device with an EDS file first")?;
        let file = dev.eds_file.clone().ok_or("The device has no EDS file")?;
        let od = self.od_for(&file)?;
        let lib = self.lib.as_ref().ok_or("No library")?;
        let bytes = std::fs::read(lib.eds_path(&file)).map_err(|e| format!("{file}: {e}"))?;
        let eds_text = crate::eds::decode_text(&bytes);
        let comm = DcfCommissioning {
            node_id: node,
            baudrate_kbit: self.settings.bitrate_kbit,
            node_name: if self.dcfv.node_name.trim().is_empty() { dev.name.clone() } else { self.dcfv.node_name.trim().to_string() },
            net_number: 0,
            network_name: String::new(),
        };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "node.dcf".to_string());
        self.dcfv.last = Some(format!("Reading node {node} into {}...", path.display()));
        self.start_task(TaskOwner::DcfSave(path), Box::new(DcfSaveTask::new(node, od, eds_text, comm, name)));
        Ok(())
    }

    pub(crate) fn dcf_compare(&mut self) {
        let Some(dcf) = self.dcfv.dcf.clone() else { return };
        let node = self.active_node();
        self.dcfv.compare = None;
        self.dcfv.last = Some(format!("Comparing node {node}..."));
        self.start_task(TaskOwner::DcfCompare, Box::new(DcfCompareTask::new(node, dcf, self.dcfv.use_default)));
    }

    pub(crate) fn dcf_restore(&mut self, dry_run: bool) {
        let Some(dcf) = self.dcfv.dcf.clone() else { return };
        let node = self.active_node();
        let opts = RestoreOptions { dry_run, use_default_values: self.dcfv.use_default, from: None, to: None, continue_on_error: self.dcfv.continue_on_error, store: self.dcfv.store && !dry_run };
        let file = self.dcfv.file.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        let go = move |app: &mut App| {
            app.dcfv.restore = None;
            app.dcfv.last = Some(format!("{} node {node}...", if dry_run { "Dry run on" } else { "Restoring" }));
            app.start_task(TaskOwner::DcfRestore, Box::new(DcfRestoreTask::new(node, dcf, opts)));
        };
        if dry_run {
            go(self);
        } else {
            let ask = self.settings.confirm_dcf_restore;
            self.confirm_then(ask, "Restore DCF", format!("Write the values of {file} into node {node}. Objects that differ are written over SDO."), "Restore", true, go);
        }
    }

    pub(crate) fn on_dcf_task(&mut self, owner: TaskOwner, result: TaskResult) {
        match (owner, result) {
            (TaskOwner::DcfSave(path), TaskResult::DcfSave(rep)) => {
                if rep.cancelled {
                    self.dcfv.last = Some("DCF save cancelled; no file written".to_string());
                    self.set_status("DCF save cancelled", true);
                    return;
                }
                match crate::library::write_atomic(&path, rep.text.as_bytes()) {
                    Ok(()) => {
                        let t = format!("Saved {} value(s) to {}{}", rep.values, path.display(), if rep.failed.is_empty() { String::new() } else { format!(", {} object(s) not read", rep.failed.len()) });
                        self.set_status(t.clone(), !rep.failed.is_empty());
                        self.dcfv.last = Some(t);
                    }
                    Err(e) => {
                        self.set_status(format!("DCF not written: {}", e.0), true);
                        self.dcfv.last = Some(format!("DCF not written: {}", e.0));
                    }
                }
            }
            (TaskOwner::DcfCompare, TaskResult::DcfCompare(rep)) => {
                let t = format!("Compared {} object(s): {} difference(s){}", rep.compared, rep.differences.len(), if rep.cancelled { ", cancelled" } else { "" });
                self.set_status(t.clone(), false);
                self.dcfv.last = Some(t);
                self.dcfv.filter = None;
                self.dcfv.restore = None;
                self.dcfv.compare = Some(rep);
            }
            (TaskOwner::DcfRestore, TaskResult::DcfRestore(rep)) => {
                let w = rep.count(|a| matches!(a, ObjectAction::Written | ObjectAction::WouldWrite));
                let f = rep.count(|a| matches!(a, ObjectAction::Failed { .. }));
                let t = if rep.dry_run { format!("Dry run: {w} object(s) would be written, nothing sent as a write") } else { format!("Restore {}: {w} written, {f} failed{}", if rep.ok { "done" } else { "not complete" }, if rep.stored == Some(true) { ", stored" } else { "" }) };
                self.set_status(t.clone(), !rep.ok);
                self.dcfv.last = Some(t);
                // After a run with failures the table shows the failed rows.
                self.dcfv.filter = (f > 0).then_some(RowKind::Failed);
                self.dcfv.compare = None;
                self.dcfv.restore = Some(rep);
            }
            _ => {}
        }
    }

    pub(crate) fn dcf_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let node = self.active_node();
        let busy = self.dcf_busy();
        let has_eds = self.active_device().and_then(|d| d.eds_file.clone()).is_some();
        egui::ScrollArea::vertical().id_salt("dcf_scroll").show(ui, |ui| {
            ui.strong(format!("Save node {node} to a DCF"));
            theme::toolbar(ui, |ui| {
                ui.label("Node name");
                ui.add(egui::TextEdit::singleline(&mut self.dcfv.node_name).desired_width(140.0).hint_text("device name"));
                let r = self.send_button(ui, has_eds && !busy, format!("{} Save DCF...", egui_phosphor::regular::FLOPPY_DISK));
                if r.clicked() {
                    if let Some(path) = rfd::FileDialog::new().set_file_name(format!("node{node}.dcf")).add_filter("DCF", &["dcf"]).save_file() {
                        if let Err(e) = self.dcf_save_to(path) {
                            self.set_status(e, true);
                        }
                    }
                }
                if !has_eds {
                    ui.label(egui::RichText::new("Needs an active device with an EDS file.").color(p.text_secondary));
                }
            });
            ui.separator();
            ui.strong("Compare or restore a DCF");
            theme::toolbar(ui, |ui| {
                if ui.button("Choose DCF...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("DCF", &["dcf", "DCF"]).add_filter("All files", &["*"]).pick_file() {
                        self.dcf_load(&path);
                    }
                }
                match &self.dcfv.file {
                    Some(f) => ui.monospace(f.display().to_string()),
                    None => ui.label(egui::RichText::new("no file chosen").color(p.text_secondary)),
                };
            });
            if let Some(e) = &self.dcfv.load_error {
                ui.colored_label(p.error, e);
            }
            theme::toolbar(ui, |ui| {
                ui.checkbox(&mut self.dcfv.use_default, "Use default values").on_hover_text("Also objects that have only a DefaultValue");
                ui.checkbox(&mut self.dcfv.continue_on_error, "Continue on error");
                ui.checkbox(&mut self.dcfv.store, "Store (0x1010) at the end");
            });
            let have = self.dcfv.dcf.is_some();
            theme::toolbar(ui, |ui| {
                if self.send_button(ui, have && !busy, "Compare").on_hover_text("Read the node and list the differences").clicked() {
                    self.dcf_compare();
                }
                if self.send_button(ui, have && !busy, "Dry run").on_hover_text("Read the node and list what a restore would write").clicked() {
                    self.dcf_restore(true);
                }
                if self.styled_send_button(ui, have && !busy, false, "Restore", "Write the values of the DCF into the node (asks first)").clicked() {
                    self.dcf_restore(false);
                }
            });
            let task = self.bus.shared.lock().ok().and_then(|s| s.task.clone());
            if let Some((tag, label, done, total)) = task.filter(|t| self.tasks.get(&t.0).is_some_and(|o| matches!(o, TaskOwner::DcfSave(_) | TaskOwner::DcfCompare | TaskOwner::DcfRestore))) {
                ui.horizontal(|ui| {
                    let frac = total.map(|t| done as f32 / t.max(1) as f32).unwrap_or(0.0);
                    ui.add(egui::ProgressBar::new(frac).desired_width(220.0).text(format!("{label}: {done}{}", total.map(|t| format!(" / {t}")).unwrap_or_default())));
                    if ui.button("Cancel").clicked() {
                        self.bus.send(BusCommand::CancelTask(tag));
                    }
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            if let Some(t) = &self.dcfv.last {
                ui.label(egui::RichText::new(t).color(p.text_secondary));
            }
            let outlined = self.settings.reduce_colour;
            if let Some(rep) = &self.dcfv.compare {
                ui.separator();
                ui.strong(format!("Compare: {} object(s), {} difference(s)", rep.compared, rep.differences.len()));
                if rep.differences.is_empty() {
                    theme::pill(ui, "no difference", p.success, p.success_tint, outlined);
                } else {
                    let rows = compare_rows(rep);
                    let same = rep.compared.saturating_sub(rep.differences.len());
                    ui.label(egui::RichText::new(format!("{same} object(s) are the same (not listed)")).small().color(p.text_secondary));
                    result_view(ui, &mut self.dcfv.filter, &rows, &[RowKind::Different, RowKind::Failed], self.settings.zebra, outlined);
                }
            }
            if let Some(rep) = &self.dcfv.restore {
                ui.separator();
                ui.strong(if rep.dry_run { "Dry run" } else { "Restore" });
                let rows = restore_rows(rep);
                let kinds: &[RowKind] = if rep.dry_run {
                    &[RowKind::WouldWrite, RowKind::Unchanged, RowKind::Skipped, RowKind::Failed, RowKind::NotRun]
                } else {
                    &[RowKind::Written, RowKind::Unchanged, RowKind::Skipped, RowKind::Failed, RowKind::NotRun]
                };
                result_view(ui, &mut self.dcfv.filter, &rows, kinds, self.settings.zebra, outlined);
            }
        });
    }
}

/// Filter chips (`label count`, toggle style) and the result table of
/// spec 8.9: Index, Sub, Name, Result (pill), Value in file, Value on
/// device, Detail.
fn result_view(ui: &mut egui::Ui, filter: &mut Option<RowKind>, rows: &[ResultRow<'_>], kinds: &[RowKind], zebra: bool, outlined: bool) {
    let p = theme::pal(ui);
    theme::toolbar(ui, |ui| {
        if ui.selectable_label(filter.is_none(), format!("All {}", rows.len())).clicked() {
            *filter = None;
        }
        for k in kinds {
            let n = rows.iter().filter(|r| r.kind == *k).count();
            if n == 0 && *k == RowKind::NotRun {
                continue;
            }
            if ui.selectable_label(*filter == Some(*k), format!("{} {n}", k.label())).clicked() {
                *filter = if *filter == Some(*k) { None } else { Some(*k) };
            }
        }
    });
    let shown: Vec<&ResultRow<'_>> = rows.iter().filter(|r| filter.is_none_or(|f| r.kind == f)).collect();
    if shown.is_empty() {
        ui.label(egui::RichText::new("No row in this filter.").color(p.text_secondary));
        return;
    }
    let row_h = ui.spacing().interact_size.y;
    egui::ScrollArea::horizontal().id_salt("dcf_result_h").show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("dcf_result")
            .striped(zebra)
            .vscroll(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(64.0))
            .column(Column::exact(40.0))
            .column(Column::remainder().at_least(180.0).clip(true))
            .column(Column::exact(110.0))
            .column(Column::exact(130.0).clip(true))
            .column(Column::exact(130.0).clip(true))
            .column(Column::exact(220.0).clip(true))
            .header(row_h, |mut h| {
                for t in ["Index", "Sub", "Name", "Result", "Value in file", "Value on device", "Detail"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, shown.len(), |mut row| {
                    let r = shown[row.index()];
                    row.col(|ui| {
                        ui.monospace(format!("{:04X}", r.index));
                    });
                    row.col(|ui| {
                        ui.monospace(format!("{:02X}", r.sub));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(r.name).truncate()).on_hover_text(r.name);
                    });
                    row.col(|ui| {
                        theme::tone_pill(ui, r.kind.tone(), r.kind.label(), outlined);
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(r.file).monospace()).truncate());
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(r.device).monospace()).truncate());
                    });
                    row.col(|ui| {
                        let c = if r.kind == RowKind::Failed { p.error } else { p.text_secondary };
                        if !r.detail.is_empty() {
                            ui.add(egui::Label::new(egui::RichText::new(r.detail).color(c)).truncate()).on_hover_text(r.detail);
                        }
                    });
                });
            });
    });
}
