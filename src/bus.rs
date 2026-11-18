//! Bus worker thread. It owns the adapter, receives and timestamps frames,
//! drives the SDO client, the node scan and the cyclic frames, tracks the
//! node states, and talks to the UI through channels. The UI never blocks
//! on the bus.
//!
//! Nothing is sent unless a command asks for it: `Connect` only opens the
//! channel.

use crate::can::pcan::PcanAdapter;
use crate::can::sim::{SimAdapter, SimNode};
use crate::can::{Bitrate, CanAdapter, CanFrame, ChannelInfo, Dir};
use crate::canopen::nmt::{nmt_frame, sync_frame, NmtCommand, NodeTracker};
use crate::canopen::sdo::{SdoClient, SdoError, SdoOutcome, SdoOutput, SdoRequest};
use crate::eds::ObjectDictionary;
use crate::settings::AdapterKind;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub enum AdapterChoice {
    Peak,
    Sim { node_id: u8, od: Option<Arc<ObjectDictionary>> },
}

pub enum BusCommand {
    ListChannels(AdapterKind),
    Connect { adapter: AdapterChoice, channel: u32, bitrate: Bitrate, listen_only: bool },
    Disconnect,
    Send(CanFrame),
    StartCyclic { key: u64, frame: CanFrame, period_ms: u64 },
    StopCyclic(u64),
    Sdo { tag: u64, node: u8, req: SdoRequest },
    /// Drop the queued SDO jobs and abort the running one.
    CancelSdo,
    Nmt { cmd: NmtCommand, node: u8 },
    Sync,
    Scan { from: u8, to: u8, timeout_ms: u64 },
    CancelScan,
    SetSdoTimeout(u64),
    SimEmcy { code: u16, reg: u8 },
    Shutdown,
}

#[derive(Debug)]
pub enum BusEvent {
    Channels { kind: AdapterKind, result: Result<Vec<ChannelInfo>, String> },
    Connected { text: String, listen_only: bool },
    Disconnected,
    Error(String),
    Info(String),
    Frames(Vec<CanFrame>),
    SdoDone { tag: u64, outcome: SdoOutcome },
    ScanFound { node: u8, result: Result<u32, String> },
    ScanFinished { cancelled: bool, found: usize },
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
}

pub type Waker = Arc<dyn Fn() + Send + Sync>;

pub struct BusHandle {
    tx: Sender<BusCommand>,
    rx: Receiver<BusEvent>,
    pub shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl BusHandle {
    pub fn spawn(waker: Waker) -> BusHandle {
        let (ctx, crx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared { status: "not connected".to_string(), ..Default::default() }));
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("bus".to_string())
            .spawn(move || Worker::new(etx, sh, waker).run(crx))
            .ok();
        BusHandle { tx: ctx, rx: erx, shared, thread }
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
}

struct Scan {
    next: u8,
    to: u8,
    total: usize,
    done: usize,
    found: usize,
    timeout_ms: u64,
}

struct Cyclic {
    key: u64,
    frame: CanFrame,
    period_ms: u64,
    next_ms: u64,
}

enum Running {
    None,
    Job(u64),
    Scan(u8),
}

struct Worker {
    events: Sender<BusEvent>,
    shared: Arc<Mutex<Shared>>,
    waker: Waker,
    adapter: Option<Box<dyn CanAdapter>>,
    listen_only: bool,
    t0: Instant,
    sdo: SdoClient,
    jobs: VecDeque<Job>,
    running: Running,
    scan: Option<Scan>,
    cyclic: Vec<Cyclic>,
    batch: Vec<CanFrame>,
    rx_count: u64,
    fps_t: Instant,
    status_t: Instant,
    shutdown: bool,
}

impl Worker {
    fn new(events: Sender<BusEvent>, shared: Arc<Mutex<Shared>>, waker: Waker) -> Worker {
        Worker {
            events,
            shared,
            waker,
            adapter: None,
            listen_only: false,
            t0: Instant::now(),
            sdo: SdoClient::default(),
            jobs: VecDeque::new(),
            running: Running::None,
            scan: None,
            cyclic: Vec::new(),
            batch: Vec::new(),
            rx_count: 0,
            fps_t: Instant::now(),
            status_t: Instant::now(),
            shutdown: false,
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
                match rx.recv_timeout(Duration::from_millis(100)) {
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
            if self.adapter.is_some() && !self.shutdown {
                self.service();
            }
        }
        self.disconnect();
    }

    fn guard_send(&self, what: &str) -> bool {
        if self.adapter.is_none() {
            self.emit(BusEvent::Error(format!("{what}: not connected")));
            return false;
        }
        if self.listen_only {
            self.emit(BusEvent::Error(format!("{what}: listen-only mode, nothing sent")));
            return false;
        }
        true
    }

    fn command(&mut self, c: BusCommand) {
        match c {
            BusCommand::Shutdown => self.shutdown = true,
            BusCommand::ListChannels(kind) => {
                let result = match kind {
                    AdapterKind::Simulator => Ok(vec![ChannelInfo { handle: 0, name: "Virtual bus".to_string(), available: true }]),
                    AdapterKind::Peak => PcanAdapter::load().and_then(|mut a| a.list_channels()).map_err(|e| e.to_string()),
                };
                self.emit(BusEvent::Channels { kind, result });
            }
            BusCommand::Connect { adapter, channel, bitrate, listen_only } => {
                self.disconnect();
                let mut a: Box<dyn CanAdapter> = match adapter {
                    AdapterChoice::Peak => match PcanAdapter::load() {
                        Ok(a) => Box::new(a),
                        Err(e) => {
                            self.emit(BusEvent::Error(format!("PEAK: {e}")));
                            return;
                        }
                    },
                    AdapterChoice::Sim { node_id, od } => {
                        let node = match od {
                            Some(od) => SimNode::new(node_id, od),
                            None => SimNode::builtin(node_id),
                        };
                        Box::new(SimAdapter::new(node))
                    }
                };
                if let Err(e) = a.open(channel, bitrate, listen_only) {
                    self.emit(BusEvent::Error(format!("connect failed: {e}")));
                    return;
                }
                self.t0 = Instant::now();
                self.fps_t = self.t0;
                self.rx_count = 0;
                self.listen_only = listen_only;
                let text = format!("{} at {}{}", a.name(), bitrate.label(), if listen_only { ", listen only" } else { "" });
                self.adapter = Some(a);
                {
                    let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
                    s.connected = true;
                    s.listen_only = listen_only;
                    s.nodes.clear();
                    s.status = "OK".to_string();
                    s.fps = 0.0;
                }
                self.emit(BusEvent::Connected { text, listen_only });
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
                    self.cyclic.retain(|c| c.key != key);
                    let next_ms = self.now_ms();
                    self.cyclic.push(Cyclic { key, frame, period_ms: period_ms.max(1), next_ms });
                }
            }
            BusCommand::StopCyclic(key) => self.cyclic.retain(|c| c.key != key),
            BusCommand::Sdo { tag, node, req } => {
                if !self.guard_send("SDO") {
                    let (index, sub) = req.index_sub();
                    let outcome = SdoOutcome { node, index, sub, upload: req.is_upload(), result: Err(SdoError::Cancelled) };
                    self.emit(BusEvent::SdoDone { tag, outcome });
                    return;
                }
                self.jobs.push_back(Job { tag, node, req });
            }
            BusCommand::CancelSdo => {
                let dropped: Vec<Job> = self.jobs.drain(..).collect();
                for j in dropped {
                    let (index, sub) = j.req.index_sub();
                    let outcome = SdoOutcome { node: j.node, index, sub, upload: j.req.is_upload(), result: Err(SdoError::Cancelled) };
                    self.emit(BusEvent::SdoDone { tag: j.tag, outcome });
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
                    self.shared.lock().unwrap_or_else(|p| p.into_inner()).scan = None;
                    self.emit(BusEvent::ScanFinished { cancelled: true, found: s.found });
                }
            }
            BusCommand::SetSdoTimeout(ms) => self.sdo.timeout_ms = ms.max(10),
            BusCommand::SimEmcy { code, reg } => match self.adapter.as_mut().and_then(|a| a.as_sim()) {
                Some(sim) => sim.send_emcy(code, reg),
                None => self.emit(BusEvent::Error("EMCY trigger works only with the simulator".to_string())),
            },
        }
    }

    fn update_scan_shared(&self) {
        let v = self.scan.as_ref().map(|s| (s.done, s.total));
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).scan = v;
    }

    fn disconnect(&mut self) {
        if let Some(mut a) = self.adapter.take() {
            a.close();
            self.flush();
            let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
            s.connected = false;
            s.listen_only = false;
            s.status = "not connected".to_string();
            s.scan = None;
            s.fps = 0.0;
            drop(s);
            // Pending SDO work ends.
            let pending: Vec<Job> = self.jobs.drain(..).collect();
            for j in pending {
                let (index, sub) = j.req.index_sub();
                let outcome = SdoOutcome { node: j.node, index, sub, upload: j.req.is_upload(), result: Err(SdoError::Cancelled) };
                self.emit(BusEvent::SdoDone { tag: j.tag, outcome });
            }
            if self.sdo.is_busy() {
                // No abort frame: the channel is closed.
                let out = self.sdo.cancel();
                self.handle_sdo_output(SdoOutput { tx: None, burst: Vec::new(), done: out.done });
            }
            if let Some(s) = self.scan.take() {
                self.emit(BusEvent::ScanFinished { cancelled: true, found: s.found });
            }
            self.cyclic.clear();
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
                self.batch.push(f);
                true
            }
            Err(e) => {
                self.emit(BusEvent::Error(format!("send 0x{:03X} failed: {e}", f.id)));
                false
            }
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
                    self.emit(BusEvent::ScanFound { node, result: r });
                }
                self.update_scan_shared();
            }
            Running::None => {}
        }
    }

    fn start_next(&mut self) {
        if self.sdo.is_busy() || self.listen_only {
            return;
        }
        let now = self.now_ms();
        if let Some(j) = self.jobs.pop_front() {
            match self.sdo.start(j.node, j.req.clone(), now) {
                Ok(f) => {
                    self.running = Running::Job(j.tag);
                    if !self.transmit(f) {
                        let out = self.sdo.cancel();
                        self.handle_sdo_output(SdoOutput { tx: None, burst: Vec::new(), done: out.done });
                    }
                }
                Err(e) => {
                    let (index, sub) = j.req.index_sub();
                    let outcome = SdoOutcome { node: j.node, index, sub, upload: j.req.is_upload(), result: Err(e) };
                    self.emit(BusEvent::SdoDone { tag: j.tag, outcome });
                }
            }
            return;
        }
        let Some(s) = self.scan.as_mut() else { return };
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
        if let Ok(f) = self.sdo.start_with(node, SdoRequest::Upload { index: 0x1000, sub: 0 }, now, timeout, false) {
            self.running = Running::Scan(node);
            if !self.transmit(f) {
                let out = self.sdo.cancel();
                self.handle_sdo_output(SdoOutput { tx: None, burst: Vec::new(), done: out.done });
            }
        }
    }

    fn service(&mut self) {
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
                    let out = self.sdo.on_frame(&f, f.timestamp_us / 1000);
                    received.push(f);
                    self.batch.push(f);
                    if out != SdoOutput::default() {
                        self.handle_sdo_output(out);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    self.emit(BusEvent::Error(format!("receive: {e}")));
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
        self.start_next();
        // Cyclic frames.
        let due: Vec<CanFrame> = self
            .cyclic
            .iter_mut()
            .filter(|c| now >= c.next_ms)
            .map(|c| {
                c.next_ms = if now > c.next_ms + c.period_ms { now + c.period_ms } else { c.next_ms + c.period_ms };
                c.frame
            })
            .collect();
        for f in due {
            self.transmit(f);
        }
        let status_due = self.status_t.elapsed() >= Duration::from_millis(500);
        let status = if status_due {
            self.status_t = Instant::now();
            self.adapter.as_mut().map(|a| a.status())
        } else {
            None
        };
        {
            let mut s = self.shared.lock().unwrap_or_else(|p| p.into_inner());
            for f in &received {
                s.nodes.on_frame(f);
            }
            s.now_us = self.now_us();
            s.sdo_queue = self.jobs.len() + self.sdo.is_busy() as usize;
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
        self.flush();
    }

    fn flush(&mut self) {
        if !self.batch.is_empty() {
            let b = std::mem::take(&mut self.batch);
            self.emit(BusEvent::Frames(b));
        }
    }
}
