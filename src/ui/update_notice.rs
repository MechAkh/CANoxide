//! The update notice (update.rs): the check in a worker thread, the
//! `Update x.y.z` pill of the menu bar and the `Update available` window.
//! Nothing is downloaded or run; the window opens the release page in the
//! browser.

use super::{theme, App};
use crate::update::{self, Release};
use eframe::egui;
use std::sync::mpsc;

/// The state of the update notice in the app.
pub(crate) struct UpdateState {
    /// Asks GitHub (`update::fetch_latest`; the tests inject their own).
    pub fetch: update::Fetcher,
    /// A running check: (started by Help > Check for updates..., result).
    pub pending: Option<(bool, mpsc::Receiver<Result<Release, String>>)>,
    /// A newer release than this version.
    pub known: Option<Release>,
    /// The pill is shown (a newer release that was not skipped, or one that
    /// a manual check found).
    pub show: bool,
    pub window: bool,
    /// The last URL given to the browser (the tests read it).
    pub opened: Option<String>,
}

impl UpdateState {
    pub(crate) fn new(fetch: update::Fetcher) -> UpdateState {
        UpdateState { fetch, pending: None, known: None, show: false, window: false, opened: None }
    }
}

impl App {
    /// At start: the automatic check, when it is on and due.
    pub(crate) fn start_update_check(&mut self, ctx: &egui::Context) {
        if self.settings.update_check && update::check_due(self.settings.update_last_check, update::now_utc()) {
            self.spawn_update_check(ctx, false);
        }
    }

    /// Help > Check for updates...: always, whatever the setting and the
    /// time of the last check.
    pub(crate) fn manual_update_check(&mut self, ctx: &egui::Context) {
        self.set_status("Checking for updates...", false);
        if let Some((manual, _)) = &mut self.upd.pending {
            // The automatic check is still running: its result is shown.
            *manual = true;
            return;
        }
        self.spawn_update_check(ctx, true);
    }

    fn spawn_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        let (tx, rx) = mpsc::channel();
        let fetch = self.upd.fetch.clone();
        let ctx = ctx.clone();
        let started = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let r = fetch();
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        match started {
            Ok(_) => self.upd.pending = Some((manual, rx)),
            Err(e) if manual => self.set_status(format!("Update check: no worker thread ({e})"), true),
            Err(_) => {}
        }
    }

    /// The result of a running check, if it has come.
    pub(crate) fn poll_update(&mut self) {
        let Some((manual, rx)) = &self.upd.pending else { return };
        let manual = *manual;
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Update check: the check ended without an answer".to_string()),
        };
        self.upd.pending = None;
        match r {
            Ok(rel) => {
                self.settings.update_last_check = Some(update::now_utc());
                self.mark_settings();
                if rel.is_update_for(crate::VERSION) {
                    let v = rel.version();
                    self.upd.show = manual || v != self.settings.update_skip_version;
                    if manual {
                        self.set_status(format!("CANoxide {v} is available (you have {})", crate::VERSION), false);
                        self.upd.window = true;
                    }
                    self.upd.known = Some(rel);
                } else {
                    self.upd.known = None;
                    self.upd.show = false;
                    self.upd.window = false;
                    if manual {
                        self.set_status(format!("CANoxide {} is the newest version.", crate::VERSION), false);
                    }
                }
            }
            Err(e) => {
                // A failed automatic check is silent.
                if manual {
                    self.set_status(e, true);
                }
            }
        }
    }

    /// The pill at the right end of the menu bar (in a right-to-left layout).
    pub(crate) fn update_pill(&mut self, ui: &mut egui::Ui) {
        let Some(rel) = self.upd.known.as_ref().filter(|_| self.upd.show) else { return };
        let p = theme::pal(ui);
        let v = rel.version();
        let b = egui::Button::new(egui::RichText::new(format!("Update {v}")).small().color(p.on_accent)).fill(p.accent_fill).corner_radius(egui::CornerRadius::same(10));
        if ui.add(b).on_hover_text(format!("CANoxide {v} is available (you have {})", crate::VERSION)).clicked() {
            self.upd.window = true;
        }
    }

    /// The `Update available` window.
    pub(crate) fn update_window(&mut self, ctx: &egui::Context) {
        if !self.upd.window {
            return;
        }
        let Some(rel) = self.upd.known.clone() else {
            self.upd.window = false;
            return;
        };
        let (mut open_page, mut skip, mut close) = (false, false, false);
        let mut open = true;
        egui::Window::new("Update available").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            let p = theme::pal(ui);
            ui.set_max_width(380.0);
            egui::Grid::new("update_grid").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                ui.label(egui::RichText::new("Available").color(p.text_secondary));
                ui.monospace(rel.version());
                ui.end_row();
                ui.label(egui::RichText::new("You have").color(p.text_secondary));
                ui.monospace(crate::VERSION);
                ui.end_row();
                ui.label(egui::RichText::new("Release").color(p.text_secondary));
                let name = rel.display_name();
                ui.label(if name.is_empty() { "-".to_string() } else { name });
                ui.end_row();
                ui.label(egui::RichText::new("Published").color(p.text_secondary));
                ui.label(rel.display_date());
                ui.end_row();
            });
            ui.add(egui::Label::new(egui::RichText::new("Nothing is downloaded: the release page opens in your browser.").small().color(p.text_secondary)).wrap());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Open the release page").color(p.on_accent)).fill(p.accent_fill)).on_hover_text(rel.page()).clicked() {
                    open_page = true;
                }
                if ui.button("Skip this version").on_hover_text("No notice until a newer version").clicked() {
                    skip = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        if open_page {
            let url = rel.page().to_string();
            ctx.open_url(egui::OpenUrl::new_tab(&url));
            self.upd.opened = Some(url);
        }
        if skip {
            self.settings.update_skip_version = rel.version();
            self.upd.show = false;
            self.mark_settings();
            self.set_status(format!("CANoxide {} skipped; the notice comes back for a newer version", rel.version()), false);
        }
        if skip || close || !open {
            self.upd.window = false;
        }
    }
}
