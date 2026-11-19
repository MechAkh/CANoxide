//! egui user interface (v0.2): menu bar, connection bar, dock area with
//! the views as tabs, status bar. Design: docs/UI_DESIGN.md.
//!
//! The UI never blocks on the bus: it sends `BusCommand`s and reads
//! `BusEvent`s each frame. Every bus send comes from a button; opening the
//! app, a view or a tab sends nothing.

mod analysis_view;
mod dcf_view;
mod dock;
mod graph;
mod library_view;
mod lss_view;
mod messages_view;
mod network;
mod od;
mod pdo_view;
mod replay_bar;
mod settings_dlg;
pub(crate) mod theme;
mod trace_view;
mod tx_view;
#[cfg(test)]
mod ui_tests;

use crate::bus::{AdapterChoice, BusCommand, BusEvent, BusHandle};
use crate::can::{pcan, Bitrate, ChannelInfo};
use crate::canopen::decode::{decode, DecodeContext};
use crate::canopen::sdo::SdoRequest;
use crate::canopen::task::TaskResult;
use crate::eds::{self, ObjectDictionary};
use crate::library::{self, Device, Library};
use crate::settings::{AdapterKind, Settings, Theme};
use crate::trace::TraceStore;
use dock::Tab;
use eframe::egui;
use egui_dock::{DockArea, DockState};
use egui_phosphor::regular as icon;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The app icon (assets/icon), embedded for the window and the About dialog.
/// The exe icon in Explorer and the taskbar comes from the icon resource of
/// build.rs.
pub const ICON_PNG: &[u8] = include_bytes!("../../assets/icon/canoxide_64.png");

/// The decoded app icon.
pub fn app_icon() -> Result<egui::IconData, String> {
    eframe::icon_data::from_png_bytes(ICON_PNG).map_err(|e| e.to_string())
}

/// Start the GUI. Blocks until the window closes.
pub fn run() -> eframe::Result {
    let boot = Boot::load();
    let size = [boot.settings.window_width.max(1024.0), boot.settings.window_height.max(640.0)];
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("CANoxide {}", crate::VERSION))
        .with_inner_size(size)
        .with_min_inner_size([1024.0, 640.0])
        .with_drag_and_drop(true);
    if let Ok(icon) = app_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("CANoxide", options, Box::new(move |cc| Ok(Box::new(App::new(cc, boot)))))
}

/// What is loaded before the window opens.
struct Boot {
    data_dir: Option<PathBuf>,
    data_mode: Option<library::DataDirMode>,
    data_error: Option<String>,
    settings: Settings,
    library: Option<Library>,
    layout: Option<DockState<Tab>>,
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
                layout: None,
                notes: Vec::new(),
            },
        }
    }

    /// Load settings, library and layout from a given data folder (the UI
    /// tests use a temporary folder; the app uses `CANoxide_data`).
    fn load_dir(dir: PathBuf) -> Boot {
        let mut b = Boot { data_dir: None, data_mode: None, data_error: None, settings: Settings::default(), library: None, layout: None, notes: Vec::new() };
        b.data_dir = Some(dir.clone());
        if let Err(e) = library::ensure_writable(&dir) {
            b.data_error = Some(format!("The data folder {} is not writable: {e}. Nothing is saved.", dir.display()));
            return b;
        }
        let (settings, warn) = Settings::load(&dir);
        b.settings = settings;
        b.notes.extend(warn);
        let (layout, note) = dock::load_layout(&dir);
        b.layout = layout;
        b.notes.extend(note);
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

enum Dialog {
    /// Add (`idx` None) or edit a device.
    Device { idx: Option<usize>, dev: Device, error: Option<String> },
    RemoveEds(String),
    RemoveDevice(usize),
}

/// A question before a bus action (Safety settings).
pub(crate) struct Confirm {
    title: String,
    text: String,
    ok: String,
    danger: bool,
    action: Box<dyn FnOnce(&mut App)>,
}

/// Who started a task (`BusCommand::Task` and friends).
pub(crate) enum TaskOwner {
    PdoRead,
    PdoApply,
    NodeInfo,
    Heartbeat,
    DcfSave(PathBuf),
    DcfCompare,
    DcfRestore,
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
    /// A log file is played back (no send control works).
    replay_active: bool,
    /// A loaded log with bad lines: the question to the user.
    replay_bad: Option<replay_bar::BadLines>,
    /// The scrubber position while it is dragged (seconds).
    replay_drag: Option<f64>,
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
    traces: BTreeMap<u8, trace_view::TraceView>,
    /// Per-COB-ID statistics of the Messages view (fed with the trace).
    msgs: crate::msgstats::MsgStats,
    msgv: messages_view::MessagesView,
    od: od::OdView,
    net: network::NetView,
    graph: graph::GraphView,
    pdo: pdo_view::PdoView,
    txv: tx_view::TxView,
    lssv: lss_view::LssView,
    dcfv: dcf_view::DcfView,
    /// User signals, Find similar bits, Find signal, history, sparklines.
    an: analysis_view::AnalysisState,
    dock: Option<DockState<Tab>>,
    /// No layout was saved: choose A or B by the window width.
    first_layout: bool,
    /// Tabs to open after the dock is drawn (`None`: a new trace tab).
    deferred: Vec<Option<Tab>>,
    settings_dlg: Option<settings_dlg::SettingsDlg>,
    about: bool,
    /// The app icon of the About dialog (made on the first open).
    about_icon: Option<egui::TextureHandle>,
    confirm: Option<Confirm>,
    tasks: HashMap<u64, TaskOwner>,
    /// The OS theme (from the window), for Theme::System.
    os_dark: Option<bool>,
    /// The style key last applied: (dark, font offset, scale).
    style_key: Option<(bool, i8, u32)>,
    status: String,
    status_error: bool,
    dialog: Option<Dialog>,
    next_tag: u64,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, boot: Boot) -> App {
        cc.egui_ctx.set_fonts(theme::fonts());
        let ctx = cc.egui_ctx.clone();
        let bus = BusHandle::spawn(Arc::new(move || ctx.request_repaint()));
        let first_layout = boot.layout.is_none();
        let mut app = App {
            data_dir: boot.data_dir,
            data_mode: boot.data_mode,
            data_error: boot.data_error,
            trace: TraceStore::new(boot.settings.trace_capacity),
            traces: BTreeMap::new(),
            msgs: crate::msgstats::MsgStats::new(),
            msgv: messages_view::MessagesView::default(),
            settings: boot.settings,
            settings_dirty: false,
            settings_saved: Instant::now(),
            lib: boot.library,
            bus,
            connected: false,
            listen_only_active: false,
            replay_active: false,
            replay_bad: None,
            replay_drag: None,
            connect_text: String::new(),
            channels: Vec::new(),
            channel_note: None,
            eds_cache: HashMap::new(),
            selected_eds: None,
            free_node: 1,
            decode_ctx: DecodeContext::default(),
            od: od::OdView::default(),
            net: network::NetView::default(),
            graph: graph::GraphView::default(),
            pdo: pdo_view::PdoView::default(),
            txv: tx_view::TxView::default(),
            lssv: lss_view::LssView::default(),
            dcfv: dcf_view::DcfView::default(),
            an: analysis_view::AnalysisState::default(),
            dock: Some(boot.layout.unwrap_or_else(|| dock::default_layout(1280.0))),
            first_layout,
            deferred: Vec::new(),
            settings_dlg: None,
            about: false,
            about_icon: None,
            confirm: None,
            tasks: HashMap::new(),
            os_dark: None,
            style_key: None,
            status: "Ready. Nothing is sent until you press a button.".to_string(),
            status_error: false,
            dialog: None,
            next_tag: 1,
        };
        if !boot.notes.is_empty() {
            app.set_status(boot.notes.join("; "), true);
        }
        app.lssv.delay_ms = app.settings.lss_switch_delay_ms;
        if let Ok(mut s) = app.bus.signals.lock() {
            s.set_capacity(app.settings.signal_capacity);
        }
        app.sync_traces();
        if let Some(n) = app.txv.load(app.lib.as_ref(), &app.settings.saved_frames, &app.bus) {
            app.set_status(n, true);
        }
        if let Some(n) = app.graph.load(app.lib.as_ref(), &app.bus) {
            app.set_status(n, true);
        }
        app.load_user_signals();
        app.bus.send(BusCommand::ListChannels(app.settings.adapter));
        app.rebuild_decode();
        if app.settings.connect_on_start {
            app.connect();
        }
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

    fn save_layout(&mut self) {
        if self.data_error.is_some() {
            return;
        }
        if let (Some(dir), Some(d)) = (&self.data_dir, &self.dock) {
            if let Err(e) = dock::save_layout(dir, d) {
                self.set_status(format!("Layout not saved: {e}"), true);
            }
        }
    }

    /// True when a send control may be used.
    pub(crate) fn can_send(&self) -> bool {
        self.connected && !self.listen_only_active
    }

    /// The tooltip of a disabled send control.
    pub(crate) fn send_hint(&self) -> &'static str {
        if !self.connected {
            "Connect first"
        } else if self.replay_active {
            "Log replay: nothing is sent"
        } else if self.listen_only_active {
            "Listen-only mode"
        } else {
            ""
        }
    }

    /// A send button: disabled (with the reason as tooltip) unless the bus
    /// may be used and `ok` holds.
    pub(crate) fn send_button(&self, ui: &mut egui::Ui, ok: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
        let can = self.can_send();
        let r = ui.add_enabled(can && ok, egui::Button::new(text));
        if !can {
            r.on_disabled_hover_text(self.send_hint())
        } else {
            r
        }
    }

    /// A send button in the caution style (or danger with `danger`, spec
    /// 5.1): disabled (with the reason) unless the bus may be used and `ok`.
    pub(crate) fn styled_send_button(&self, ui: &mut egui::Ui, ok: bool, danger: bool, text: &str, tip: &str) -> egui::Response {
        let can = self.can_send();
        let r = if danger { theme::danger_button(ui, can && ok, egui_phosphor::regular::LIGHTNING, text) } else { theme::caution_button(ui, can && ok, text) };
        if can {
            r.on_hover_text(tip)
        } else {
            r.on_disabled_hover_text(self.send_hint())
        }
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

    /// The EDS of a node from the library (first device with that node).
    pub(crate) fn od_of_node(&mut self, node: u8) -> Option<Arc<ObjectDictionary>> {
        let file = self.lib.as_ref()?.devices.iter().find(|d| d.node_id == node)?.eds_file.clone()?;
        self.od_for(&file).ok()
    }

    /// Node -> dictionary map for the decoders (UI trace and worker), from
    /// the library.
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
        let pdos: Vec<(u8, Vec<crate::canopen::pdo::PdoConfig>)> = self.pdo.read_configs();
        self.decode_ctx = DecodeContext::new(map.clone());
        self.decode_ctx.set_user_signals(Arc::new(self.an.usigs.clone()));
        for (n, p) in pdos {
            self.decode_ctx.set_node_pdos(n, p);
        }
        self.bus.send(BusCommand::SetDecodeNodes(map));
    }

    pub(crate) fn send_sdo(&mut self, node: u8, req: SdoRequest, pending: od::Pending) {
        let tag = self.tag();
        self.od.pending.insert(tag, pending);
        let opts = crate::canopen::sdo::StartOptions::mode(self.settings.sdo_transfer_mode);
        self.bus.send(BusCommand::SdoOpts { tag, node, req, opts });
    }

    /// Start a task and remember who gets its result.
    pub(crate) fn start_task(&mut self, owner: TaskOwner, task: Box<dyn crate::canopen::task::SdoTask>) -> u64 {
        let tag = self.tag();
        self.tasks.insert(tag, owner);
        self.bus.send(BusCommand::Task { tag, task });
        tag
    }

    /// Run `action` now, or after a question when `ask` is true.
    pub(crate) fn confirm_then(&mut self, ask: bool, title: &str, text: String, ok: &str, danger: bool, action: impl FnOnce(&mut App) + 'static) {
        if ask {
            self.confirm = Some(Confirm { title: title.to_string(), text, ok: ok.to_string(), danger, action: Box::new(action) });
        } else {
            action(self);
        }
    }

    fn connect(&mut self) {
        let bitrate = Bitrate::from_kbit(self.settings.bitrate_kbit).unwrap_or(Bitrate::K250);
        let (adapter, channel) = match self.settings.adapter {
            AdapterKind::Peak => (AdapterChoice::Peak, self.settings.channel),
            AdapterKind::Replay => {
                if self.settings.replay_file.trim().is_empty() {
                    self.set_status("Choose a log file first (Browse...)", true);
                    return;
                }
                let path = PathBuf::from(self.settings.replay_file.trim());
                (AdapterChoice::Replay { path, tail: self.trace.capacity(), ctx: self.decode_ctx.clone() }, 0)
            }
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
        self.bus.send(BusCommand::SetSdoBlockSize(self.settings.sdo_block_size));
        self.bus.send(BusCommand::SetLssTimeout(self.settings.lss_timeout_ms));
        self.bus.send(BusCommand::Connect { adapter, channel, bitrate, listen_only: self.settings.listen_only });
        self.set_status("Connecting...", false);
    }

    // ------------------------------------------------------------ traces

    /// Make the trace views match the open trace tabs.
    fn sync_traces(&mut self) {
        let open = self.dock.as_ref().map(dock::open_traces).unwrap_or_default();
        for n in &open {
            if !self.traces.contains_key(n) {
                let view = if *n == 1 { 0 } else { self.trace.add_view() };
                self.traces.insert(*n, trace_view::TraceView::new(view, &self.settings));
            }
        }
    }

    pub(crate) fn trace_paused(&self, n: u8) -> bool {
        self.traces.get(&n).is_some_and(|t| self.trace.view_paused(t.view))
    }

    #[cfg(test)]
    pub(crate) fn trace_delta(&self, n: u8) -> bool {
        self.traces.get(&n).is_some_and(|t| t.delta)
    }

    pub(crate) fn close_trace(&mut self, n: u8) {
        if let Some(t) = self.traces.remove(&n) {
            if t.view != 0 {
                self.trace.remove_view(t.view);
            } else {
                // View 0 stays; it is reset for the next Trace 1.
                self.trace.set_view_filter(0, Default::default());
                self.trace.set_view_paused(0, false);
            }
        }
    }

    pub(crate) fn new_trace_tab(&mut self) {
        match self.dock.as_ref() {
            Some(d) => {
                let open = dock::open_traces(d);
                if let Some(n) = (1..=dock::MAX_TRACES).find(|n| !open.contains(n)) {
                    self.open_tab(Tab::Trace(n));
                }
            }
            // A view asked while the dock is drawn: after the frame.
            None => self.deferred.push(None),
        }
    }

    pub(crate) fn open_tab(&mut self, tab: Tab) {
        match self.dock.as_mut() {
            Some(d) => dock::open_tab(d, tab),
            None => {
                self.deferred.push(Some(tab));
                return;
            }
        }
        self.sync_traces();
    }

    /// Tab requests made by views while the dock was drawn.
    fn run_deferred(&mut self) {
        for t in std::mem::take(&mut self.deferred) {
            match t {
                Some(tab) => self.open_tab(tab),
                None => self.new_trace_tab(),
            }
        }
    }

    pub(crate) fn close_tab(&mut self, tab: Tab) {
        if let Some(d) = self.dock.as_mut() {
            if dock::close_tab(d, tab) {
                if let Tab::Trace(n) = tab {
                    self.close_trace(n);
                }
            }
        }
    }

    pub(crate) fn is_open(&self, tab: Tab) -> bool {
        self.dock.as_ref().is_some_and(|d| dock::is_open(d, tab))
    }

    fn reset_layout(&mut self, width: f32) {
        let open: Vec<u8> = self.traces.keys().copied().collect();
        for n in open {
            self.close_trace(n);
        }
        self.dock = Some(dock::default_layout(width));
        self.sync_traces();
        self.set_status(format!("Layout {} restored", if width < 1600.0 { "A" } else { "B" }), false);
    }

    // ------------------------------------------------------------ events

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
                BusEvent::Connected { text, listen_only, replay } => {
                    self.connected = true;
                    self.listen_only_active = listen_only;
                    self.replay_active = replay;
                    self.replay_drag = None;
                    if replay {
                        self.trace.clear();
                    }
                    self.set_status(format!("Connected: {text}"), false);
                    self.connect_text = text;
                    self.rebuild_decode();
                    self.graph.on_connected(&self.bus);
                    self.msgs.reset();
                    self.msgv.on_reset();
                }
                BusEvent::Disconnected => {
                    self.connected = false;
                    self.listen_only_active = false;
                    self.replay_active = false;
                    self.replay_drag = None;
                    self.msgv.set_fixed_clock(None);
                    self.net.scanning = false;
                    self.graph.polling = false;
                    self.set_status("Disconnected", false);
                }
                BusEvent::Error(s) => self.set_status(s, true),
                BusEvent::Info(s) => self.set_status(s, false),
                BusEvent::Frames(v) => {
                    for f in v {
                        let d = decode(&f, &self.decode_ctx);
                        self.msgs.on_frame(&f, &d);
                        self.an.sparks.on_frame(&f);
                        self.trace.push(f, d);
                    }
                    self.msgv.on_frames(self.msgs.latest_us());
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
                BusEvent::TaskProgress { .. } => {}
                BusEvent::TaskDone { tag, result } => self.on_task_done(tag, result),
                BusEvent::PdoConfigRead(c) => {
                    self.decode_ctx.set_node_pdos(c.node, c.pdos.clone());
                    self.pdo.on_config_read(c);
                }
                BusEvent::LssDone { tag, result } => self.on_lss_done(tag, result),
                BusEvent::LogStarted { path } => {
                    self.set_status(format!("Trace log started: {}", path.display()), false);
                }
                BusEvent::LogStopped { path, rows, error } => match error {
                    Some(e) => self.set_status(format!("Trace log {} stopped: {e}", path.display()), true),
                    None => self.set_status(format!("Trace log stopped: {rows} row(s) in {}", path.display()), false),
                },
                BusEvent::ReplayBadLines { path, frames, bad, errors } => {
                    let first = errors.first().map(|e| format!("; first: {e}")).unwrap_or_default();
                    self.set_status(format!("{}: {bad} bad line(s){first}", path.display()), true);
                    self.replay_bad = Some(replay_bar::BadLines { path, frames, bad, errors });
                }
                BusEvent::ReplaySnapshot { msgs, tail } => {
                    self.trace.clear();
                    self.an.sparks.clear();
                    for (f, d) in tail {
                        self.an.sparks.on_frame(&f);
                        self.trace.push(f, d);
                    }
                    self.msgs = *msgs;
                    self.msgv.on_reset();
                    self.msgv.on_frames(self.msgs.latest_us());
                }
            }
        }
    }

    fn on_task_done(&mut self, tag: u64, result: TaskResult) {
        let Some(owner) = self.tasks.remove(&tag) else { return };
        match owner {
            TaskOwner::PdoRead | TaskOwner::PdoApply => self.on_pdo_task(owner, result),
            TaskOwner::NodeInfo | TaskOwner::Heartbeat => self.on_net_task(owner, result),
            TaskOwner::DcfSave(_) | TaskOwner::DcfCompare | TaskOwner::DcfRestore => self.on_dcf_task(owner, result),
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

    fn open_eds_dialog(&mut self) {
        if self.lib.is_none() {
            return;
        }
        if let Some(paths) = rfd::FileDialog::new().add_filter("EDS / DCF", &["eds", "dcf", "EDS", "DCF"]).add_filter("All files", &["*"]).pick_files() {
            self.add_eds_files(paths);
        }
    }

    // ------------------------------------------------------------ frame

    fn apply_style(&mut self, ctx: &egui::Context) {
        let os = ctx.input(|i| i.raw.system_theme).map(|t| t == egui::Theme::Dark);
        if os.is_some() {
            self.os_dark = os;
        }
        let s = self.settings_dlg.as_ref().map(|d| &d.draft).unwrap_or(&self.settings);
        let key = (theme::is_dark(s.theme, self.os_dark), s.font_offset, (s.ui_scale * 1000.0) as u32);
        if self.style_key != Some(key) {
            let s = s.clone();
            theme::apply(ctx, &s, self.os_dark);
            self.style_key = Some(key);
        }
    }

    pub(crate) fn is_dark(&self) -> bool {
        theme::is_dark(self.settings.theme, self.os_dark)
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add_enabled(self.lib.is_some(), egui::Button::new("Open EDS...").shortcut_text("Ctrl+O")).clicked() {
                    ui.close_menu();
                    self.open_eds_dialog();
                }
                if ui.button("Save trace as...").clicked() {
                    ui.close_menu();
                    let n = self.traces.keys().next().copied().unwrap_or(1);
                    self.save_trace_csv(n);
                }
                if ui.add_enabled(self.data_dir.is_some(), egui::Button::new("Open data folder")).clicked() {
                    ui.close_menu();
                    if let Some(d) = &self.data_dir {
                        open_folder(d);
                    }
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.close_menu();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                for t in Tab::VIEWS {
                    let mut open = self.is_open(t);
                    if ui.checkbox(&mut open, t.name()).changed() {
                        if open {
                            self.open_tab(t);
                        } else {
                            self.close_tab(t);
                        }
                    }
                }
                for n in 1..=dock::MAX_TRACES {
                    let t = Tab::Trace(n);
                    let mut open = self.is_open(t);
                    if ui.checkbox(&mut open, t.name()).changed() {
                        if open {
                            self.open_tab(t);
                        } else {
                            self.close_tab(t);
                        }
                    }
                }
                ui.separator();
                let n = self.dock.as_ref().map(|d| dock::open_traces(d).len()).unwrap_or(0);
                if ui.add_enabled(n < dock::MAX_TRACES as usize, egui::Button::new("New trace tab")).clicked() {
                    ui.close_menu();
                    self.new_trace_tab();
                }
                ui.separator();
                if ui.button("Reset layout").clicked() {
                    ui.close_menu();
                    let w = ui.ctx().screen_rect().width();
                    self.reset_layout(w);
                }
                if ui.add(egui::Button::new("Full screen").shortcut_text("F11")).clicked() {
                    ui.close_menu();
                    let fs = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
                }
            });
            if ui.button("Settings").on_hover_text("Ctrl+,").clicked() {
                self.open_settings();
            }
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard shortcuts").clicked() {
                    ui.close_menu();
                    self.set_status("Ctrl+, Settings · Ctrl+O Open EDS · Ctrl+Enter connect/disconnect · F11 full screen", false);
                }
                if ui.button("About CANoxide").clicked() {
                    ui.close_menu();
                    self.about = true;
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (glyph, next, tip) = match self.settings.theme {
                    Theme::System => (icon::DESKTOP, Theme::Dark, "Theme: System (click: Dark)"),
                    Theme::Dark => (icon::MOON, Theme::Light, "Theme: Dark (click: Light)"),
                    Theme::Light => (icon::SUN, Theme::System, "Theme: Light (click: System)"),
                };
                if ui.add(egui::Button::new(glyph).frame(false)).on_hover_text(tip).clicked() {
                    self.settings.theme = next;
                    self.mark_settings();
                }
            });
        });
    }

    fn connection_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            let locked = self.connected;
            ui.label(egui::RichText::new("Adapter").small().color(p.text_secondary));
            let before = self.settings.adapter;
            ui.add_enabled_ui(!locked, |ui| {
                egui::ComboBox::from_id_salt("adapter").width(150.0).selected_text(self.settings.adapter.label()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.adapter, AdapterKind::Simulator, AdapterKind::Simulator.label());
                    ui.selectable_value(&mut self.settings.adapter, AdapterKind::Peak, AdapterKind::Peak.label());
                    ui.selectable_value(&mut self.settings.adapter, AdapterKind::Replay, AdapterKind::Replay.label());
                });
            });
            if self.settings.adapter != before {
                self.channels.clear();
                self.channel_note = None;
                self.bus.send(BusCommand::ListChannels(self.settings.adapter));
                self.mark_settings();
            }

            let replay = self.settings.adapter == AdapterKind::Replay;
            ui.label(egui::RichText::new(if replay { "File" } else { "Channel" }).small().color(p.text_secondary));
            if replay {
                self.replay_file_picker(ui, locked);
            } else if self.settings.adapter == AdapterKind::Peak {
                let ch = self.settings.channel;
                ui.add_enabled_ui(!locked, |ui| {
                    egui::ComboBox::from_id_salt("channel").width(120.0).selected_text(pcan::channel_name(ch as u16)).show_ui(ui, |ui| {
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
                    if ui.small_button(icon::ARROW_COUNTER_CLOCKWISE).on_hover_text("Ask the driver which channels are available (sends nothing)").clicked() {
                        self.bus.send(BusCommand::ListChannels(AdapterKind::Peak));
                    }
                });
                if self.settings.channel != ch {
                    self.mark_settings();
                }
                if let Some(n) = &self.channel_note {
                    ui.colored_label(p.warning, n);
                }
            } else {
                ui.label("virtual bus");
            }

            ui.label(egui::RichText::new("Bitrate").small().color(p.text_secondary));
            let br = self.settings.bitrate_kbit;
            ui.add_enabled_ui(!locked, |ui| {
                let cur = Bitrate::from_kbit(br).map(|b| b.label()).unwrap_or_else(|| format!("{br} kbit/s"));
                egui::ComboBox::from_id_salt("bitrate").width(110.0).selected_text(cur).show_ui(ui, |ui| {
                    for b in Bitrate::ALL {
                        ui.selectable_value(&mut self.settings.bitrate_kbit, b.kbit(), b.label());
                    }
                });
                if ui.checkbox(&mut self.settings.listen_only, "Listen only").on_hover_text("Open the channel in listen-only mode; every send control is disabled").changed() {
                    self.settings_dirty = true;
                }
            });
            if self.settings.bitrate_kbit != br {
                self.mark_settings();
            }

            if self.connected {
                if ui.add(egui::Button::new("Disconnect").min_size(egui::vec2(110.0, 0.0))).clicked() {
                    self.bus.send(BusCommand::Disconnect);
                }
            } else {
                let b = egui::Button::new(egui::RichText::new("Connect").color(p.on_accent)).fill(p.accent_fill).min_size(egui::vec2(110.0, 0.0));
                if ui.add(b).on_hover_text("Opens the channel only; nothing is sent").clicked() {
                    self.connect();
                }
            }

            // The node scan of the Network view (one implementation): the
            // Network tab comes to the front to show the result.
            if self.scan_running() {
                if let Some((done, total)) = self.scan_progress() {
                    let frac = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                    ui.add(egui::ProgressBar::new(frac).desired_width(90.0).text(format!("{done} / {total}")));
                }
                if ui.button("Cancel scan").clicked() {
                    self.cancel_scan();
                }
            } else {
                let (a, b) = (self.settings.scan_first, self.settings.scan_last);
                let r = self.send_button(ui, true, "Scan nodes");
                let r = if self.can_send() { r.on_hover_text(format!("SDO read of 0x1000 on nodes {a}..{b}, one at a time")) } else { r };
                if r.clicked() {
                    self.start_scan();
                    self.open_tab(Tab::Network);
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (status, load) = {
                    let s = self.bus.shared.lock().unwrap_or_else(|x| x.into_inner());
                    (s.status.clone(), s.bus_load)
                };
                ui.add(egui::Label::new(egui::RichText::new(format!("{:>6.0}", load.frames_per_s)).monospace()));
                ui.label(egui::RichText::new("fr/s").small().color(p.text_secondary));
                ui.add_space(6.0);
                ui.label(egui::RichText::new(format!("{:.1} %", load.current_pct)).monospace());
                let frac = (load.current_pct / 100.0).clamp(0.0, 1.0) as f32;
                let col = if load.current_pct > 90.0 {
                    p.error
                } else if load.current_pct > 70.0 {
                    p.warning
                } else {
                    p.text_accent
                };
                let (r, _) = ui.allocate_exact_size(egui::vec2(60.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 3.0, p.bg_button);
                let mut f = r;
                f.set_width(r.width() * frac);
                ui.painter().rect_filled(f, 3.0, col);
                ui.label(egui::RichText::new("Load").small().color(p.text_secondary));
                ui.add_space(6.0);
                if self.replay_active {
                    theme::pill(ui, format!("{}  Replay: nothing is sent", icon::INFO), p.info, p.info_tint, self.settings.reduce_colour);
                } else if self.listen_only_active {
                    theme::pill(ui, format!("{}  Listen only", icon::INFO), p.info, p.info_tint, self.settings.reduce_colour);
                }
                if self.connected {
                    let s = status.to_ascii_lowercase();
                    let (fg, bg) = if s.contains("off") {
                        (p.error, p.error_tint)
                    } else if s.contains("passive") || s.contains("warn") {
                        (p.warning, p.warning_tint)
                    } else {
                        (p.success, p.success_tint)
                    };
                    let text = if status.is_empty() { "Error active".to_string() } else { format!("Bus: {status}") };
                    theme::pill(ui, text, fg, bg, self.settings.reduce_colour);
                } else {
                    theme::pill(ui, "Not connected", p.text_secondary, egui::Color32::TRANSPARENT, false);
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        ui.horizontal(|ui| {
            let summary = if self.connected && self.replay_active {
                let name = std::path::Path::new(&self.settings.replay_file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                format!("Replay · {name} · nothing is sent")
            } else if self.connected {
                let ch = match self.settings.adapter {
                    AdapterKind::Simulator => "virtual bus".to_string(),
                    _ => pcan::channel_name(self.settings.channel as u16),
                };
                let br = Bitrate::from_kbit(self.settings.bitrate_kbit).map(|b| b.label()).unwrap_or_default();
                format!("Connected · {} · {ch} · {br}{}", self.settings.adapter.label(), if self.listen_only_active { " · listen only" } else { "" })
            } else {
                "Not connected. Nothing is sent until you press a button.".to_string()
            };
            ui.label(egui::RichText::new(summary).small().color(p.text_secondary));
            ui.separator();
            let (glyph, c) = if self.status_error { (icon::WARNING_CIRCLE, p.error) } else { (icon::CHECK_CIRCLE, p.text_primary) };
            ui.label(egui::RichText::new(glyph).small().color(c));
            ui.add(egui::Label::new(egui::RichText::new(&self.status).small().color(c)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(d) = &self.data_dir {
                    let mode = self.data_mode.map(|m| m.label()).unwrap_or("data folder");
                    let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if ui.add(egui::Button::new(egui::RichText::new(format!("{}  {name} ({mode})", icon::FOLDER_OPEN)).small()).frame(false)).on_hover_text(format!("Data folder ({mode}): {}", d.display())).clicked() {
                        open_folder(d);
                    }
                }
                let pct = if self.trace.capacity() == 0 { 0.0 } else { self.trace.len() as f64 * 100.0 / self.trace.capacity() as f64 };
                ui.label(egui::RichText::new(format!("Trace {pct:.0} % of {}", self.trace.capacity())).small().color(p.text_secondary));
                let (q, prog) = self.bus.shared.lock().map(|s| (s.sdo_queue, s.sdo_progress)).unwrap_or((0, None));
                if q > 0 {
                    if ui.small_button("Cancel SDO").clicked() {
                        self.bus.send(BusCommand::CancelSdo);
                    }
                    let t = match prog {
                        Some((n, i, s, pr)) => match pr.total {
                            Some(t) => format!("SDO {n} {i:04X}:{s:02X} {} / {t} bytes", pr.done),
                            None => format!("SDO {n} {i:04X}:{s:02X} {} bytes", pr.done),
                        },
                        None => format!("SDO queue {q}"),
                    };
                    ui.label(egui::RichText::new(t).small());
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(c) = self.confirm.take() {
            let p = if self.is_dark() { &theme::DARK } else { &theme::LIGHT };
            let mut keep = true;
            let mut yes = false;
            egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.heading(&c.title);
                ui.add(egui::Label::new(&c.text).wrap());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let fill = if c.danger { p.danger_fill } else { p.accent_fill };
                    if ui.add(egui::Button::new(egui::RichText::new(&c.ok).color(p.on_accent)).fill(fill)).clicked() {
                        yes = true;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            if yes {
                (c.action)(self);
            } else if keep {
                self.confirm = Some(c);
            } else {
                self.set_status("Cancelled; nothing sent", false);
            }
        }
        if self.about {
            if self.about_icon.is_none() {
                self.about_icon = app_icon().ok().map(|i| ctx.load_texture("about_icon", egui::ColorImage::from_rgba_unmultiplied([i.width as usize, i.height as usize], &i.rgba), egui::TextureOptions::LINEAR));
            }
            let icon = self.about_icon.clone();
            egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    if let Some(t) = &icon {
                        ui.add(egui::Image::new((t.id(), egui::vec2(48.0, 48.0))).alt_text("CANoxide icon"));
                    }
                    ui.label(egui::RichText::new("CANoxide").text_style(egui::TextStyle::Name("title".into())));
                });
                ui.monospace(format!("Version {}", crate::VERSION));
                ui.label("A portable CANopen bench tool. Copyright (c) 2026 MechAkh. Licence: MIT.");
                ui.label("Fonts: Inter and JetBrains Mono (SIL Open Font License 1.1, see licenses\\). Icons: Phosphor (MIT).");
                ui.hyperlink("https://github.com/MechAkh/CANoxide");
                if ui.button("Close").clicked() {
                    self.about = false;
                }
            });
        }
        self.settings_dialog(ctx);
        self.replay_dialogs(ctx);
        self.graph_dialogs(ctx);
        self.define_signal_dialog(ctx);
        self.similar_window(ctx);
        self.pdo_dialogs(ctx);
        self.library_dialogs(ctx);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let (comma, open, enter, f11) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::O),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F11),
            )
        });
        if comma {
            self.open_settings();
        }
        if open {
            self.open_eds_dialog();
        }
        if enter {
            if self.connected {
                self.bus.send(BusCommand::Disconnect);
            } else {
                self.connect();
            }
        }
        if f11 {
            let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
        }
    }
}

/// Open a folder in the Explorer (only on a user click).
fn open_folder(p: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = p;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_style(ctx);
        self.poll_bus();
        self.shortcuts(ctx);

        // The first start without a saved layout: A or B by the window width.
        if self.first_layout {
            self.first_layout = false;
            let w = ctx.screen_rect().width();
            self.dock = Some(dock::default_layout(w));
            self.sync_traces();
        }

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

        let p = if self.is_dark() { &theme::DARK } else { &theme::LIGHT };
        let bar = egui::Frame::new().fill(p.bg_frame).inner_margin(egui::Margin::symmetric(8, 2));
        egui::TopBottomPanel::top("menu_bar").frame(bar).show(ctx, |ui| self.menu_bar(ui));
        egui::TopBottomPanel::top("connect_bar").frame(bar.inner_margin(egui::Margin::symmetric(8, 4))).show(ctx, |ui| self.connection_bar(ui));
        if self.replay_active {
            let pos = self.bus.shared.lock().map(|s| s.now_us).unwrap_or(0);
            self.msgv.set_fixed_clock(Some(pos));
            egui::TopBottomPanel::top("replay_bar").frame(bar.inner_margin(egui::Margin::symmetric(8, 2))).show(ctx, |ui| self.transport_bar(ui));
        }
        egui::TopBottomPanel::bottom("status_bar").frame(bar).show(ctx, |ui| self.status_bar(ui));
        if let Some(e) = self.data_error.clone() {
            egui::TopBottomPanel::top("data_error").frame(egui::Frame::new().fill(p.error_tint).inner_margin(egui::Margin::symmetric(8, 4))).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(p.error, icon::WARNING_CIRCLE);
                    ui.label(e);
                });
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg_frame).inner_margin(egui::Margin::symmetric(4, 4))).show(ctx, |ui| {
            if let Some(mut d) = self.dock.take() {
                let mut style = egui_dock::Style::from_egui(ui.style());
                style.main_surface_border_stroke = egui::Stroke::NONE;
                style.tab_bar.bg_fill = p.bg_bar;
                style.tab.tab_body.bg_fill = p.bg_panel;
                style.tab.active.bg_fill = p.bg_panel;
                style.tab.active.text_color = p.text_primary;
                style.tab.focused.bg_fill = p.bg_panel;
                style.tab.focused.text_color = p.text_primary;
                style.tab.inactive.bg_fill = p.bg_bar;
                style.tab.inactive.text_color = p.text_secondary;
                style.tab.hovered.bg_fill = p.bg_row_hover;
                style.tab.hovered.text_color = p.text_primary;
                style.separator.color_idle = p.border_subtle;
                style.separator.color_hovered = p.text_accent;
                style.separator.color_dragged = p.text_accent;
                style.overlay.selection_color = p.text_accent.linear_multiply(0.25);
                DockArea::new(&mut d).id(egui::Id::new("dock")).style(style).show_leaf_collapse_buttons(false).show_leaf_close_all_buttons(false).show_inside(ui, &mut dock::Viewer { app: self });
                self.dock = Some(d);
            }
        });
        self.run_deferred();
        self.sync_traces();
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
            // Node list ages, frames/s, graph and progress.
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.bus.send(BusCommand::Disconnect);
        self.save_settings();
        self.save_layout();
    }
}
