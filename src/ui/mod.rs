//! egui user interface: connect bar, library panel, tabs and status line.
//!
//! The UI never blocks on the bus: it sends `BusCommand`s and reads
//! `BusEvent`s each frame. Every bus send comes from a button.

mod network;
mod od;
mod trace_view;
#[cfg(test)]
mod ui_tests;

use crate::bus::{AdapterChoice, BusCommand, BusEvent, BusHandle};
use crate::can::{pcan, Bitrate, ChannelInfo};
use crate::canopen::decode::{decode, DecodeContext};
use crate::canopen::sdo::SdoRequest;
use crate::eds::{self, ObjectDictionary};
use crate::library::{self, Device, Library};
use crate::settings::{AdapterKind, Settings};
use crate::trace::TraceStore;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Start the GUI. Blocks until the window closes.
pub fn run() -> eframe::Result {
    let boot = Boot::load();
    let size = [boot.settings.window_width, boot.settings.window_height];
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CANopenTester")
            .with_inner_size(size)
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("CANopenTester", options, Box::new(move |cc| Ok(Box::new(App::new(cc, boot)))))
}

/// What is loaded before the window opens.
struct Boot {
    data_dir: Option<PathBuf>,
    data_mode: Option<library::DataDirMode>,
    data_error: Option<String>,
    settings: Settings,
    library: Option<Library>,
    notes: Vec<String>,
}

impl Boot {
    fn load() -> Boot {
        match library::default_data_dir() {
            Ok(info) => {
                let mut b = Boot::load_dir(info.path.clone());
                b.notes.extend(info.note.clone());
                b.data_mode = Some(info.mode);
                b
            }
            Err(e) => Boot {
                data_mode: None,
                data_dir: None,
                data_error: Some(format!("No data folder: {e}. Nothing is saved.")),
                settings: Settings::default(),
                library: None,
                notes: Vec::new(),
            },
        }
    }

    /// Load settings and library from a given data folder (the UI tests
    /// use a temporary folder; the app uses `CANopenTester_data`).
    fn load_dir(dir: PathBuf) -> Boot {
        let mut b = Boot { data_dir: None, data_mode: None, data_error: None, settings: Settings::default(), library: None, notes: Vec::new() };
        b.data_dir = Some(dir.clone());
        if let Err(e) = library::ensure_writable(&dir) {
            b.data_error = Some(format!("The data folder {} is not writable: {e}. Nothing is saved.", dir.display()));
            return b;
        }
        let (settings, warn) = Settings::load(&dir);
        b.settings = settings;
        b.notes.extend(warn);
        match Library::open(&dir) {
            Ok(l) => {
                b.notes.extend(l.load_warning.clone());
                b.library = Some(l);
            }
            Err(e) => b.data_error = Some(format!("Library not loaded: {e}")),
        }
        b
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Od,
    Network,
    Trace,
}

enum Dialog {
    /// Add (`idx` None) or edit a device.
    Device { idx: Option<usize>, dev: Device, error: Option<String> },
    RemoveEds(String),
    RemoveDevice(usize),
}

pub(crate) struct App {
    data_dir: Option<PathBuf>,
    data_mode: Option<library::DataDirMode>,
    data_error: Option<String>,
    settings: Settings,
    settings_dirty: bool,
    settings_saved: Instant,
    lib: Option<Library>,
    bus: BusHandle,
    connected: bool,
    listen_only_active: bool,
    connect_text: String,
    channels: Vec<ChannelInfo>,
    channel_note: Option<String>,
    /// Parsed EDS files by file name in `eds\`.
    eds_cache: HashMap<String, Result<Arc<ObjectDictionary>, String>>,
    selected_eds: Option<String>,
    /// Node ID used when no device is active.
    free_node: u8,
    decode_ctx: DecodeContext,
    trace: TraceStore,
    tv: trace_view::TraceView,
    od: od::OdView,
    net: network::NetView,
    tab: Tab,
    status: String,
    status_error: bool,
    dialog: Option<Dialog>,
    next_tag: u64,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, boot: Boot) -> App {
        cc.egui_ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::System);
        let ctx = cc.egui_ctx.clone();
        let bus = BusHandle::spawn(Arc::new(move || ctx.request_repaint()));
        let mut app = App {
            data_dir: boot.data_dir,
            data_mode: boot.data_mode,
            data_error: boot.data_error,
            trace: TraceStore::new(boot.settings.trace_capacity),
            settings: boot.settings,
            settings_dirty: false,
            settings_saved: Instant::now(),
            lib: boot.library,
            bus,
            connected: false,
            listen_only_active: false,
            connect_text: String::new(),
            channels: Vec::new(),
            channel_note: None,
            eds_cache: HashMap::new(),
            selected_eds: None,
            free_node: 1,
            decode_ctx: DecodeContext::default(),
            tv: trace_view::TraceView::default(),
            od: od::OdView::default(),
            net: network::NetView::default(),
            tab: Tab::Od,
            status: "Ready. Nothing is sent until you press a button.".to_string(),
            status_error: false,
            dialog: None,
            next_tag: 1,
        };
        if !boot.notes.is_empty() {
            app.set_status(boot.notes.join("; "), true);
        }
        app.bus.send(BusCommand::ListChannels(app.settings.adapter));
        app.rebuild_decode();
        app
    }

    pub(crate) fn set_status(&mut self, s: impl Into<String>, error: bool) {
        self.status = s.into();
        self.status_error = error;
    }

    fn mark_settings(&mut self) {
        self.settings_dirty = true;
    }

    fn save_settings(&mut self) {
        self.settings_dirty = false;
        self.settings_saved = Instant::now();
        if self.data_error.is_some() {
            return;
        }
        if let Some(dir) = &self.data_dir {
            if let Err(e) = self.settings.save(dir) {
                let msg = format!("Settings not saved: {e}");
                self.set_status(msg, true);
            }
        }
    }

    /// True when a send control may be used.
    pub(crate) fn can_send(&self) -> bool {
        self.connected && !self.listen_only_active
    }

    pub(crate) fn tag(&mut self) -> u64 {
        self.next_tag += 1;
        self.next_tag
    }

    pub(crate) fn active_idx(&self) -> Option<usize> {
        let name = self.settings.active_device.as_ref()?;
        self.lib.as_ref()?.find_device(name)
    }

    pub(crate) fn active_device(&self) -> Option<&Device> {
        let i = self.active_idx()?;
        self.lib.as_ref()?.devices.get(i)
    }

    pub(crate) fn active_node(&self) -> u8 {
        self.active_device().map(|d| d.node_id).unwrap_or(self.free_node)
    }

    /// Parsed EDS by file name (cached).
    pub(crate) fn od_for(&mut self, file: &str) -> Result<Arc<ObjectDictionary>, String> {
        if let Some(r) = self.eds_cache.get(file) {
            return r.clone();
        }
        let r = match &self.lib {
            Some(lib) => eds::load(&lib.eds_path(file)).map(Arc::new).map_err(|e| format!("{file}: {e}")),
            None => Err("no library".to_string()),
        };
        self.eds_cache.insert(file.to_string(), r.clone());
        r
    }

    pub(crate) fn active_od(&mut self) -> Option<Result<Arc<ObjectDictionary>, String>> {
        let file = self.active_device()?.eds_file.clone()?;
        Some(self.od_for(&file))
    }

    /// Node -> dictionary map for the trace decoder, from the library.
    fn rebuild_decode(&mut self) {
        let pairs: Vec<(u8, String)> = match &self.lib {
            Some(l) => l.devices.iter().filter_map(|d| Some((d.node_id, d.eds_file.clone()?))).collect(),
            None => Vec::new(),
        };
        let mut map = HashMap::new();
        for (node, file) in pairs {
            if map.contains_key(&node) {
                continue;
            }
            if let Ok(od) = self.od_for(&file) {
                map.insert(node, od);
            }
        }
        self.decode_ctx = DecodeContext::new(map);
    }

    pub(crate) fn send_sdo(&mut self, node: u8, req: SdoRequest, pending: od::Pending) {
        let tag = self.tag();
        self.od.pending.insert(tag, pending);
        self.bus.send(BusCommand::Sdo { tag, node, req });
    }

    fn connect(&mut self) {
        let bitrate = Bitrate::from_kbit(self.settings.bitrate_kbit).unwrap_or(Bitrate::K250);
        let (adapter, channel) = match self.settings.adapter {
            AdapterKind::Peak => (AdapterChoice::Peak, self.settings.channel),
            AdapterKind::Simulator => {
                let node_id = self.active_node();
                let od = match self.active_od() {
                    Some(Ok(od)) => Some(od),
                    _ => None,
                };
                (AdapterChoice::Sim { node_id, od }, 0)
            }
        };
        self.bus.send(BusCommand::SetSdoTimeout(self.settings.sdo_timeout_ms));
        self.bus.send(BusCommand::Connect { adapter, channel, bitrate, listen_only: self.settings.listen_only });
        self.set_status("Connecting...", false);
    }

    fn poll_bus(&mut self) {
        while let Some(e) = self.bus.try_recv() {
            match e {
                BusEvent::Channels { kind, result } => {
                    if kind == self.settings.adapter {
                        match result {
                            Ok(v) => {
                                self.channel_note = if v.is_empty() && kind == AdapterKind::Peak { Some("no available channel".to_string()) } else { None };
                                self.channels = v;
                            }
                            Err(e) => {
                                self.channels.clear();
                                self.channel_note = Some(e);
                            }
                        }
                    }
                }
                BusEvent::Connected { text, listen_only } => {
                    self.connected = true;
                    self.listen_only_active = listen_only;
                    self.set_status(format!("Connected: {text}"), false);
                    self.connect_text = text;
                }
                BusEvent::Disconnected => {
                    self.connected = false;
                    self.listen_only_active = false;
                    self.net.cyclic_active.clear();
                    self.net.scanning = false;
                    self.set_status("Disconnected", false);
                }
                BusEvent::Error(s) => self.set_status(s, true),
                BusEvent::Info(s) => self.set_status(s, false),
                BusEvent::Frames(v) => {
                    if self.tv.paused {
                        self.tv.skipped += v.len() as u64;
                    } else {
                        for f in v {
                            let d = decode(&f, &self.decode_ctx);
                            self.trace.push(f, d);
                        }
                    }
                }
                BusEvent::SdoDone { tag, outcome } => self.on_sdo_done(tag, outcome),
                BusEvent::ScanFound { node, result } => {
                    self.net.scan_results.retain(|(n, _)| *n != node);
                    self.net.scan_results.push((node, result));
                }
                BusEvent::ScanFinished { cancelled, found } => {
                    self.net.scanning = false;
                    let what = if cancelled { "Scan cancelled" } else { "Scan finished" };
                    self.set_status(format!("{what}: {found} node(s) answered"), false);
                }
                // v0.2 events: the v0.2 UI handles them.
                _ => {}
            }
        }
    }

    fn add_eds_files(&mut self, paths: Vec<PathBuf>) {
        let Some(lib) = self.lib.as_mut() else {
            self.set_status("The library is not available (data folder error)", true);
            return;
        };
        let mut added = Vec::new();
        let mut errors = Vec::new();
        for p in paths {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext != "eds" && ext != "dcf" {
                errors.push(format!("{}: not an .eds or .dcf file", p.display()));
                continue;
            }
            match lib.add_eds(&p) {
                Ok(name) => added.push(name),
                Err(e) => errors.push(e.to_string()),
            }
        }
        for n in &added {
            self.eds_cache.remove(n);
        }
        if let Some(last) = added.last() {
            self.selected_eds = Some(last.clone());
        }
        if errors.is_empty() {
            self.set_status(format!("Added EDS: {}", added.join(", ")), false);
        } else {
            self.set_status(errors.join("; "), true);
        }
        self.rebuild_decode();
    }

    // ---------------------------------------------------------------- panels

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let locked = self.connected;
            ui.label("Adapter");
            let before = self.settings.adapter;
            ui.add_enabled_ui(!locked, |ui| {
                egui::ComboBox::from_id_salt("adapter").selected_text(self.settings.adapter.label()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.adapter, AdapterKind::Peak, AdapterKind::Peak.label());
                    ui.selectable_value(&mut self.settings.adapter, AdapterKind::Simulator, AdapterKind::Simulator.label());
                });
            });
            if self.settings.adapter != before {
                self.channels.clear();
                self.channel_note = None;
                self.bus.send(BusCommand::ListChannels(self.settings.adapter));
                self.mark_settings();
            }

            if self.settings.adapter == AdapterKind::Peak {
                ui.label("Channel");
                let ch = self.settings.channel;
                ui.add_enabled_ui(!locked, |ui| {
                    egui::ComboBox::from_id_salt("channel").selected_text(pcan::channel_name(ch as u16)).show_ui(ui, |ui| {
                        if self.channels.is_empty() {
                            ui.label("No available channel reported");
                        }
                        for c in &self.channels {
                            ui.selectable_value(&mut self.settings.channel, c.handle, format!("{} (available)", c.name));
                        }
                        ui.separator();
                        ui.label("Manual choice");
                        for h in pcan::PCAN_USB_CHANNELS {
                            ui.selectable_value(&mut self.settings.channel, h as u32, pcan::channel_name(h));
                        }
                    });
                    if ui.small_button("Refresh").on_hover_text("Ask the driver which channels are available (sends nothing)").clicked() {
                        self.bus.send(BusCommand::ListChannels(AdapterKind::Peak));
                    }
                });
                if self.settings.channel != ch {
                    self.mark_settings();
                }
                if let Some(n) = &self.channel_note {
                    ui.colored_label(ui.visuals().warn_fg_color, n);
                }
            } else {
                ui.label("Channel: virtual bus");
            }

            ui.label("Bitrate");
            let br = self.settings.bitrate_kbit;
            ui.add_enabled_ui(!locked, |ui| {
                let cur = Bitrate::from_kbit(br).map(|b| b.label()).unwrap_or_else(|| format!("{br} kbit/s"));
                egui::ComboBox::from_id_salt("bitrate").selected_text(cur).show_ui(ui, |ui| {
                    for b in Bitrate::ALL {
                        ui.selectable_value(&mut self.settings.bitrate_kbit, b.kbit(), b.label());
                    }
                });
                if ui
                    .checkbox(&mut self.settings.listen_only, "Listen only")
                    .on_hover_text("Open the channel in listen-only mode; every send control is disabled")
                    .changed()
                {
                    self.settings_dirty = true;
                }
            });
            if self.settings.bitrate_kbit != br {
                self.mark_settings();
            }

            if self.connected {
                if ui.button("Disconnect").clicked() {
                    self.bus.send(BusCommand::Disconnect);
                }
            } else if ui.button("Connect").on_hover_text("Opens the channel only; nothing is sent").clicked() {
                self.connect();
            }

            ui.separator();
            let (status, fps) = {
                let s = self.bus.shared.lock().unwrap_or_else(|p| p.into_inner());
                (s.status.clone(), s.fps)
            };
            if self.connected {
                ui.label(format!("Bus: {status}"));
                ui.label(format!("{fps:.0} frames/s"));
                if self.listen_only_active {
                    ui.colored_label(ui.visuals().warn_fg_color, "LISTEN ONLY");
                }
            } else {
                ui.label("Not connected");
            }
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = &self.data_error {
            ui.colored_label(ui.visuals().error_fg_color, e);
            ui.separator();
        }
        egui::ScrollArea::vertical().id_salt("left").show(ui, |ui| {
            self.devices_section(ui);
            ui.add_space(8.0);
            self.eds_section(ui);
        });
    }

    fn devices_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Devices");
        let have_lib = self.lib.is_some();
        let names: Vec<(String, u8, Option<String>)> = match &self.lib {
            Some(l) => l.devices.iter().map(|d| (d.name.clone(), d.node_id, d.eds_file.clone())).collect(),
            None => Vec::new(),
        };
        let active = self.active_idx();
        if names.is_empty() {
            ui.label("No devices yet.");
        }
        for (i, (name, node, eds)) in names.iter().enumerate() {
            let text = format!("{name}  (node {node}){}", if eds.is_none() { ", no EDS" } else { "" });
            if ui.selectable_label(active == Some(i), text).on_hover_text(eds.clone().unwrap_or_default()).clicked() {
                self.settings.active_device = if active == Some(i) { None } else { Some(name.clone()) };
                self.mark_settings();
                self.od.reset_values();
            }
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(have_lib, egui::Button::new("Add")).clicked() {
                let dev = Device { name: String::new(), node_id: self.active_node(), eds_file: self.selected_eds.clone(), bitrate_kbit: self.settings.bitrate_kbit, notes: String::new() };
                self.dialog = Some(Dialog::Device { idx: None, dev, error: None });
            }
            if ui.add_enabled(active.is_some(), egui::Button::new("Edit")).clicked() {
                if let (Some(i), Some(d)) = (active, self.active_device().cloned()) {
                    self.dialog = Some(Dialog::Device { idx: Some(i), dev: d, error: None });
                }
            }
            if ui.add_enabled(active.is_some(), egui::Button::new("Remove")).clicked() {
                if let Some(i) = active {
                    self.dialog = Some(Dialog::RemoveDevice(i));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Node ID");
            match active {
                Some(i) => {
                    let mut n = names[i].1;
                    if ui.add(egui::DragValue::new(&mut n).range(1..=127)).changed() {
                        if let Some(lib) = self.lib.as_mut() {
                            let mut d = lib.devices[i].clone();
                            d.node_id = n;
                            if let Err(e) = lib.update_device(i, d) {
                                self.set_status(format!("Device not saved: {e}"), true);
                            }
                        }
                        self.od.reset_values();
                        self.rebuild_decode();
                    }
                }
                None => {
                    if ui.add(egui::DragValue::new(&mut self.free_node).range(1..=127)).on_hover_text("Node for SDO and NMT when no device is active").changed() {
                        self.od.reset_values();
                    }
                }
            }
        });
    }

    fn eds_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("EDS files");
        let files = self.lib.as_ref().map(|l| l.list_eds()).unwrap_or_default();
        if files.is_empty() {
            ui.label("No EDS files. Add one or drop .eds/.dcf files onto the window.");
        }
        for f in &files {
            let sel = self.selected_eds.as_deref() == Some(f.as_str());
            if ui.selectable_label(sel, f).clicked() {
                self.selected_eds = if sel { None } else { Some(f.clone()) };
            }
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.lib.is_some(), egui::Button::new("Add...")).clicked() {
                if let Some(paths) = rfd::FileDialog::new().add_filter("EDS / DCF", &["eds", "dcf", "EDS", "DCF"]).add_filter("All files", &["*"]).pick_files() {
                    self.add_eds_files(paths);
                }
            }
            if ui.add_enabled(self.selected_eds.is_some(), egui::Button::new("Remove")).clicked() {
                if let Some(f) = self.selected_eds.clone() {
                    self.dialog = Some(Dialog::RemoveEds(f));
                }
            }
        });
        let Some(sel) = self.selected_eds.clone() else { return };
        ui.separator();
        match self.od_for(&sel) {
            Err(e) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            Ok(od) => {
                ui.strong(&sel);
                egui::Grid::new("eds_info").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, v) in od.file_info.iter().chain(od.device_info.iter()) {
                        if v.is_empty() {
                            continue;
                        }
                        ui.label(k);
                        ui.add(egui::Label::new(v).wrap());
                        ui.end_row();
                    }
                    ui.label("Objects");
                    ui.label(format!("{} ({} entries){}", od.objects.len(), od.entry_count(), if od.is_dcf { ", DCF" } else { "" }));
                    ui.end_row();
                    if let Some(n) = od.node_id {
                        ui.label("DCF node ID");
                        ui.label(n.to_string());
                        ui.end_row();
                    }
                });
                if od.warnings.is_empty() {
                    ui.label("No parse warnings.");
                } else {
                    ui.colored_label(ui.visuals().warn_fg_color, format!("{} parse warning(s):", od.warnings.len()));
                    egui::ScrollArea::vertical().id_salt("eds_warn").max_height(160.0).show(ui, |ui| {
                        for w in &od.warnings {
                            ui.add(egui::Label::new(egui::RichText::new(w).small()).wrap());
                        }
                    });
                }
            }
        }
    }

    fn status_line(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let c = if self.status_error { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
            ui.add(egui::Label::new(egui::RichText::new(&self.status).color(c)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let q = self.bus.shared.lock().map(|s| s.sdo_queue).unwrap_or(0);
                if q > 0 {
                    ui.label(format!("SDO queue {q}"));
                    if ui.small_button("Cancel SDO").clicked() {
                        self.bus.send(BusCommand::CancelSdo);
                    }
                }
                if let Some(d) = &self.data_dir {
                    let mode = self.data_mode.map(|m| format!(" ({})", m.label())).unwrap_or_default();
                    ui.label(egui::RichText::new(format!("{}{mode}", d.display())).small()).on_hover_text(format!("Data folder{mode}"));
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let eds_files = self.lib.as_ref().map(|l| l.list_eds()).unwrap_or_default();
        match &mut dialog {
            Dialog::Device { idx, dev, error } => {
                let title = if idx.is_some() { "Edit device" } else { "Add device" };
                let mut save = false;
                egui::Modal::new(egui::Id::new("device_dialog")).show(ctx, |ui| {
                    ui.heading(title);
                    egui::Grid::new("dev_grid").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut dev.name);
                        ui.end_row();
                        ui.label("Node ID");
                        ui.add(egui::DragValue::new(&mut dev.node_id).range(1..=127));
                        ui.end_row();
                        ui.label("EDS file");
                        egui::ComboBox::from_id_salt("dev_eds").selected_text(dev.eds_file.clone().unwrap_or_else(|| "(none)".to_string())).show_ui(ui, |ui| {
                            ui.selectable_value(&mut dev.eds_file, None, "(none)");
                            for f in &eds_files {
                                ui.selectable_value(&mut dev.eds_file, Some(f.clone()), f);
                            }
                        });
                        ui.end_row();
                        ui.label("Bitrate");
                        let cur = Bitrate::from_kbit(dev.bitrate_kbit).map(|b| b.label()).unwrap_or_else(|| "(none)".to_string());
                        egui::ComboBox::from_id_salt("dev_br").selected_text(cur).show_ui(ui, |ui| {
                            for b in Bitrate::ALL {
                                ui.selectable_value(&mut dev.bitrate_kbit, b.kbit(), b.label());
                            }
                        });
                        ui.end_row();
                        ui.label("Notes");
                        ui.add(egui::TextEdit::multiline(&mut dev.notes).desired_rows(3));
                        ui.end_row();
                    });
                    if let Some(e) = error {
                        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            save = true;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if save {
                    if let Some(lib) = self.lib.as_mut() {
                        dev.name = dev.name.trim().to_string();
                        let r = match idx {
                            Some(i) => lib.update_device(*i, dev.clone()),
                            None => lib.add_device(dev.clone()).map(|_| ()),
                        };
                        match r {
                            Ok(()) => {
                                keep = false;
                                self.settings.active_device = Some(dev.name.clone());
                                self.mark_settings();
                                self.od.reset_values();
                                let msg = format!("Device {} saved", dev.name);
                                self.set_status(msg, false);
                            }
                            Err(e) => *error = Some(e.to_string()),
                        }
                    }
                    self.rebuild_decode();
                }
            }
            Dialog::RemoveEds(name) => {
                let mut yes = false;
                egui::Modal::new(egui::Id::new("rm_eds")).show(ctx, |ui| {
                    ui.label(format!("Remove the EDS file {name} from the library? Devices that use it keep no EDS."));
                    ui.horizontal(|ui| {
                        yes = ui.button("Remove").clicked();
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if yes {
                    keep = false;
                    let name = name.clone();
                    if let Some(lib) = self.lib.as_mut() {
                        match lib.remove_eds(&name) {
                            Ok(()) => {
                                self.eds_cache.remove(&name);
                                self.selected_eds = None;
                                self.set_status(format!("Removed {name}"), false);
                            }
                            Err(e) => self.set_status(e.to_string(), true),
                        }
                    }
                    self.rebuild_decode();
                }
            }
            Dialog::RemoveDevice(i) => {
                let i = *i;
                let name = self.lib.as_ref().and_then(|l| l.devices.get(i)).map(|d| d.name.clone()).unwrap_or_default();
                let mut yes = false;
                egui::Modal::new(egui::Id::new("rm_dev")).show(ctx, |ui| {
                    ui.label(format!("Remove the device {name}? The EDS file stays in the library."));
                    ui.horizontal(|ui| {
                        yes = ui.button("Remove").clicked();
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if yes {
                    keep = false;
                    if let Some(lib) = self.lib.as_mut() {
                        match lib.remove_device(i) {
                            Ok(d) => {
                                self.settings.active_device = None;
                                self.mark_settings();
                                self.set_status(format!("Removed device {}", d.name), false);
                            }
                            Err(e) => self.set_status(e.to_string(), true),
                        }
                    }
                    self.od.reset_values();
                    self.rebuild_decode();
                }
            }
        }
        if keep {
            self.dialog = Some(dialog);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_bus();

        // Drag and drop of EDS files.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.add_eds_files(dropped);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        // Window size for the next start.
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            if (r.width() - self.settings.window_width).abs() > 2.0 || (r.height() - self.settings.window_height).abs() > 2.0 {
                self.settings.window_width = r.width();
                self.settings.window_height = r.height();
                self.mark_settings();
            }
        }

        egui::TopBottomPanel::top("connect_bar").show(ctx, |ui| self.top_bar(ui));
        egui::TopBottomPanel::bottom("status_line").show(ctx, |ui| self.status_line(ui));
        egui::SidePanel::left("library").resizable(true).default_width(280.0).width_range(200.0..=600.0).show(ctx, |ui| self.left_panel(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Od, "Object dictionary");
                ui.selectable_value(&mut self.tab, Tab::Network, "Network");
                ui.selectable_value(&mut self.tab, Tab::Trace, "Trace");
            });
            ui.separator();
            match self.tab {
                Tab::Od => self.od_tab(ui),
                Tab::Network => self.network_tab(ui),
                Tab::Trace => self.trace_tab(ui),
            }
        });
        self.dialogs(ctx);

        if hovering {
            egui::Area::new(egui::Id::new("drop_hint")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| ui.heading("Drop .eds / .dcf files to add them to the library"));
            });
        }

        if self.settings_dirty && self.settings_saved.elapsed() > Duration::from_secs(1) {
            self.save_settings();
        }
        if self.connected {
            // Node list ages, frames/s and scan progress.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.bus.send(BusCommand::Disconnect);
        self.save_settings();
    }
}
