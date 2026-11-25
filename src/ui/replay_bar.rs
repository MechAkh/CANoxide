//! Log replay (0.3): the file picker of the connection bar, the transport
//! bar (play / pause, step, speed, scrubber, loop) shown only during a
//! replay, the load progress and the question about bad lines. The replay
//! itself runs in the bus worker (`crate::replay`); this module only draws
//! and sends commands. A replay never transmits.

use super::{theme, App};
use crate::bus::{BusCommand, ReplayInfo};
use crate::replay::{LineError, Speed, SPEEDS};
use eframe::egui;
use egui_phosphor::regular as icon;
use std::path::PathBuf;

/// A loaded log with bad lines, waiting for the user.
pub(crate) struct BadLines {
    pub path: PathBuf,
    pub frames: usize,
    pub bad: usize,
    pub errors: Vec<LineError>,
}

/// Seconds with three decimals (the transport bar clock).
pub(crate) fn time_text(us: u64) -> String {
    format!("{:.3} s", us as f64 / 1e6)
}

impl App {
    fn replay_info(&self) -> Option<ReplayInfo> {
        self.bus.shared.lock().ok().and_then(|s| s.replay.clone())
    }

    /// The channel part of the connection bar for the replay adapter: the
    /// file and a Browse button.
    pub(crate) fn replay_file_picker(&mut self, ui: &mut egui::Ui, locked: bool) {
        let p = theme::pal(ui);
        let name = if self.settings.replay_file.is_empty() {
            "no file chosen".to_string()
        } else {
            std::path::Path::new(&self.settings.replay_file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        };
        ui.add(egui::Label::new(egui::RichText::new(name).color(if self.settings.replay_file.is_empty() { p.text_secondary } else { p.text_primary })).truncate())
            .on_hover_text(if self.settings.replay_file.is_empty() { "Choose a log file" } else { self.settings.replay_file.as_str() });
        if ui.add_enabled(!locked, egui::Button::new(format!("{}  Browse...", icon::FOLDER_OPEN))).on_hover_text("A trace CSV written by CANoxide, a candump log, a PEAK .trc or a Vector .asc file").clicked() {
            let picked = rfd::FileDialog::new()
                .set_title("Log file to play back")
                .add_filter("Log files", &["csv", "log", "txt", "trc", "asc"])
                .add_filter("All files", &["*"])
                .pick_file();
            if let Some(f) = picked {
                self.settings.replay_file = f.display().to_string();
                self.mark_settings();
            }
        }
        let loading = self.bus.shared.lock().ok().and_then(|s| s.replay_loading.clone());
        if let Some((_, done, total)) = loading {
            let frac = if total == 0 { 0.0 } else { done as f32 / total as f32 };
            ui.add(egui::ProgressBar::new(frac).desired_width(110.0).text(format!("Loading {:.0} %", frac * 100.0)));
        }
    }

    /// The transport bar (only while a log is played back).
    pub(crate) fn transport_bar(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.replay_info() else { return };
        let p = theme::pal(ui);
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            theme::pill(ui, format!("{}  Replay", icon::FILM_STRIP), p.info, p.info_tint, self.settings.reduce_colour);
            let busy = r.seeking.is_some();
            let (glyph, name, tip) = if r.playing { (icon::PAUSE, "Pause", "Pause the replay") } else { (icon::PLAY, "Play", "Play the log from the cursor") };
            if theme::icon_button(ui, true, glyph, name, tip).clicked() {
                self.bus.send(if r.playing { BusCommand::ReplayPause } else { BusCommand::ReplayPlay });
            }
            if theme::icon_button(ui, !busy && r.cursor < r.frames, icon::SKIP_FORWARD, "Step", "One frame forward").clicked() {
                self.bus.send(BusCommand::ReplayStep);
            }
            ui.label(egui::RichText::new("Speed").small().color(p.text_secondary));
            let mut speed = r.speed;
            let ir = egui::ComboBox::from_id_salt("replay_speed").width(150.0).selected_text(speed.label()).show_ui(ui, |ui| {
                for s in SPEEDS {
                    ui.selectable_value(&mut speed, Speed::Factor(s), Speed::Factor(s).label());
                }
                ui.selectable_value(&mut speed, Speed::Max, Speed::Max.label());
            });
            ir.response.on_hover_text("Replay speed (log time per wall time)").widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Replay speed"));
            if speed != r.speed {
                self.bus.send(BusCommand::ReplaySpeed(speed));
            }
            let mut looping = r.looping;
            if ui.checkbox(&mut looping, "Loop").on_hover_text("Start again at the end of the log").changed() {
                self.bus.send(BusCommand::ReplayLoop(looping));
            }
            ui.add_space(8.0);
            let total_s = r.duration_us as f64 / 1e6;
            let shown = self.replay_drag.unwrap_or(r.pos_us as f64 / 1e6);
            ui.label(egui::RichText::new(format!("{} / {}", time_text((shown * 1e6) as u64), time_text(r.duration_us))).monospace());
            ui.label(egui::RichText::new(format!("frame {} / {}", r.cursor, r.frames)).small().color(p.text_secondary));
            if let Some((done, total)) = r.seeking {
                let frac = if total == 0 { 1.0 } else { done as f32 / total as f32 };
                ui.add(egui::ProgressBar::new(frac).desired_width(110.0).text(format!("Seeking {:.0} %", frac * 100.0)));
            }
            // The scrubber takes the rest of the bar.
            let w = (ui.available_width() - 8.0).max(120.0);
            ui.spacing_mut().slider_width = w;
            let mut v = shown;
            let resp = ui.add(egui::Slider::new(&mut v, 0.0..=total_s.max(0.001)).show_value(false)).on_hover_text("Drag or click to move the replay; the views are rebuilt up to this time");
            if resp.dragged() || resp.changed() {
                self.replay_drag = Some(v);
            }
            let released = resp.drag_stopped() || (resp.changed() && !resp.dragged());
            if released {
                if let Some(t) = self.replay_drag.take() {
                    self.replay_seek((t * 1e6).round() as u64);
                }
            }
        });
        if r.playing || r.seeking.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
        }
    }

    /// Move the replay to `pos_us` (log time).
    pub(crate) fn replay_seek(&mut self, pos_us: u64) {
        self.bus.send(BusCommand::ReplaySeek { pos_us, ctx: self.decode_ctx.clone() });
    }

    /// The question about a log with bad lines.
    pub(crate) fn replay_dialogs(&mut self, ctx: &egui::Context) {
        let Some(b) = self.replay_bad.as_ref() else { return };
        let p = if self.is_dark() { &theme::DARK } else { &theme::LIGHT };
        let mut answer: Option<bool> = None;
        egui::Modal::new(egui::Id::new("replay_bad_lines")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.heading("Log file with bad lines");
            let name = b.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.add(egui::Label::new(format!("{name}: {} line(s) could not be read; {} frame(s) are good.", b.bad, b.frames)).wrap());
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for e in b.errors.iter().take(50) {
                    ui.label(egui::RichText::new(e.to_string()).monospace().small().color(p.error));
                }
                if b.bad > 50 {
                    ui.label(egui::RichText::new(format!("... and {} more", b.bad - 50)).small());
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Load the good frames").color(p.on_accent)).fill(p.accent_fill)).clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(yes) = answer {
            self.replay_bad = None;
            self.bus.send(BusCommand::ReplayAccept(yes));
        }
    }
}
