//! Drive (CiA 402) view: polls the status objects of one node, shows the
//! state machine and the status bits, and writes the controlword commands,
//! the mode of operation and the targets. The state machine itself is
//! `canopen::cia402`.
//!
//! "Enable operation" and the target writes sit in the group "Motion
//! (danger)", locked until the user ticks the check box; the tick is not
//! saved and is cleared on disconnect and on a change of the node.

use super::theme::{self, Tone};
use super::{App, TaskOwner};
use crate::canopen::cia402::{self, Command, DriveState, DriveStatus};
use crate::canopen::task::{ReadsTask, SdoStep, StepsTask, TaskResult};
use eframe::egui;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// The text of the safety check box of the motion group.
pub(crate) const DRIVE_CHECK: &str = "I have checked that the drive can move safely";

/// Abort codes that drop an object from the poll: object / sub-index does
/// not exist.
const NOT_THERE: [u32; 2] = [0x0602_0000, 0x0609_0011];

pub(crate) struct DriveView {
    /// The node of the view (follows the active node).
    pub node: Option<u8>,
    pub poll: bool,
    pub period_ms: u64,
    last_poll: Option<Instant>,
    pub poll_tag: Option<u64>,
    pub status: DriveStatus,
    /// Polled at least once since the node was chosen.
    pub have_status: bool,
    /// Objects that aborted with "does not exist" (shown as n/a).
    pub na: BTreeSet<u16>,
    pub mode_sel: i8,
    /// The safety check box of the motion group (not saved).
    pub checked: bool,
    pub target_velocity: i32,
    pub target_position: i32,
    pub target_torque: i16,
    /// Tag of a running command or write.
    pub cmd_tag: Option<u64>,
    /// The result of the last command or write: (text, error).
    pub note: Option<(String, bool)>,
}

impl Default for DriveView {
    fn default() -> Self {
        DriveView {
            node: None,
            poll: false,
            period_ms: 200,
            last_poll: None,
            poll_tag: None,
            status: DriveStatus::default(),
            have_status: false,
            na: BTreeSet::new(),
            mode_sel: 3,
            checked: false,
            target_velocity: 0,
            target_position: 0,
            target_torque: 0,
            cmd_tag: None,
            note: None,
        }
    }
}

impl DriveView {
    /// Forget the node state: a new node, or the bus went away.
    pub(crate) fn reset(&mut self) {
        self.checked = false;
        self.status = DriveStatus::default();
        self.have_status = false;
        self.na.clear();
        self.note = None;
    }

    pub(crate) fn on_disconnected(&mut self) {
        self.poll = false;
        self.checked = false;
        self.poll_tag = None;
        self.cmd_tag = None;
    }
}

fn opt<T: std::fmt::Display>(na: bool, v: Option<T>) -> String {
    match (na, v) {
        (true, _) => "n/a".to_string(),
        (false, Some(v)) => v.to_string(),
        (false, None) => "-".to_string(),
    }
}

impl App {
    /// Called each frame: follows the active node and runs the poll.
    pub(crate) fn drive_tick(&mut self) {
        let node = self.active_node();
        if self.drv.node != Some(node) {
            self.drv.node = Some(node);
            self.drv.reset();
        }
        if !self.drv.poll {
            return;
        }
        if !self.can_send() || !self.is_open(super::dock::Tab::Drive) {
            self.drv.poll = false;
            return;
        }
        let due = self.drv.last_poll.is_none_or(|t| t.elapsed() >= Duration::from_millis(self.drv.period_ms.max(20)));
        if due && self.drv.poll_tag.is_none() {
            let items: Vec<(u16, u8)> = cia402::POLL_OBJECTS.iter().filter(|i| !self.drv.na.contains(i)).map(|i| (*i, 0)).collect();
            if items.is_empty() {
                return;
            }
            self.drv.last_poll = Some(Instant::now());
            let tag = self.start_task(TaskOwner::DrivePoll(node), Box::new(ReadsTask::new("drive status", node, items)));
            self.drv.poll_tag = Some(tag);
        }
    }

    pub(crate) fn on_drive_task(&mut self, owner: TaskOwner, result: TaskResult) {
        match (owner, result) {
            (TaskOwner::DrivePoll(node), TaskResult::Reads(r)) => {
                self.drv.poll_tag = None;
                if self.drv.node != Some(node) || r.cancelled {
                    return;
                }
                for x in &r.reads {
                    if x.abort_code.is_some_and(|c| NOT_THERE.contains(&c)) {
                        self.drv.na.insert(x.index);
                    }
                }
                let s = DriveStatus::from_reads(&r);
                if s.statusword.is_none() {
                    if let Some(e) = r.reads.iter().find(|x| x.index == cia402::STATUSWORD).and_then(|x| x.error.clone()) {
                        self.drv.note = Some((format!("Statusword not read: {e}"), true));
                    }
                }
                self.drv.status = s;
                self.drv.have_status = true;
            }
            (TaskOwner::DriveCommand(what), TaskResult::Steps(r)) => {
                self.drv.cmd_tag = None;
                let node = r.done.first().map(|s| s.step.node).unwrap_or(0);
                let t = if r.ok {
                    format!("{what} sent to node {node}")
                } else if r.cancelled {
                    format!("{what} cancelled")
                } else {
                    format!("{what} on node {node} failed: {}", r.first_error().and_then(|s| s.error.clone()).unwrap_or_default())
                };
                self.set_status(t.clone(), !r.ok);
                self.drv.note = Some((t, !r.ok));
                // Show the new state at once.
                self.drv.last_poll = None;
            }
            _ => {}
        }
    }

    fn drive_write(&mut self, what: String, steps: Vec<SdoStep>) {
        self.drv.note = None;
        let tag = self.start_task(TaskOwner::DriveCommand(what.clone()), Box::new(StepsTask::new(what, steps, true)));
        self.drv.cmd_tag = Some(tag);
    }

    pub(crate) fn drive_command(&mut self, node: u8, c: Command) {
        let steps = c.controlwords().iter().map(|w| SdoStep::write(node, cia402::CONTROLWORD, 0, w.to_le_bytes().to_vec(), format!("controlword 0x{w:04X} ({})", c.name()))).collect();
        self.drive_write(c.name().to_string(), steps);
    }

    pub(crate) fn drive_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let node = self.active_node();
        let can = self.can_send();
        let busy = self.drv.cmd_tag.is_some();
        let outlined = self.settings.reduce_colour;
        egui::ScrollArea::vertical().id_salt("drive_scroll").show(ui, |ui| {
            if !can {
                ui.colored_label(p.warning, if self.listen_only_active { "Listen only: every command is disabled." } else { "Not connected: the commands are disabled." });
            }
            theme::toolbar(ui, |ui| {
                ui.label(egui::RichText::new(format!("Node {node}")).strong());
                ui.separator();
                let r = ui.add_enabled(can, egui::Checkbox::new(&mut self.drv.poll, "Poll"));
                let r = if can { r.on_hover_text("Read 6041, 6061, 6064, 606C, 6077 and 603F by SDO each period") } else { r.on_disabled_hover_text(self.send_hint()) };
                if r.changed() && self.drv.poll {
                    self.drv.last_poll = None;
                }
                ui.add(egui::DragValue::new(&mut self.drv.period_ms).range(50..=10_000).suffix(" ms")).on_hover_text("Poll period");
                if self.drv.poll {
                    theme::pill(ui, "polling", p.info, p.info_tint, outlined);
                }
            });
            ui.add_space(4.0);

            // State machine.
            let st = self.drv.status.state;
            let state_text = match (self.drv.have_status, st, self.drv.status.statusword) {
                (false, _, _) => "State: unknown (not polled)".to_string(),
                (true, Some(s), _) => format!("State: {}", s.name()),
                (true, None, Some(sw)) => format!("State: no valid state (statusword 0x{sw:04X})"),
                (true, None, None) => "State: unknown".to_string(),
            };
            ui.label(egui::RichText::new(state_text).strong());
            theme::toolbar(ui, |ui| {
                for s in DriveState::ALL {
                    if Some(s) == st {
                        let tone = match s {
                            DriveState::Fault | DriveState::FaultReactionActive => Tone::Error,
                            DriveState::QuickStopActive => Tone::Warning,
                            DriveState::OperationEnabled => Tone::Info,
                            _ => Tone::Success,
                        };
                        theme::tone_pill(ui, tone, s.name(), outlined);
                    } else {
                        theme::pill(ui, s.name(), p.text_disabled, egui::Color32::TRANSPARENT, false);
                    }
                }
            });
            ui.add_space(4.0);
            let s = self.drv.status.clone();
            let na = |i: u16| self.drv.na.contains(&i);
            egui::Grid::new("drive_values").num_columns(4).show(ui, |ui| {
                ui.label("Statusword");
                ui.monospace(if na(cia402::STATUSWORD) { "n/a".to_string() } else { s.statusword.map(|v| format!("0x{v:04X}")).unwrap_or_else(|| "-".into()) });
                ui.label("Mode (6061)");
                ui.monospace(if na(cia402::MODES_DISPLAY) { "n/a".to_string() } else { s.mode_name.clone().unwrap_or_else(|| "-".into()) });
                ui.end_row();
                ui.label("Position (6064)");
                ui.monospace(opt(na(cia402::POSITION_ACTUAL), s.position));
                ui.label("Velocity (606C)");
                ui.monospace(opt(na(cia402::VELOCITY_ACTUAL), s.velocity));
                ui.end_row();
                ui.label("Torque (6077)");
                ui.monospace(opt(na(cia402::TORQUE_ACTUAL), s.torque));
                ui.label("Error code (603F)");
                ui.monospace(if na(cia402::ERROR_CODE) { "n/a".to_string() } else { s.error_code.map(|v| format!("0x{v:04X}")).unwrap_or_else(|| "-".into()) });
                ui.end_row();
            });
            if let Some(b) = s.bits {
                theme::toolbar(ui, |ui| {
                    for (name, on) in b.list() {
                        let g = if on { egui_phosphor::regular::CHECK_SQUARE } else { egui_phosphor::regular::SQUARE };
                        ui.label(egui::RichText::new(format!("{g} {name}")).color(if on { p.text_primary } else { p.text_secondary }));
                    }
                });
            }

            // Commands.
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Commands").strong());
            let allowed = |c: Command| st.is_some_and(|s| s.allows(c));
            theme::toolbar(ui, |ui| {
                let big = egui::vec2(150.0, 34.0);
                for c in [Command::QuickStop, Command::DisableVoltage] {
                    let b = egui::Button::new(egui::RichText::new(format!("{}  {}", egui_phosphor::regular::HAND_PALM, c.name())).size(16.0).color(egui::Color32::WHITE)).fill(p.danger_fill).min_size(big);
                    let r = ui.add_enabled(can, b);
                    let r = if can { r.on_hover_text(format!("Write the controlword {} (always allowed while connected)", c.controlwords().iter().map(|w| format!("0x{w:04X}")).collect::<Vec<_>>().join(", "))) } else { r.on_disabled_hover_text(self.send_hint()) };
                    if r.clicked() {
                        self.drive_command(node, c);
                    }
                }
            });
            theme::toolbar(ui, |ui| {
                for c in [Command::Shutdown, Command::SwitchOn, Command::DisableOperation, Command::FaultReset] {
                    let r = self.send_button(ui, !busy && allowed(c), c.name());
                    let r = if can && !allowed(c) { r.on_disabled_hover_text("Not allowed in this state (poll first)") } else { r };
                    if r.clicked() {
                        self.drive_command(node, c);
                    }
                }
            });
            theme::toolbar(ui, |ui| {
                ui.label("Mode of operation (6060)");
                theme::toolbar_combo(ui, "drive_mode", None, &cia402::mode_text(self.drv.mode_sel), Some(240.0), |ui| {
                    for m in cia402::MODES {
                        ui.selectable_value(&mut self.drv.mode_sel, m, cia402::mode_text(m));
                    }
                });
                // A mode change while the drive is enabled can start a motion.
                let mode_ok = st != Some(DriveState::OperationEnabled) || self.drv.checked;
                let r = self.send_button(ui, !busy && mode_ok, "Write mode");
                let r = if can && !mode_ok { r.on_disabled_hover_text("Operation enabled: tick the check box of the motion group first") } else { r };
                if r.clicked() {
                    let m = self.drv.mode_sel;
                    self.drive_write(format!("Mode {}", cia402::mode_text(m)), vec![SdoStep::write(node, cia402::MODES_OF_OPERATION, 0, vec![m as u8], "mode of operation")]);
                }
            });

            // Motion (danger).
            ui.add_space(8.0);
            egui::Frame::new().stroke(egui::Stroke::new(1.0_f32, p.danger_fill)).corner_radius(4).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.label(egui::RichText::new(format!("{}  Motion (danger)", egui_phosphor::regular::WARNING)).strong().color(p.error));
                ui.label(egui::RichText::new("These commands can make the drive move.").small().color(p.text_secondary));
                ui.add_enabled(can, egui::Checkbox::new(&mut self.drv.checked, DRIVE_CHECK));
                let unlocked = can && self.drv.checked;
                ui.add_enabled_ui(unlocked, |ui| {
                    theme::toolbar(ui, |ui| {
                        let c = Command::EnableOperation;
                        if self.styled_send_button(ui, unlocked && !busy && allowed(c), true, c.name(), "Write the controlword 0x000F").clicked() {
                            self.drive_command(node, c);
                        }
                    });
                    egui::Grid::new("drive_targets").num_columns(3).show(ui, |ui| {
                        ui.label("Target velocity (60FF)");
                        ui.add(egui::DragValue::new(&mut self.drv.target_velocity));
                        if self.styled_send_button(ui, unlocked && !busy, true, "Write 0x60FF", "Write the target velocity").clicked() {
                            let v = self.drv.target_velocity;
                            self.drive_write(format!("Target velocity {v}"), vec![SdoStep::write(node, cia402::TARGET_VELOCITY, 0, v.to_le_bytes().to_vec(), "target velocity")]);
                        }
                        ui.end_row();
                        ui.label("Target position (607A)");
                        ui.add(egui::DragValue::new(&mut self.drv.target_position));
                        if self.styled_send_button(ui, unlocked && !busy, true, "Write 0x607A", "Write the target position").clicked() {
                            let v = self.drv.target_position;
                            self.drive_write(format!("Target position {v}"), vec![SdoStep::write(node, cia402::TARGET_POSITION, 0, v.to_le_bytes().to_vec(), "target position")]);
                        }
                        ui.end_row();
                        ui.label("Target torque (6071)");
                        ui.add(egui::DragValue::new(&mut self.drv.target_torque));
                        if self.styled_send_button(ui, unlocked && !busy, true, "Write 0x6071", "Write the target torque").clicked() {
                            let v = self.drv.target_torque;
                            self.drive_write(format!("Target torque {v}"), vec![SdoStep::write(node, cia402::TARGET_TORQUE, 0, v.to_le_bytes().to_vec(), "target torque")]);
                        }
                        ui.end_row();
                    });
                });
            });
            if let Some((t, err)) = &self.drv.note {
                ui.colored_label(if *err { p.error } else { p.text_secondary }, t.as_str());
            }
        });
        if self.drv.poll {
            ui.ctx().request_repaint_after(Duration::from_millis(self.drv.period_ms.clamp(50, 500)));
        }
    }
}
