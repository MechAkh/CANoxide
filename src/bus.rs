//! Bus worker thread. It owns the adapter, receives and timestamps frames,
//! drives the SDO client, SDO tasks (PDO read/apply, node info, DCF), the
//! node scan, SDO signal polling, the LSS master and the transmit list,
//! tracks node states, bus load and the network overview, writes the
//! continuous trace log, and talks to the UI through channels. The UI
//! never blocks on the bus.
//!
//! Nothing is sent unless a command asks for it: `Connect` only opens the
//! channel. SDO signal polling sends only after `SetSignalPolling(true)`.
//! In listen-only mode every sending command is refused.
//!
//! SDO priority: user requests (`Sdo`) first, then the scan, then tasks,
//! then signal polls. One request at a time.

use crate::busload::{BusLoad, BusLoadStats};
use crate::can::kvaser::KvaserAdapter;
use crate::can::pcan::PcanAdapter;
use crate::can::sim::{SimAdapter, SimNode};
use crate::can::slcan::{self, SlcanAdapter, SlcanTarget};
use crate::can::sontheim::SontheimAdapter;
use crate::can::{Bitrate, CanAdapter, CanFrame, ChannelInfo, Dir};
use crate::canopen::decode::{DecodeContext, Decoded, J1939Tracker, SdoBlockTracker};
use crate::canopen::lss::{LssCommand, LssError, LssMaster, LssOutput, LssResult};
use crate::canopen::nmt::{nmt_frame, sync_frame, NmtCommand, NodeTracker};
use crate::canopen::pdo::NodePdoConfig;
use crate::canopen::sdo::{SdoClient, SdoError, SdoOutcome, SdoOutput, SdoRequest, StartOptions};
use crate::canopen::task::{SdoTask, StepsTask, TaskResult};
use crate::canopen::types::DisplayMode;
use crate::eds::ObjectDictionary;
use crate::msgstats::MsgStats;
use crate::netinfo::{heartbeat_steps, NetworkOverview, NodeInfoTask};
use crate::replay::{FileAdapter, LineError, ParsedLog, Rebuild, ReplayPlayer, Speed, Tick};
use crate::settings::AdapterKind;
use crate::signals::{PollRequest, SignalStore};
use crate::trace::{TraceEntry, TraceFilter, TraceLogger};
use crate::txlist::{Trigger, TxEntry, TxList};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub enum AdapterChoice {
    Peak,
    /// ifm CANfox (Sontheim MT-API); the channel is the net number.
    Canfox,
    /// Kvaser CANlib; the channel is the CANlib channel number.
    Kvaser,
    /// SLCAN on a COM port or over TCP (the channel is not used).
    Slcan(SlcanTarget),
    /// The simulator; `extended_demo` adds the cyclic extended-ID demo
    /// frames (setting `sim_extended_demo`).
    Sim { node_id: u8, od: Option<Arc<ObjectDictionary>>, extended_demo: bool },
    /// Play a log file back (listen-only, never transmits). `tail` is the
    /// trace capacity (the newest decoded frames sent after a seek); `ctx`
    /// the decoder of the UI trace, for the seek rebuild.
    Replay { path: PathBuf, tail: usize, ctx: DecodeContext },
}

/// Frames fed per service round while a seek is rebuilt.
const REBUILD_CHUNK: usize = 20_000;

pub enum BusCommand {
    ListChannels(AdapterKind),
    Connect { adapter: AdapterChoice, channel: u32, bitrate: Bitrate, listen_only: bool },
    Disconnect,
    Send(CanFrame),
    /// A periodic frame. It is an entry of the transmit list with this key
    /// (the keys share one name space with `TxUpsert`).
    StartCyclic { key: u64, frame: CanFrame, period_ms: u64 },
    StopCyclic(u64),
    Sdo { tag: u64, node: u8, req: SdoRequest },
    /// An SDO request with a transfer mode / timeout (`StartOptions`).
    SdoOpts { tag: u64, node: u8, req: SdoRequest, opts: StartOptions },
    /// Drop the queued SDO jobs and abort the running one.
    CancelSdo,
    Nmt { cmd: NmtCommand, node: u8 },
    Sync,
    Scan { from: u8, to: u8, timeout_ms: u64 },
    CancelScan,
    SetSdoTimeout(u64),
    /// Block size the SDO client offers on a block transfer (1..127).
    SetSdoBlockSize(u8),
    SimEmcy { code: u16, reg: u8 },
    // ---- v0.2 ----
    /// Queue a multi-step SDO task (PDO read/apply, node info, DCF, ...).
    /// Ends with `TaskDone`; progress in `TaskProgress` and `Shared::task`.
    Task { tag: u64, task: Box<dyn SdoTask> },
    /// Cancel a queued or running task (it ends with its partial result).
    CancelTask(u64),
    /// Read 0x1000, 0x1001, 0x1018, 0x1008 of these nodes (`None`: every
    /// node of the network overview). A task.
    RefreshNodeInfo { tag: u64, nodes: Option<Vec<u8>> },
    /// Write 0x1017 = `ms` on these nodes (`None`: every known node). A task.
    SetHeartbeatAll { tag: u64, ms: u16, nodes: Option<Vec<u8>> },
    /// EDS of each node for the worker decoder (signals, trace log).
    SetDecodeNodes(HashMap<u8, Arc<ObjectDictionary>>),
    /// Use a PDO configuration (read earlier, from the library) for the
    /// decoder instead of the EDS defaults of that node.
    SetNodePdos(NodePdoConfig),
    /// Back to the EDS defaults for this node.
    ClearNodePdos(u8),
    /// The user signals for the worker decoder (trace log).
    SetUserSignals(Arc<Vec<crate::usersig::UserSignal>>),
    /// Decode 29-bit frames as J1939 (true) or as raw frames. Off also
    /// clears the J1939 view.
    SetJ1939(bool),
    /// Allow (true) or stop SDO signal polling. Off after `Connect`.
    SetSignalPolling(bool),
    /// One LSS service. Ends with `LssDone`.
    Lss { tag: u64, cmd: LssCommand },
    CancelLss,
    SetLssTimeout(u64),
    /// Transmit list: add or replace an entry (does not start it).
    TxUpsert(TxEntry),
    TxRemove(u64),
    /// Start a periodic / on-receive entry (refused in listen-only).
    TxStart(u64),
    TxStop(u64),
    TxStopAll,
    /// Send an entry once.
    TxFire(u64),
    /// Replace the whole list (all entries stopped).
    TxLoad(Vec<TxEntry>),
    /// Start the continuous trace log to a CSV file (export columns).
    StartLog { path: PathBuf, mode: DisplayMode, filter: Option<TraceFilter> },
    StopLog,
    /// Reset the bus load peak and average.
    ResetBusLoad,
    // ---- v0.3: replay ----
    /// Load (true) or drop (false) a log whose bad lines were reported
    /// with `ReplayBadLines`.
    ReplayAccept(bool),
    ReplayPlay,
    ReplayPause,
    /// One frame forward (pauses).
    ReplayStep,
    /// Move to this log time: the views are rebuilt from the start of the
    /// log without pacing (`ReplaySnapshot` at the end).
    ReplaySeek { pos_us: u64, ctx: DecodeContext },
    ReplaySpeed(Speed),
    ReplayLoop(bool),
    Shutdown,
}

#[derive(Debug)]
pub enum BusEvent {
    Channels { kind: AdapterKind, result: Result<Vec<ChannelInfo>, String> },
    Connected { text: String, listen_only: bool, replay: bool },
    Disconnected,
    Error(String),
    Info(String),
    Frames(Vec<CanFrame>),
    SdoDone { tag: u64, outcome: SdoOutcome },
    ScanFound { node: u8, result: Result<u32, String> },
    ScanFinished { cancelled: bool, found: usize },
    // ---- v0.2 ----
    TaskProgress { tag: u64, done: usize, total: Option<usize> },
    TaskDone { tag: u64, result: TaskResult },
    /// A task read a PDO configuration; the worker decoder uses it now.
    PdoConfigRead(NodePdoConfig),
    LssDone { tag: u64, result: Result<LssResult, LssError> },
    LogStarted { path: PathBuf },
    /// The log ended (`StopLog`, disconnect, or a write error).
    LogStopped { path: PathBuf, rows: u64, error: Option<String> },
    // ---- v0.3: replay ----
    /// The log has bad lines: load the rest (`ReplayAccept(true)`) or not.
    ReplayBadLines { path: PathBuf, frames: usize, bad: usize, errors: Vec<LineError> },
    /// The view state at the new replay position (after a seek or a
    /// restart): the UI replaces its Messages statistics and its trace.
    ReplaySnapshot { msgs: Box<MsgStats>, tail: Vec<(CanFrame, Decoded)> },
}

/// The state of a replay (for the transport bar).
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayInfo {
    pub path: PathBuf,
    pub frames: usize,
    pub duration_us: u64,
    pub pos_us: u64,
    /// Frames fed so far.
    pub cursor: usize,
    pub playing: bool,
    pub speed: Speed,
    pub looping: bool,
    /// (done, total) frames of a running seek.
    pub seeking: Option<(usize, usize)>,
}

/// State the UI reads (locked briefly).
#[derive(Default)]
pub struct Shared {
    pub nodes: NodeTracker,
    pub status: String,
    pub fps: f64,
    pub connected: bool,
    pub listen_only: bool,
    /// (done, total) of a running scan.
    pub scan: Option<(usize, usize)>,
    pub now_us: u64,
    pub sdo_queue: usize,
    // ---- v0.2 ----
    pub bus_load: BusLoadStats,
    pub network: NetworkOverview,
    /// PDO configurations in use by the worker decoder, by node.
    pub pdo_configs: BTreeMap<u8, NodePdoConfig>,
    /// Running task: (tag, label, done, total).
    pub task: Option<(u64, String, usize, Option<usize>)>,
    /// Queued tasks (not counting the running one).
    pub task_queue: usize,
    /// (done, total) of a running LSS service.
    pub lss: Option<(usize, usize)>,
    /// Copy of the transmit list (with `enabled` = running).
    pub txlist: Vec<TxEntry>,
    pub signal_polling: bool,
    /// Path and rows of the running trace log.
    pub log: Option<(PathBuf, u64)>,
    /// (node, index, sub, progress) of the running SDO transfer.
    pub sdo_progress: Option<(u8, u16, u8, crate::canopen::sdo::SdoProgress)>,
    // ---- v0.3 ----
    /// The running replay.
    pub replay: Option<ReplayInfo>,
    /// (file, bytes read, file size) while a log loads.
    pub replay_loading: Option<(PathBuf, usize, usize)>,
    /// The frames of the replayed log (the analysis uses them up to the
    /// replay cursor).
    pub replay_frames: Option<Arc<Vec<CanFrame>>>,
    /// The J1939 view: parameter groups, addresses, DM1/DM2 faults (fed
    /// with the received frames while extended IDs are J1939).
    pub j1939: crate::j1939::J1939Monitor,
}

pub type Waker = Arc<dyn Fn() + Send + Sync>;

pub struct BusHandle {
    tx: Sender<BusCommand>,
    rx: Receiver<BusEvent>,
    pub shared: Arc<Mutex<Shared>>,
    /// Signal store fed by the worker (PDO and raw signals from every
    /// frame, SDO polls). The UI adds/removes signals and reads slices.
    pub signals: Arc<Mutex<SignalStore>>,
    thread: Option<JoinHandle<()>>,
}

impl BusHandle {
    pub fn spawn(waker: Waker) -> BusHandle {
        let (ctx, crx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared { status: "not connected".to_string(), ..Default::default() }));
        let signals = Arc::new(Mutex::new(SignalStore::default()));
        let sh = shared.clone();
        let sg = signals.clone();
        let thread = std::thread::Builder::new()
            .name("bus".to_string())
            .spawn(move || Worker::new(etx, sh, sg, waker).run(crx))
            .ok();
        BusHandle { tx: ctx, rx: erx, shared, signals, thread }
    }

    pub fn send(&self, c: BusCommand) {
        let _ = self.tx.send(c);
    }

    pub fn try_recv(&self) -> Option<BusEvent> {
        self.rx.try_recv().ok()
    }
}

impl Drop for BusHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(BusCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Job {
    tag: u64,
    node: u8,
    req: SdoRequest,
    opts: StartOptions,
}

struct Scan {
    next: u8,
    to: u8,
    total: usize,
    done: usize,
    found: usize,
    timeout_ms: u64,
}

enum Running {
    None,
    Job(u64),
    Scan(u8),
    /// A step of the front task.
    Task,
    Poll(u64),
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A log being read by a loader thread.
struct Loading {
    path: PathBuf,
    bitrate: Bitrate,
    tail: usize,
    ctx: DecodeContext,
    progress: Arc<AtomicUsize>,
    total: usize,
    rx: Receiver<Result<ParsedLog, String>>,
}

/// A read log waiting for `ReplayAccept`.
struct PendingLog {
    path: PathBuf,
    bitrate: Bitrate,
    tail: usize,
    ctx: DecodeContext,
    log: ParsedLog,
}

struct ReplayRun {
    path: PathBuf,
    frames: Arc<Vec<CanFrame>>,
    player: ReplayPlayer,
    ctx: DecodeContext,
    tail: usize,
    kbit: u32,
    rebuild: Option<Rebuild>,
}

struct Worker {
    events: Sender<BusEvent>,
    shared: Arc<Mutex<Shared>>,
    signals: Arc<Mutex<SignalStore>>,
    waker: Waker,
    adapter: Option<Box<dyn CanAdapter>>,
    listen_only: bool,
    t0: Instant,
    sdo: SdoClient,
    /// (index, sub) of the last started transfer, for `Shared::sdo_progress`.
    sdo_current: Option<(u16, u8)>,
    jobs: VecDeque<Job>,
    running: Running,
    scan: Option<Scan>,
    tasks: VecDeque<(u64, Box<dyn SdoTask>)>,
    polling: bool,
    polls: VecDeque<PollRequest>,
    lss: LssMaster,
    lss_tag: Option<u64>,
    txlist: TxList,
    decode: DecodeContext,
    block: SdoBlockTracker,
    /// The J1939 transport state of the trace log texts.
    j1939_log: J1939Tracker,
    log: Option<TraceLogger>,
    log_seq: u64,
    load: BusLoad,
    batch: Vec<CanFrame>,
    rx_count: u64,
    /// The last receive error, so that a lasting fault (a dead driver) is
    /// reported once and not in every service round.
    rx_error: Option<String>,
    fps_t: Instant,
    status_t: Instant,
    shutdown: bool,
    replay: Option<ReplayRun>,
    loading: Option<Loading>,
    pending_log: Option<PendingLog>,
}

impl Worker {
    fn new(events: Sender<BusEvent>, shared: Arc<Mutex<Shared>>, signals: Arc<Mutex<SignalStore>>, waker: Waker) -> Worker {
        Worker {
            events,
            shared,
            signals,
            waker,
            adapter: None,
            listen_only: false,
            t0: Instant::now(),
            sdo: SdoClient::default(),
            sdo_current: None,
            jobs: VecDeque::new(),
            running: Running::None,
            scan: None,
            tasks: VecDeque::new(),
            polling: false,
            polls: VecDeque::new(),
            lss: LssMaster::default(),
            lss_tag: None,
            txlist: TxList::default(),
            decode: DecodeContext::default(),
            block: SdoBlockTracker::new(),
            j1939_log: J1939Tracker::new(),
            log: None,
            log_seq: 0,
            load: BusLoad::new(250),
            batch: Vec::new(),
            rx_count: 0,
            rx_error: None,
            fps_t: Instant::now(),
            status_t: Instant::now(),
            shutdown: false,
            replay: None,
            loading: None,
            pending_log: None,
        }
    }

    fn emit(&self, e: BusEvent) {
        let _ = self.events.send(e);
        (self.waker)();
    }

    fn now_us(&self) -> u64 {
        self.t0.elapsed().as_micros() as u64
    }

    fn now_ms(&self) -> u64 {
        self.now_us() / 1000
    }

    fn run(mut self, rx: Receiver<BusCommand>) {
        while !self.shutdown {
            if self.adapter.is_none() {
                let wait = if self.loading.is_some() { 20 } else { 100 };
                match rx.recv_timeout(Duration::from_millis(wait)) {
                    Ok(c) => self.command(c),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            loop {
                match rx.try_recv() {
                    Ok(c) => self.command(c),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.shutdown = true;
                        break;
                    }
                }
            }
            self.poll_loading();
            if self.adapter.is_some() && !self.shutdown {
                self.service();
            }
        }
        self.disconnect();
        self.stop_log(None);
    }

    fn guard_send(&self, what: &str) -> bool {
        if self.adapter.is_none() {
            self.emit(BusEvent::Error(format!("{what}: not connected")));
            return false;
        }
        if self.replay.is_some() {
            self.emit(BusEvent::Error(format!("{what}: a log is played back, nothing is sent")));
            return false;
        }
        if self.listen_only {
            self.emit(BusEvent::Error(format!("{what}: listen-only mode, nothing sent")));
            return false;
        }
        true
    }

    fn cancelled_outcome(node: u8, req: &SdoRequest) -> SdoOutcome {
        let (index, sub) = req.index_sub();
        SdoOutcome { node, index, sub, upload: req.is_upload(), result: Err(SdoError::Cancelled) }
    }

    fn command(&mut self, c: BusCommand) {
        match c {
            BusCommand::Shutdown => self.shutdown = true,
            BusCommand::ListChannels(kind) => {
                let result = match kind {
                    AdapterKind::Simulator => Ok(vec![ChannelInfo { handle: 0, name: "Virtual bus".to_string(), available: true }]),
                    AdapterKind::Peak => PcanAdapter::load().and_then(|mut a| a.list_channels()).map_err(|e| e.to_string()),
                    AdapterKind::Replay => Ok(Vec::new()),
                    AdapterKind::Canfox => SontheimAdapter::load().and_then(|mut a| a.list_channels()).map_err(|e| e.to_string()),
                    AdapterKind::Kvaser => KvaserAdapter::load().and_then(|mut a| a.list_channels()).map_err(|e| e.to_string()),
                    // The COM ports of the registry; no port is opened.
                    AdapterKind::SlcanSerial => Ok(slcan::list_com_ports()
                        .into_iter()
                        .map(|p| ChannelInfo { handle: slcan::com_number(&p).unwrap_or(0), name: p, available: true })
                        .collect()),
                    AdapterKind::SlcanTcp => Ok(Vec::new()),
                };
                self.emit(BusEvent::Channels { kind, result });
            }
            BusCommand::Connect { adapter, channel, bitrate, listen_only } => {
                self.disconnect();
                let mut a: Box<dyn CanAdapter> = match adapter {
                    AdapterChoice::Replay { path, tail, ctx } => {
                        self.start_loading(path, bitrate, tail, ctx);
                        return;
                    }
                    AdapterChoice::Peak => match PcanAdapter::load() {
                        Ok(a) => Box::new(a),
                        Err(e) => {
                            self.emit(BusEvent::Error(format!("PEAK: {e}")));
                            return;
                        }
                    },
                    AdapterChoice::Canfox => match SontheimAdapter::load() {
                        Ok(a) => Box::new(a),
                        Err(e) => {
                            self.emit(BusEvent::Error(format!("CANfox: {e}")));
                            return;
                        }
                    },
                    AdapterChoice::Kvaser => match KvaserAdapter::load() {
                        Ok(a) => Box::new(a),
                        Err(e) => {
                            self.emit(BusEvent::Error(format!("Kvaser: {e}")));
                            return;
                        }
                    },
                    AdapterChoice::Slcan(target) => Box::new(SlcanAdapter::new(target)),
                    AdapterChoice::Sim { node_id, od, extended_demo } => {
                        let node = match od {
                            Some(od) => SimNode::new(node_id, od),
                            None => SimNode::builtin(node_id),
                        };
                        Box::new(SimAdapter::new(node).with_extended_demo(extended_demo))
                    }
                };
                if let Err(e) = a.open(channel, bitrate, listen_only) {
                    self.emit(BusEvent::Error(format!("connect failed: {e}")));
                    return;
                }
                self.t0 = Instant::now();
                self.fps_t = self.t0;
                self.rx_count = 0;
                self.rx_error = None;
                self.listen_only = listen_only;
                self.load = BusLoad::new(bitrate.kbit());
                self.block.reset();
                self.j1939_log.reset();
                self.polling = false;
                self.polls.clear();
                let text = format!("{} at {}{}", a.name(), bitrate.label(), if listen_only { ", listen only" } else { "" });
                self.adapter = Some(a);
                {
                    let mut s = lock(&self.shared);
                    s.connected = true;
                    s.listen_only = listen_only;
                    s.nodes.clear();
                    s.network.clear();
                    s.j1939.clear();
                    s.status = "OK".to_string();
                    s.fps = 0.0;
                    s.bus_load = BusLoadStats::default();
                    s.signal_polling = false;
                }
                self.emit(BusEvent::Connected { text, listen_only, replay: false });
            }
            BusCommand::Disconnect => {
                self.disconnect();
            }
            BusCommand::Send(f) => {
                if self.guard_send("send") {
                    self.transmit(f);
                }
            }
            BusCommand::StartCyclic { key, frame, period_ms } => {
                if self.guard_send("cyclic send") {
                    self.txlist.upsert(TxEntry::from_frame(key, format!("cyclic {key}"), &frame, Trigger::Periodic { period_ms: period_ms.max(1) }));
                    let now = self.now_ms();
                    self.txlist.start(key, now);
                    self.update_tx_shared();
                }
            }
            BusCommand::StopCyclic(key) => {
                self.txlist.remove(key);
                self.update_tx_shared();
            }
            BusCommand::Sdo { tag, node, req } => self.queue_sdo(tag, node, req, StartOptions::default()),
            BusCommand::SdoOpts { tag, node, req, opts } => self.queue_sdo(tag, node, req, opts),
            BusCommand::CancelSdo => {
                let dropped: Vec<Job> = self.jobs.drain(..).collect();
                for j in dropped {
                    self.emit(BusEvent::SdoDone { tag: j.tag, outcome: Self::cancelled_outcome(j.node, &j.req) });
                }
                if let Running::Job(_) = self.running {
                    let out = self.sdo.cancel();
                    self.handle_sdo_output(out);
                }
            }
            BusCommand::Nmt { cmd, node } => {
                if self.guard_send("NMT") {
                    self.transmit(nmt_frame(cmd, node));
                }
            }
            BusCommand::Sync => {
                if self.guard_send("SYNC") {
                    self.transmit(sync_frame());
                }
            }
            BusCommand::Scan { from, to, timeout_ms } => {
                if self.guard_send("scan") {
                    let (from, to) = (from.clamp(1, 127), to.clamp(1, 127));
                    if from <= to {
                        self.scan = Some(Scan { next: from, to, total: (to - from + 1) as usize, done: 0, found: 0, timeout_ms });
                        self.update_scan_shared();
                    }
                }
            }
            BusCommand::CancelScan => {
                if let Some(s) = self.scan.take() {
                    lock(&self.shared).scan = None;
                    self.emit(BusEvent::ScanFinished { cancelled: true, found: s.found });
                }
            }
            BusCommand::SetSdoTimeout(ms) => self.sdo.timeout_ms = ms.max(10),
            BusCommand::SetSdoBlockSize(n) => self.sdo.block_size = n.clamp(1, crate::canopen::sdo::MAX_BLOCK_SIZE),
            BusCommand::SimEmcy { code, reg } => match self.adapter.as_mut().and_then(|a| a.as_sim()) {
                Some(sim) => sim.send_emcy(code, reg),
                None => self.emit(BusEvent::Error("EMCY trigger works only with the simulator".to_string())),
            },
            BusCommand::Task { tag, task } => self.queue_task(tag, task),
            BusCommand::CancelTask(tag) => self.cancel_task(tag),
            BusCommand::RefreshNodeInfo { tag, nodes } => {
                let nodes = nodes.unwrap_or_else(|| lock(&self.shared).network.known_nodes());
                self.queue_task(tag, Box::new(NodeInfoTask::new(&nodes)));
            }
            BusCommand::SetHeartbeatAll { tag, ms, nodes } => {
                let nodes = nodes.unwrap_or_else(|| lock(&self.shared).network.known_nodes());
                let steps = heartbeat_steps(&nodes, ms);
                self.queue_task(tag, Box::new(StepsTask::new(format!("set heartbeat {ms} ms"), steps, false)));
            }
            BusCommand::SetDecodeNodes(nodes) => self.decode.set_nodes(nodes),
            BusCommand::SetUserSignals(v) => self.decode.set_user_signals(v),
            BusCommand::SetJ1939(on) => {
                self.decode.set_j1939(on);
                self.j1939_log.reset();
                if !on {
                    lock(&self.shared).j1939.clear();
                }
            }
            BusCommand::SetNodePdos(cfg) => self.use_pdo_config(cfg),
            BusCommand::ClearNodePdos(node) => {
                self.decode.clear_node_pdos(node);
                lock(&self.shared).pdo_configs.remove(&node);
            }
            BusCommand::SetSignalPolling(on) => {
                if on && !self.guard_send("signal polling") {
                    return;
                }
                self.polling = on;
                if !on {
                    self.polls.clear();
                }
                lock(&self.shared).signal_polling = on;
            }
            BusCommand::Lss { tag, cmd } => {
                if !self.guard_send("LSS") {
                    self.emit(BusEvent::LssDone { tag, result: Err(LssError::Cancelled) });
                    return;
                }
                if self.lss.is_busy() {
                    self.emit(BusEvent::LssDone { tag, result: Err(LssError::Busy) });
                    return;
                }
                self.lss_tag = Some(tag);
                let out = self.lss.start(cmd, self.now_ms());
                self.handle_lss_output(out);
            }
            BusCommand::CancelLss => {
                if self.lss.is_busy() {
                    let out = self.lss.cancel();
                    self.handle_lss_output(out);
                }
            }
            BusCommand::SetLssTimeout(ms) => self.lss.timeout_ms = ms.max(1),
            BusCommand::TxUpsert(e) => {
                let running = self.txlist.get(e.key).is_some_and(|o| o.enabled);
                let key = e.key;
                self.txlist.upsert(TxEntry { enabled: false, ..e });
                if running {
                    let now = self.now_ms();
                    self.txlist.start(key, now);
                }
                self.update_tx_shared();
            }
            BusCommand::TxRemove(key) => {
                self.txlist.remove(key);
                self.update_tx_shared();
            }
            BusCommand::TxStart(key) => {
                if self.guard_send("transmit list") {
                    let now = self.now_ms();
                    self.txlist.start(key, now);
                    self.update_tx_shared();
                }
            }
            BusCommand::TxStop(key) => {
                self.txlist.stop(key);
                self.update_tx_shared();
            }
            BusCommand::TxStopAll => {
                self.txlist.stop_all();
                self.update_tx_shared();
            }
            BusCommand::TxFire(key) => {
                if self.guard_send("transmit list") {
                    match self.txlist.fire(key) {
                        Some(f) => {
                            self.transmit(f);
                        }
                        None => self.emit(BusEvent::Error(format!("transmit list: no entry {key}"))),
                    }
                }
            }
            BusCommand::TxLoad(entries) => {
                self.txlist = TxList::new(entries.into_iter().map(|e| TxEntry { enabled: false, ..e }).collect());
                self.update_tx_shared();
            }
            BusCommand::StartLog { path, mode, filter } => {
                if self.replay.is_some() {
                    self.emit(BusEvent::Error("trace log: not available while a log is played back".to_string()));
                    return;
                }
                self.stop_log(None);
                match TraceLogger::start(&path, mode, filter, self.now_ms()) {
                    Ok(l) => {
                        self.log = Some(l);
                        lock(&self.shared).log = Some((path.clone(), 0));
                        self.emit(BusEvent::LogStarted { path });
                    }
                    Err(e) => self.emit(BusEvent::Error(format!("trace log {}: {e}", path.display()))),
                }
            }
            BusCommand::StopLog => self.stop_log(None),
            BusCommand::ResetBusLoad => {
                self.load.reset();
                lock(&self.shared).bus_load = BusLoadStats::default();
            }
            BusCommand::ReplayAccept(yes) => {
                if let Some(p) = self.pending_log.take() {
                    if yes {
                        self.start_replay(p);
                    } else {
                        self.emit(BusEvent::Info(format!("Log not loaded: {}", p.path.display())));
                    }
                }
            }
            BusCommand::ReplayPlay => {
                let wall = self.now_us();
                if let Some(mut r) = self.replay.take() {
                    if r.rebuild.is_none() && r.player.at_end() {
                        // Play at the end starts again from the beginning.
                        self.begin_rebuild(&mut r, 0);
                    }
                    r.player.play(wall);
                    self.replay = Some(r);
                }
            }
            BusCommand::ReplayPause => {
                if let Some(r) = self.replay.as_mut() {
                    r.player.pause();
                }
            }
            BusCommand::ReplayStep => {
                if let Some(mut r) = self.replay.take() {
                    if r.rebuild.is_none() {
                        if let Some(i) = r.player.step(&r.frames) {
                            let f = r.frames[i];
                            self.feed_replay_frame(f);
                        }
                    }
                    self.replay = Some(r);
                }
            }
            BusCommand::ReplaySeek { pos_us, ctx } => {
                if let Some(mut r) = self.replay.take() {
                    r.ctx = ctx;
                    self.begin_rebuild(&mut r, pos_us);
                    self.replay = Some(r);
                }
            }
            BusCommand::ReplaySpeed(s) => {
                let wall = self.now_us();
                if let Some(r) = self.replay.as_mut() {
                    r.player.set_speed(s, wall);
                }
            }
            BusCommand::ReplayLoop(on) => {
                if let Some(r) = self.replay.as_mut() {
                    r.player.set_looping(on);
                }
            }
        }
    }

    // ------------------------------------------------------------ replay

    fn start_loading(&mut self, path: PathBuf, bitrate: Bitrate, tail: usize, ctx: DecodeContext) {
        let total = std::fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0);
        let progress = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let (p, pr, waker) = (path.clone(), progress.clone(), self.waker.clone());
        let spawned = std::thread::Builder::new().name("replay-load".to_string()).spawn(move || {
            let r = crate::replay::load_log(&p, Some(&pr));
            let _ = tx.send(r);
            waker();
        });
        if let Err(e) = spawned {
            self.emit(BusEvent::Error(format!("replay: {e}")));
            return;
        }
        lock(&self.shared).replay_loading = Some((path.clone(), 0, total));
        self.loading = Some(Loading { path, bitrate, tail, ctx, progress, total, rx });
        (self.waker)();
    }

    fn poll_loading(&mut self) {
        let Some(l) = self.loading.as_ref() else { return };
        let r = match l.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                let done = l.progress.load(Ordering::Relaxed);
                lock(&self.shared).replay_loading = Some((l.path.clone(), done.min(l.total), l.total));
                return;
            }
            Err(TryRecvError::Disconnected) => Err("the loader stopped".to_string()),
        };
        let Some(l) = self.loading.take() else { return };
        lock(&self.shared).replay_loading = None;
        match r {
            Err(e) => self.emit(BusEvent::Error(format!("replay {}: {e}", l.path.display()))),
            Ok(log) => {
                let p = PendingLog { path: l.path, bitrate: l.bitrate, tail: l.tail, ctx: l.ctx, log };
                if p.log.bad_lines > 0 {
                    let ev = BusEvent::ReplayBadLines { path: p.path.clone(), frames: p.log.frames.len(), bad: p.log.bad_lines, errors: p.log.errors.clone() };
                    self.pending_log = Some(p);
                    self.emit(ev);
                } else {
                    self.start_replay(p);
                }
            }
        }
    }

    fn start_replay(&mut self, p: PendingLog) {
        self.disconnect();
        let PendingLog { path, bitrate, tail, ctx, log } = p;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let frames = Arc::new(log.frames);
        let player = ReplayPlayer::new(&frames);
        self.t0 = Instant::now();
        self.fps_t = self.t0;
        self.rx_count = 0;
        self.listen_only = true;
        self.load = BusLoad::new(bitrate.kbit());
        self.block.reset();
        self.j1939_log.reset();
        self.polling = false;
        self.polls.clear();
        lock(&self.signals).clear();
        let mut skipped = if log.bad_lines > 0 { format!(", {} bad line(s) skipped", log.bad_lines) } else { String::new() };
        if log.skipped > 0 {
            skipped.push_str(&format!(", {} other record(s) skipped", log.skipped));
        }
        let text = format!("log file {name} ({}, {} frames, {:.3} s{skipped})", log.format.label(), frames.len(), player.duration_us() as f64 / 1e6);
        self.adapter = Some(Box::new(FileAdapter::new(format!("replay {name}"), Vec::new(), None)));
        let run = ReplayRun { path, frames, player, ctx, tail, kbit: bitrate.kbit(), rebuild: None };
        {
            let mut s = lock(&self.shared);
            s.connected = true;
            s.listen_only = true;
            s.nodes.clear();
            s.network.clear();
            s.j1939.clear();
            s.status = "OK".to_string();
            s.fps = 0.0;
            s.bus_load = BusLoadStats::default();
            s.signal_polling = false;
            s.now_us = 0;
            s.replay = Some(Self::replay_info(&run));
            s.replay_frames = Some(run.frames.clone());
        }
        self.replay = Some(run);
        self.emit(BusEvent::Connected { text, listen_only: true, replay: true });
    }

    fn replay_info(r: &ReplayRun) -> ReplayInfo {
        ReplayInfo {
            path: r.path.clone(),
            frames: r.frames.len(),
            duration_us: r.player.duration_us(),
            pos_us: r.player.pos_us(),
            cursor: r.player.cursor(),
            playing: r.player.playing(),
            speed: r.player.speed(),
            looping: r.player.looping(),
            seeking: r.rebuild.as_ref().map(|b| (b.done, b.target)),
        }
    }

    /// Seek: the views are rebuilt from the start of the log to `pos_us`.
    fn begin_rebuild(&mut self, r: &mut ReplayRun, pos_us: u64) {
        let wall = self.now_us();
        let target = r.player.seek(&r.frames, pos_us, wall);
        lock(&self.signals).clear();
        r.rebuild = Some(Rebuild::new(target, r.player.pos_us(), r.tail, r.kbit));
    }

    fn feed_replay_frame(&mut self, f: CanFrame) {
        self.batch.push(f);
        self.on_bus_frame(&f);
        if f.dir == Dir::Rx {
            let mut s = lock(&self.shared);
            s.nodes.on_frame(&f);
            s.network.on_frame(&f);
            if self.decode.j1939() {
                s.j1939.on_frame(&f);
            }
        }
        self.rx_count += 1;
    }

    /// A service round of a replay: a step of a running seek, or the frames
    /// that are due. Nothing is sent.
    fn service_replay(&mut self) {
        let Some(mut r) = self.replay.take() else { return };
        if let Some(mut rb) = r.rebuild.take() {
            let finished = rb.step(&r.frames, &r.ctx, Some(&mut lock(&self.signals)), REBUILD_CHUNK);
            if finished {
                let Rebuild { msgs, tail, nodes, network, load, pos_us, j1939, .. } = rb;
                self.load = load;
                {
                    let mut s = lock(&self.shared);
                    s.nodes = nodes;
                    s.network = network;
                    s.j1939 = j1939;
                    s.bus_load = self.load.stats();
                    s.now_us = pos_us;
                }
                self.emit(BusEvent::ReplaySnapshot { msgs: Box::new(msgs), tail: tail.into() });
                let wall = self.now_us();
                r.player.resume_clock(wall);
            } else {
                r.rebuild = Some(rb);
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
            let wall = self.now_us();
            match r.player.advance(&r.frames, wall) {
                Tick::Restart => self.begin_rebuild(&mut r, 0),
                Tick::Frames(range) => {
                    let frames = r.frames.clone();
                    for f in &frames[range] {
                        self.feed_replay_frame(*f);
                    }
                }
            }
            if r.rebuild.is_none() {
                self.load.advance(r.player.pos_us());
            }
        }
        {
            let mut s = lock(&self.shared);
            if r.rebuild.is_none() {
                s.now_us = r.player.pos_us();
                s.bus_load = self.load.stats();
            }
            s.replay = Some(Self::replay_info(&r));
            let el = self.fps_t.elapsed();
            if el >= Duration::from_secs(1) {
                s.fps = self.rx_count as f64 / el.as_secs_f64();
                self.rx_count = 0;
                self.fps_t = Instant::now();
            }
        }
        self.replay = Some(r);
        self.flush();
    }

    fn queue_sdo(&mut self, tag: u64, node: u8, req: SdoRequest, opts: StartOptions) {
        if !self.guard_send("SDO") {
            self.emit(BusEvent::SdoDone { tag, outcome: Self::cancelled_outcome(node, &req) });
            return;
        }
        self.jobs.push_back(Job { tag, node, req, opts });
    }

    fn queue_task(&mut self, tag: u64, task: Box<dyn SdoTask>) {
        if !self.guard_send(&task.label()) {
            let mut task = task;
            task.cancel();
            self.emit(BusEvent::TaskDone { tag, result: task.into_result() });
            return;
        }
        self.tasks.push_back((tag, task));
        self.update_task_shared();
    }

    fn cancel_task(&mut self, tag: u64) {
        let Some(pos) = self.tasks.iter().position(|(t, _)| *t == tag) else { return };
        if pos == 0 && matches!(self.running, Running::Task) {
            // The outcome of the aborted step goes to the task first.
            let out = self.sdo.cancel();
            self.handle_sdo_output(out);
        }
        if let Some((_, t)) = self.tasks.get_mut(pos) {
            t.cancel();
        }
        if pos != 0 || !matches!(self.running, Running::Task) {
            if let Some((tag, t)) = self.tasks.remove(pos) {
                self.finish_task(tag, t);
            }
        }
        self.update_task_shared();
    }

    fn finish_task(&mut self, tag: u64, t: Box<dyn SdoTask>) {
        let result = t.into_result();
        if let TaskResult::PdoConfig(cfg) = &result {
            self.use_pdo_config(cfg.clone());
            self.emit(BusEvent::PdoConfigRead(cfg.clone()));
        }
        if let TaskResult::NodeInfo(infos) = &result {
            let mut s = lock(&self.shared);
            for i in infos {
                s.network.merge(i);
            }
        }
        self.emit(BusEvent::TaskDone { tag, result });
    }

    fn use_pdo_config(&mut self, cfg: NodePdoConfig) {
        self.decode.set_node_pdos(cfg.node, cfg.pdos.clone());
        lock(&self.shared).pdo_configs.insert(cfg.node, cfg);
    }

    fn update_task_shared(&self) {
        let v = self.tasks.front().map(|(tag, t)| {
            let (d, n) = t.progress();
            (*tag, t.label(), d, n)
        });
        let mut s = lock(&self.shared);
        s.task = v;
        s.task_queue = self.tasks.len().saturating_sub(1);
    }

    fn update_tx_shared(&self) {
        lock(&self.shared).txlist = self.txlist.entries.clone();
    }

    fn update_scan_shared(&self) {
        let v = self.scan.as_ref().map(|s| (s.done, s.total));
        lock(&self.shared).scan = v;
    }

    fn stop_log(&mut self, error: Option<String>) {
        if let Some(l) = self.log.take() {
            let path = l.path().to_path_buf();
            let (rows, err) = match l.stop() {
                Ok(n) => (n, error),
                Err(e) => (0, Some(error.unwrap_or_else(|| e.to_string()))),
            };
            lock(&self.shared).log = None;
            self.emit(BusEvent::LogStopped { path, rows, error: err });
        }
    }

    fn disconnect(&mut self) {
        if self.loading.take().is_some() {
            lock(&self.shared).replay_loading = None;
        }
        self.pending_log = None;
        if self.replay.take().is_some() {
            let mut s = lock(&self.shared);
            s.replay = None;
            s.replay_frames = None;
        }
        if let Some(mut a) = self.adapter.take() {
            a.close();
            self.flush();
            {
                let mut s = lock(&self.shared);
                s.connected = false;
                s.listen_only = false;
                s.status = "not connected".to_string();
                s.scan = None;
                s.fps = 0.0;
                s.lss = None;
                s.signal_polling = false;
            }
            // Pending SDO work ends.
            let pending: Vec<Job> = self.jobs.drain(..).collect();
            for j in pending {
                self.emit(BusEvent::SdoDone { tag: j.tag, outcome: Self::cancelled_outcome(j.node, &j.req) });
            }
            if self.sdo.is_busy() {
                // No abort frame: the channel is closed.
                let out = self.sdo.cancel();
                self.handle_sdo_output(SdoOutput { tx: None, burst: Vec::new(), done: out.done });
            }
            if let Some(s) = self.scan.take() {
                self.emit(BusEvent::ScanFinished { cancelled: true, found: s.found });
            }
            let tasks: Vec<(u64, Box<dyn SdoTask>)> = self.tasks.drain(..).collect();
            for (tag, mut t) in tasks {
                t.cancel();
                self.finish_task(tag, t);
            }
            self.update_task_shared();
            if self.lss.is_busy() {
                let out = self.lss.cancel();
                self.handle_lss_output(LssOutput { tx: Vec::new(), done: out.done });
            }
            self.polling = false;
            self.polls.clear();
            self.txlist.stop_all();
            self.update_tx_shared();
            self.stop_log(None);
            self.running = Running::None;
            self.emit(BusEvent::Disconnected);
        }
    }

    /// Send a frame; on success it goes to the trace as Tx.
    fn transmit(&mut self, mut f: CanFrame) -> bool {
        let Some(a) = self.adapter.as_mut() else { return false };
        match a.send(&f) {
            Ok(()) => {
                f.dir = Dir::Tx;
                f.timestamp_us = self.t0.elapsed().as_micros() as u64;
                self.on_bus_frame(&f);
                self.batch.push(f);
                true
            }
            Err(e) => {
                self.emit(BusEvent::Error(format!("send 0x{} failed: {e}", f.can_id())));
                false
            }
        }
    }

    /// Bookkeeping for each frame on the bus (sent or received): bus load,
    /// signals, trace log.
    fn on_bus_frame(&mut self, f: &CanFrame) {
        self.load.add(f);
        lock(&self.signals).on_frame(f, &self.decode);
        if self.log.is_some() {
            let mut d = self.block.decode(f, &self.decode);
            self.j1939_log.annotate(f, &self.decode, &mut d);
            let e = TraceEntry::new(self.log_seq, *f, d);
            self.log_seq += 1;
            let now = self.now_ms();
            let r = self.log.as_mut().map(|l| l.write(&e, now));
            if let Some(Err(err)) = r {
                self.stop_log(Some(err.to_string()));
            }
        }
    }

    fn handle_lss_output(&mut self, out: LssOutput) {
        for f in out.tx {
            self.transmit(f);
        }
        lock(&self.shared).lss = self.lss.progress();
        if let Some(result) = out.done {
            let tag = self.lss_tag.take().unwrap_or(0);
            self.emit(BusEvent::LssDone { tag, result });
        }
    }

    fn handle_sdo_output(&mut self, out: SdoOutput) {
        if let Some(f) = out.tx {
            self.transmit(f);
        }
        for f in out.burst {
            self.transmit(f);
        }
        let Some(done) = out.done else { return };
        match std::mem::replace(&mut self.running, Running::None) {
            Running::Job(tag) => self.emit(BusEvent::SdoDone { tag, outcome: done }),
            Running::Scan(node) => {
                let result = match &done.result {
                    Ok(d) => Some(Ok(crate::canopen::types::le_u64(d) as u32)),
                    Err(SdoError::ServerAbort(c)) => Some(Err(format!("abort 0x{c:08X}"))),
                    Err(SdoError::ClientAbort(c)) => Some(Err(format!("protocol error 0x{c:08X}"))),
                    _ => None,
                };
                if let Some(s) = self.scan.as_mut() {
                    s.done += 1;
                    if result.is_some() {
                        s.found += 1;
                    }
                }
                if let Some(r) = result {
                    lock(&self.shared).network.add(node);
                    self.emit(BusEvent::ScanFound { node, result: r });
                }
                self.update_scan_shared();
            }
            Running::Task => {
                if let Some((tag, t)) = self.tasks.front_mut() {
                    t.on_outcome(&done);
                    let (d, n) = t.progress();
                    let tag = *tag;
                    self.emit(BusEvent::TaskProgress { tag, done: d, total: n });
                }
                self.update_task_shared();
            }
            Running::Poll(id) => {
                if let Ok(data) = &done.result {
                    let t = self.now_us() as f64 / 1e6;
                    lock(&self.signals).on_poll_value(id, t, data, &self.decode);
                }
            }
            Running::None => {}
        }
    }

    /// Start an SDO request; a start error or a failed send ends it at once.
    fn start_sdo(&mut self, node: u8, req: SdoRequest, opts: StartOptions, running: Running) -> Result<(), SdoOutcome> {
        let now = self.now_ms();
        let (index, sub) = req.index_sub();
        let upload = req.is_upload();
        match self.sdo.start_opts(node, req, now, opts) {
            Ok(f) => {
                self.running = running;
                self.sdo_current = Some((index, sub));
                if !self.transmit(f) {
                    let out = self.sdo.cancel();
                    self.handle_sdo_output(SdoOutput { tx: None, burst: Vec::new(), done: out.done });
                }
                Ok(())
            }
            Err(e) => Err(SdoOutcome { node, index, sub, upload, result: Err(e) }),
        }
    }

    fn start_next(&mut self) {
        if self.sdo.is_busy() || self.listen_only {
            return;
        }
        if let Some(j) = self.jobs.pop_front() {
            if let Err(outcome) = self.start_sdo(j.node, j.req, j.opts, Running::Job(j.tag)) {
                self.emit(BusEvent::SdoDone { tag: j.tag, outcome });
            }
            return;
        }
        if let Some(s) = self.scan.as_mut() {
            if s.next > s.to {
                let found = s.found;
                self.scan = None;
                self.update_scan_shared();
                self.emit(BusEvent::ScanFinished { cancelled: false, found });
                return;
            }
            let node = s.next;
            let timeout = s.timeout_ms;
            s.next += 1;
            let opts = StartOptions { timeout_ms: Some(timeout), abort_on_timeout: false, ..Default::default() };
            let _ = self.start_sdo(node, SdoRequest::Upload { index: 0x1000, sub: 0 }, opts, Running::Scan(node));
            return;
        }
        // Tasks: a step that cannot start is answered at once; a few per
        // service round at most.
        let now = self.now_ms();
        for _ in 0..16 {
            let Some((_, t)) = self.tasks.front_mut() else { break };
            t.on_time(now);
            if t.waiting() {
                break;
            }
            match t.next() {
                Some(call) => {
                    match self.start_sdo(call.node, call.req, StartOptions::mode(call.mode), Running::Task) {
                        Ok(()) => return,
                        Err(outcome) => {
                            if let Some((_, t)) = self.tasks.front_mut() {
                                t.on_outcome(&outcome);
                            }
                        }
                    }
                }
                None => {
                    if let Some((tag, t)) = self.tasks.pop_front() {
                        self.finish_task(tag, t);
                    }
                    self.update_task_shared();
                }
            }
        }
        if !self.tasks.is_empty() {
            return;
        }
        if self.polling {
            if self.polls.is_empty() {
                let now = self.now_ms();
                let due = lock(&self.signals).polls_due(now);
                self.polls.extend(due);
            }
            if let Some(p) = self.polls.pop_front() {
                let _ = self.start_sdo(p.node, SdoRequest::Upload { index: p.index, sub: p.sub }, StartOptions::default(), Running::Poll(p.id));
            }
        }
    }

    fn service(&mut self) {
        if self.replay.is_some() {
            self.service_replay();
            return;
        }
        let mut received: Vec<CanFrame> = Vec::new();
        for i in 0..2000 {
            let wait = if i == 0 { Duration::from_millis(1) } else { Duration::ZERO };
            let r = match self.adapter.as_mut() {
                Some(a) => a.receive(wait),
                None => return,
            };
            match r {
                Ok(Some(mut f)) => {
                    f.dir = Dir::Rx;
                    f.timestamp_us = self.now_us();
                    let now = f.timestamp_us / 1000;
                    let out = self.sdo.on_frame(&f, now);
                    received.push(f);
                    self.batch.push(f);
                    self.on_bus_frame(&f);
                    if let Some((_, t)) = self.tasks.front_mut() {
                        t.on_frame(&f);
                    }
                    if out != SdoOutput::default() {
                        self.handle_sdo_output(out);
                    }
                    if self.lss.is_busy() {
                        let out = self.lss.on_frame(&f, now);
                        if out != LssOutput::default() {
                            self.handle_lss_output(out);
                        }
                    }
                    if !self.listen_only {
                        for t in self.txlist.on_rx(&f) {
                            self.transmit(t);
                        }
                    }
                }
                Ok(None) => {
                    self.rx_error = None;
                    break;
                }
                Err(e) => {
                    let text = format!("receive: {e}");
                    if self.rx_error.as_deref() != Some(text.as_str()) {
                        self.emit(BusEvent::Error(text.clone()));
                        self.rx_error = Some(text);
                    }
                    // A lasting fault returns at once: do not spin.
                    std::thread::sleep(Duration::from_millis(10));
                    break;
                }
            }
        }
        self.rx_count += received.len() as u64;
        let now = self.now_ms();
        let out = self.sdo.poll(now);
        if out != SdoOutput::default() {
            self.handle_sdo_output(out);
        }
        if self.lss.is_busy() {
            let out = self.lss.poll(now);
            if out != LssOutput::default() {
                self.handle_lss_output(out);
            }
        }
        self.start_next();
        if matches!(self.running, Running::Task) {
            if let (Some(p), Some((_, t))) = (self.sdo.progress(), self.tasks.front_mut()) {
                t.on_sdo_progress(p.done, p.total);
                self.update_task_shared();
            }
        }
        // Periodic entries of the transmit list.
        if !self.listen_only {
            for f in self.txlist.due(now) {
                self.transmit(f);
            }
        }
        if let Some(l) = self.log.as_mut() {
            if let Err(e) = l.tick(now) {
                self.stop_log(Some(e.to_string()));
            }
        }
        let now_us = self.now_us();
        self.load.advance(now_us);
        let status_due = self.status_t.elapsed() >= Duration::from_millis(500);
        let status = if status_due {
            self.status_t = Instant::now();
            self.adapter.as_mut().map(|a| a.status())
        } else {
            None
        };
        {
            let mut s = lock(&self.shared);
            let j1939 = self.decode.j1939();
            for f in &received {
                s.nodes.on_frame(f);
                s.network.on_frame(f);
                if j1939 {
                    s.j1939.on_frame(f);
                }
            }
            s.now_us = now_us;
            s.sdo_queue = self.jobs.len() + self.sdo.is_busy() as usize;
            s.sdo_progress = match (self.sdo.node(), self.sdo.progress(), self.sdo_current) {
                (Some(n), Some(p), Some((i, sub))) => Some((n, i, sub, p)),
                _ => None,
            };
            s.bus_load = self.load.stats();
            if let (Some(l), Some(v)) = (self.log.as_ref(), s.log.as_mut()) {
                v.1 = l.rows();
            }
            if let Some(st) = status {
                s.status = st;
            }
            let el = self.fps_t.elapsed();
            if el >= Duration::from_secs(1) {
                s.fps = self.rx_count as f64 / el.as_secs_f64();
                self.rx_count = 0;
                self.fps_t = Instant::now();
            }
        }
        if self.tasks.front().is_some() {
            self.update_task_shared();
        }
        self.flush();
    }

    fn flush(&mut self) {
        if !self.batch.is_empty() {
            let b = std::mem::take(&mut self.batch);
            self.emit(BusEvent::Frames(b));
        }
    }
}
