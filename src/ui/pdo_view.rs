//! PDO configuration view: read the PDOs of a node over SDO, edit the
//! communication and mapping parameters, apply the change, enable or
//! disable a PDO, and save / load the configuration in the library.
//! A read or a write runs only when the user presses its button.

use super::{theme, App, TaskOwner};
use crate::bus::BusCommand;
use crate::canopen::pdo::{parse_map_entry, plan_apply, plan_enable, plan_enable_all, MapEntry, NodePdoConfig, PdoConfig, PdoKind, PdoReadOptions, PdoReadTask};
use crate::canopen::task::{StepsReport, StepsTask, TaskResult};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::BTreeMap;

/// The PDO in the editor, with its text fields.
pub(crate) struct Editor {
    pub node: u8,
    pub old: PdoConfig,
    pub new: PdoConfig,
    pub cob_text: String,
    pub add_text: String,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct PdoView {
    /// Read (or loaded) configurations by node.
    pub configs: BTreeMap<u8, NodePdoConfig>,
    pub editor: Option<Editor>,
    /// (node, the configurations after the change) of a running apply.
    applying: Option<(u8, Vec<PdoConfig>)>,
    /// Report of the last apply with failures (shown in a dialog).
    report: Option<StepsReport>,
    pub last: Option<String>,
}

impl PdoView {
    pub(crate) fn read_configs(&self) -> Vec<(u8, Vec<PdoConfig>)> {
        self.configs.values().map(|c| (c.node, c.pdos.clone())).collect()
    }

    pub(crate) fn on_config_read(&mut self, c: NodePdoConfig) {
        if let Some(e) = &self.editor {
            if e.node == c.node {
                self.editor = None;
            }
        }
        self.configs.insert(c.node, c);
    }

    /// An edit that is not applied.
    pub(crate) fn has_pending(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.old != e.new)
    }

    fn open_editor(&mut self, node: u8, cfg: &PdoConfig) {
        self.editor = Some(Editor { node, old: cfg.clone(), new: cfg.clone(), cob_text: format!("{:X}", cfg.cob_id), add_text: String::new(), error: None });
    }
}

fn mapping_text(m: &[MapEntry]) -> String {
    if m.is_empty() {
        return "(none)".to_string();
    }
    m.iter().map(|e| format!("{:04X}:{:02X}/{}", e.index, e.sub, e.bits)).collect::<Vec<_>>().join(", ")
}

fn trans_text(t: u8) -> String {
    match t {
        0 => "0 acyclic sync".to_string(),
        1..=240 => format!("{t} sync"),
        252 => "252 RTR sync".to_string(),
        253 => "253 RTR event".to_string(),
        254 => "254 event (mfr)".to_string(),
        255 => "255 event (profile)".to_string(),
        _ => format!("{t} reserved"),
    }
}

impl App {
    fn pdo_busy(&self) -> bool {
        self.tasks.values().any(|o| matches!(o, TaskOwner::PdoRead | TaskOwner::PdoApply))
    }

    /// Read every PDO of `node` (a task).
    pub(crate) fn read_pdos(&mut self, node: u8) {
        let opts = PdoReadOptions { stop_after_missing: self.settings.pdo_scan_stop.max(1), ..PdoReadOptions::default() };
        self.pdo.last = Some(format!("Reading the PDOs of node {node}..."));
        self.start_task(TaskOwner::PdoRead, Box::new(PdoReadTask::new(node, opts)));
    }

    /// Run planned PDO writes; `after` is the configuration when they work.
    fn run_pdo_steps(&mut self, node: u8, label: String, steps: Vec<crate::canopen::task::SdoStep>, after: Vec<PdoConfig>) {
        if steps.is_empty() {
            self.set_status("No change to apply", false);
            return;
        }
        let n = steps.len();
        let ask = self.settings.confirm_pdo_apply;
        self.confirm_then(ask, "Apply PDO change", format!("{label} on node {node}: {n} SDO write(s). The node may stop or change its PDOs."), "Apply", false, move |app| {
            app.pdo.applying = Some((node, after));
            app.pdo.last = Some(format!("{label}: writing..."));
            app.start_task(TaskOwner::PdoApply, Box::new(StepsTask::new(label, steps, true)));
        });
    }

    fn pdo_set_valid(&mut self, node: u8, cfg: &PdoConfig, enable: bool) {
        let Some(all) = self.pdo.configs.get(&node).map(|c| c.pdos.clone()) else { return };
        let steps = plan_enable(node, cfg, enable);
        let after: Vec<PdoConfig> = all
            .into_iter()
            .map(|mut c| {
                if c.kind == cfg.kind && c.number == cfg.number {
                    c.valid = enable;
                }
                c
            })
            .collect();
        self.run_pdo_steps(node, format!("{} {}", if enable { "Enable" } else { "Disable" }, cfg.label()), steps, after);
    }

    fn pdo_set_all(&mut self, node: u8, kind: PdoKind, enable: bool) {
        let Some(all) = self.pdo.configs.get(&node).map(|c| c.pdos.clone()) else { return };
        let steps = plan_enable_all(node, &all, Some(kind), enable);
        let after: Vec<PdoConfig> = all
            .into_iter()
            .map(|mut c| {
                if c.kind == kind && !c.mapping.is_empty() {
                    c.valid = enable;
                }
                c
            })
            .collect();
        self.run_pdo_steps(node, format!("{} every {}", if enable { "Enable" } else { "Disable" }, kind.label()), steps, after);
    }

    fn pdo_apply_editor(&mut self) {
        let Some(e) = &self.pdo.editor else { return };
        let (node, old, new) = (e.node, e.old.clone(), e.new.clone());
        let Some(all) = self.pdo.configs.get(&node).map(|c| c.pdos.clone()) else { return };
        let steps = plan_apply(node, &old, &new);
        let after: Vec<PdoConfig> = all.into_iter().map(|c| if c.kind == new.kind && c.number == new.number { new.clone() } else { c }).collect();
        self.run_pdo_steps(node, format!("Apply {}", new.label()), steps, after);
    }

    pub(crate) fn on_pdo_task(&mut self, owner: TaskOwner, result: TaskResult) {
        match (owner, result) {
            (TaskOwner::PdoRead, TaskResult::PdoConfig(c)) => {
                let text = format!("Node {}: {} PDO(s) read{}{}", c.node, c.pdos.len(), if c.complete { "" } else { ", incomplete" }, if c.errors.is_empty() { String::new() } else { format!(", {} error(s)", c.errors.len()) });
                self.set_status(text.clone(), !c.errors.is_empty() || !c.complete);
                self.pdo.last = Some(text);
                self.pdo.configs.insert(c.node, c);
                self.rebuild_decode();
            }
            (TaskOwner::PdoApply, TaskResult::Steps(rep)) => {
                let applying = self.pdo.applying.take();
                let failed = rep.done.iter().filter(|r| !r.ok).count();
                let text = if rep.ok {
                    format!("PDO change applied: {} write(s)", rep.done.len())
                } else if rep.cancelled {
                    format!("PDO change cancelled after {} write(s)", rep.done.len())
                } else {
                    format!("PDO change failed: {failed} write(s) failed, {} not run", rep.not_run.len())
                };
                self.set_status(text.clone(), !rep.ok);
                self.pdo.last = Some(text);
                if rep.ok {
                    if let Some((node, after)) = applying {
                        let cfg = match self.pdo.configs.get(&node) {
                            Some(c) => NodePdoConfig { pdos: after, ..c.clone() },
                            None => return,
                        };
                        self.pdo.configs.insert(node, cfg.clone());
                        self.pdo.editor = None;
                        self.decode_ctx.set_node_pdos(node, cfg.pdos.clone());
                        self.bus.send(BusCommand::SetNodePdos(cfg));
                    }
                } else {
                    self.pdo.report = Some(rep);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn pdo_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let node = self.active_node();
        let busy = self.pdo_busy();
        let device = self.active_device().map(|d| d.name.clone());
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Node {node}"));
            if self.send_button(ui, !busy, format!("{} Read PDOs", egui_phosphor::regular::DOWNLOAD_SIMPLE)).on_hover_text("Read 0x1400..0x1BFF over SDO").clicked() {
                self.read_pdos(node);
            }
            let have = self.pdo.configs.contains_key(&node);
            if ui.add_enabled(have && device.is_some(), egui::Button::new("Save to library")).on_hover_text("Save the read configuration with the device").clicked() {
                if let (Some(lib), Some(dev), Some(c)) = (&self.lib, &device, self.pdo.configs.get(&node)) {
                    match lib.save_pdo_config(dev, c) {
                        Ok(()) => self.set_status(format!("PDO configuration of {dev} saved"), false),
                        Err(e) => self.set_status(format!("PDO configuration not saved: {e}"), true),
                    }
                }
            }
            if ui.add_enabled(device.is_some(), egui::Button::new("Load from library")).on_hover_text("Use the saved configuration for decoding (sends nothing)").clicked() {
                if let (Some(lib), Some(dev)) = (&self.lib, &device) {
                    match lib.load_pdo_config(dev) {
                        Ok(Some(mut c)) => {
                            c.node = node;
                            self.pdo.configs.insert(node, c.clone());
                            self.decode_ctx.set_node_pdos(node, c.pdos.clone());
                            self.bus.send(BusCommand::SetNodePdos(c));
                            self.set_status(format!("PDO configuration of {dev} loaded"), false);
                        }
                        Ok(None) => self.set_status(format!("No saved PDO configuration for {dev}"), true),
                        Err(e) => self.set_status(format!("PDO configuration not read: {e}"), true),
                    }
                }
            }
            ui.separator();
            for kind in [PdoKind::Tpdo, PdoKind::Rpdo] {
                if self.send_button(ui, have && !busy, format!("Disable all {}", kind.label())).clicked() {
                    self.pdo_set_all(node, kind, false);
                }
                if self.send_button(ui, have && !busy, format!("Enable all {}", kind.label())).clicked() {
                    self.pdo_set_all(node, kind, true);
                }
            }
        });
        // Progress of a running PDO task.
        let task = self.bus.shared.lock().ok().and_then(|s| s.task.clone());
        if let Some((tag, label, done, total)) = task.filter(|t| self.tasks.get(&t.0).is_some_and(|o| matches!(o, TaskOwner::PdoRead | TaskOwner::PdoApply))) {
            ui.horizontal(|ui| {
                let frac = total.map(|t| done as f32 / t.max(1) as f32).unwrap_or(0.0);
                ui.add(egui::ProgressBar::new(frac).desired_width(200.0).text(format!("{label}: {done}{}", total.map(|t| format!(" / {t}")).unwrap_or_default())));
                if ui.button("Cancel").clicked() {
                    self.bus.send(BusCommand::CancelTask(tag));
                }
            });
        }
        if let Some(t) = &self.pdo.last {
            ui.label(egui::RichText::new(t).color(p.text_secondary));
        }
        ui.separator();
        let Some(cfg) = self.pdo.configs.get(&node).cloned() else {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::ARROWS_LEFT_RIGHT).size(28.0).color(p.text_secondary));
                ui.label(format!("No PDO configuration of node {node}."));
                ui.label(egui::RichText::new("Press Read PDOs (connected), or Load from library.").color(p.text_secondary));
            });
            return;
        };
        ui.label(egui::RichText::new(format!("Read {}{}", cfg.read_time_text(), if cfg.complete { "" } else { ", incomplete" })).color(p.text_secondary));
        for e in &cfg.errors {
            ui.colored_label(p.warning, e);
        }
        let row_h = self.settings.row_density.row_height().max(24.0);
        let can_send = self.can_send();
        let mut edit = None;
        let mut toggle = None;
        let editing = self.pdo.editor.as_ref().map(|e| (e.node, e.new.kind, e.new.number));
        let avail = ui.available_height();
        let table_h = if self.pdo.editor.is_some() { (avail * 0.45).max(120.0) } else { avail };
        TableBuilder::new(ui)
            .id_salt("pdo_table")
            .striped(self.settings.zebra)
            .resizable(true)
            .max_scroll_height(table_h)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(64.0))
            .column(Column::initial(70.0))
            .column(Column::initial(76.0))
            .column(Column::initial(120.0))
            .column(Column::initial(70.0))
            .column(Column::initial(70.0))
            .column(Column::initial(260.0).clip(true))
            .column(Column::remainder().at_least(130.0))
            .header(row_h, |mut h| {
                for t in ["PDO", "COB-ID", "State", "Transmission", "Inhibit", "Event", "Mapping", ""] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for c in &cfg.pdos {
                    body.row(row_h, |mut row| {
                        row.col(|ui| {
                            let sel = editing == Some((node, c.kind, c.number));
                            ui.label(egui::RichText::new(c.label()).strong().color(if sel { p.text_accent } else { p.text_primary }));
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{}{:X}", if c.extended { "x" } else { "" }, c.cob_id));
                        });
                        row.col(|ui| {
                            if c.valid {
                                theme::pill(ui, "enabled", p.success, p.success_tint, self.settings.reduce_colour);
                            } else {
                                theme::pill(ui, "disabled", p.text_secondary, p.bg_button, self.settings.reduce_colour);
                            }
                        });
                        row.col(|ui| {
                            ui.label(trans_text(c.trans_type));
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{:.1} ms", c.inhibit_100us as f32 / 10.0));
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{} ms", c.event_timer_ms));
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(format!("{} bit: {}", c.total_bits(), mapping_text(&c.mapping))).monospace()).truncate());
                        });
                        row.col(|ui| {
                            if ui.button("Edit").clicked() {
                                edit = Some(c.clone());
                            }
                            let (t, on) = if c.valid { ("Disable", false) } else { ("Enable", true) };
                            let r = ui.add_enabled(can_send && !busy, egui::Button::new(t));
                            if r.clicked() {
                                toggle = Some((c.clone(), on));
                            }
                        });
                    });
                }
            });
        if let Some(c) = edit {
            self.pdo.open_editor(node, &c);
        }
        if let Some((c, on)) = toggle {
            self.pdo_set_valid(node, &c, on);
        }
        self.pdo_editor(ui, busy);
    }

    fn pdo_editor(&mut self, ui: &mut egui::Ui, busy: bool) {
        let p = theme::pal(ui);
        let can_send = self.can_send();
        let hint = self.send_hint();
        let Some(e) = self.pdo.editor.as_mut() else { return };
        ui.separator();
        let mut apply = false;
        let mut close = false;
        egui::ScrollArea::vertical().id_salt("pdo_editor").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(format!("{} of node {}", e.new.label(), e.node));
                if e.old != e.new {
                    theme::pill(ui, "pending", p.warning, p.warning_tint, false);
                }
            });
            egui::Grid::new("pdo_edit_grid").num_columns(2).show(ui, |ui| {
                ui.label("COB-ID 0x");
                if ui.add(egui::TextEdit::singleline(&mut e.cob_text).desired_width(90.0)).changed() {
                    match u32::from_str_radix(e.cob_text.trim().trim_start_matches("0x"), 16) {
                        Ok(v) if v <= if e.new.extended { 0x1FFF_FFFF } else { 0x7FF } => {
                            e.new.cob_id = v;
                            e.error = None;
                        }
                        _ => e.error = Some("COB-ID: hex, at most 7FF (or 1FFFFFFF extended)".to_string()),
                    }
                }
                ui.end_row();
                ui.label("Enabled");
                ui.checkbox(&mut e.new.valid, "valid (bit 31 clear)");
                ui.end_row();
                ui.label("Transmission type");
                ui.add(egui::DragValue::new(&mut e.new.trans_type).range(0..=255));
                ui.end_row();
                ui.label("Inhibit time");
                ui.add(egui::DragValue::new(&mut e.new.inhibit_100us).range(0..=65535).suffix(" x 100 us"));
                ui.end_row();
                ui.label("Event timer");
                ui.add(egui::DragValue::new(&mut e.new.event_timer_ms).range(0..=65535).suffix(" ms"));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.strong(format!("Mapping ({} bit of 64)", e.new.total_bits()));
            let mut remove = None;
            let mut up = None;
            let n = e.new.mapping.len();
            for (i, m) in e.new.mapping.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.monospace(format!("{:>2}  {:04X}:{:02X}  {:>2} bit", i + 1, m.index, m.sub, m.bits));
                    if ui.add_enabled(i > 0, egui::Button::new(egui_phosphor::regular::ARROW_UP).small()).on_hover_text("Move up").clicked() {
                        up = Some(i);
                    }
                    if ui.add_enabled(i + 1 < n, egui::Button::new(egui_phosphor::regular::ARROW_DOWN).small()).on_hover_text("Move down").clicked() {
                        up = Some(i + 1);
                    }
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = up {
                e.new.mapping.swap(i - 1, i);
            }
            if let Some(i) = remove {
                e.new.mapping.remove(i);
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut e.add_text).desired_width(120.0).hint_text("6000:1:8"));
                if ui.button("Add entry").clicked() {
                    match parse_map_entry(&e.add_text) {
                        Some(m) if e.new.total_bits() + m.bits as u32 <= 64 && m.bits > 0 => {
                            e.new.mapping.push(m);
                            e.add_text.clear();
                            e.error = None;
                        }
                        Some(_) => e.error = Some("the PDO holds at most 64 bits".to_string()),
                        None => e.error = Some("entry: index:sub:bits in hex, for example 6000:1:8".to_string()),
                    }
                }
            });
            if let Some(err) = &e.error {
                ui.colored_label(p.error, err);
            }
            ui.horizontal(|ui| {
                let changed = e.old != e.new;
                let r = ui.add_enabled(can_send && changed && !busy && e.error.is_none(), egui::Button::new("Apply"));
                if r.clicked() {
                    apply = true;
                }
                if !can_send {
                    r.on_disabled_hover_text(hint);
                }
                if ui.add_enabled(changed, egui::Button::new("Revert")).clicked() {
                    e.new = e.old.clone();
                    e.cob_text = format!("{:X}", e.old.cob_id);
                    e.error = None;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.pdo.editor = None;
        }
        if apply {
            self.pdo_apply_editor();
        }
    }

    pub(crate) fn pdo_dialogs(&mut self, ctx: &egui::Context) {
        let Some(rep) = &self.pdo.report else { return };
        let mut close = false;
        let err_c = if self.is_dark() { theme::DARK.error } else { theme::LIGHT.error };
        egui::Modal::new(egui::Id::new("pdo_report")).show(ctx, |ui| {
            ui.heading("PDO change not complete");
            for r in &rep.done {
                let t = format!("{:04X}:{:02X} {}: {}", r.step.index, r.step.sub, r.step.what, if r.ok { "ok".to_string() } else { r.error.clone().unwrap_or_default() });
                if r.ok {
                    ui.label(t);
                } else {
                    ui.colored_label(err_c, t);
                }
            }
            for s in &rep.not_run {
                ui.label(format!("{:04X}:{:02X} {}: not run", s.index, s.sub, s.what));
            }
            ui.label("Read the PDOs again to see the state of the node.");
            if ui.button("Close").clicked() {
                close = true;
            }
        });
        if close {
            self.pdo.report = None;
        }
    }
}
