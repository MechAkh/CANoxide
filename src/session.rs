//! A blocking driver over one adapter: SDO transfers (all modes), SDO
//! tasks, LSS services, frames sent and received. The CLI and the tests use
//! it; the GUI uses the non-blocking `bus` worker, which runs the same
//! state machines.
//!
//! Nothing is sent unless a method that sends is called; in listen-only
//! mode each such method fails before anything is sent.

use crate::can::sim::{SimAdapter, SimNode};
use crate::can::{Bitrate, CanAdapter, CanError, CanFrame, Dir};
use crate::canopen::lss::{LssCommand, LssError, LssMaster, LssOutput, LssResult};
use crate::canopen::sdo::{SdoClient, SdoOutcome, SdoOutput, SdoRequest, StartOptions};
use crate::canopen::task::{SdoTask, TaskResult};
use std::collections::VecDeque;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Most frames kept for `take_frames`.
pub const FRAME_LOG_CAP: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The command sends, and the session is listen-only.
    ListenOnly(String),
    /// The adapter refused a frame or failed.
    Adapter(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::ListenOnly(w) => write!(f, "{w}: listen-only mode, nothing sent"),
            SessionError::Adapter(e) => write!(f, "adapter: {e}"),
        }
    }
}

/// Cancel flag and progress callback for long work.
#[derive(Default)]
pub struct Ctl<'a> {
    pub cancel: Option<&'a AtomicBool>,
    pub progress: Option<&'a mut dyn FnMut(usize, Option<usize>)>,
}

impl Ctl<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }
    fn report(&mut self, done: usize, total: Option<usize>) {
        if let Some(p) = self.progress.as_mut() {
            p(done, total);
        }
    }
}

pub struct Session {
    adapter: Box<dyn CanAdapter>,
    pub sdo: SdoClient,
    pub lss: LssMaster,
    t0: Instant,
    listen_only: bool,
    bitrate: Bitrate,
    frames: VecDeque<CanFrame>,
    /// Received frames collected during a task step (for `on_frame`).
    tap: Option<Vec<CanFrame>>,
}

impl Session {
    /// Open a channel. Sends nothing.
    pub fn open(mut adapter: Box<dyn CanAdapter>, channel: u32, bitrate: Bitrate, listen_only: bool) -> Result<Session, CanError> {
        adapter.open(channel, bitrate, listen_only)?;
        Ok(Session {
            adapter,
            sdo: SdoClient::default(),
            lss: LssMaster::default(),
            t0: Instant::now(),
            listen_only,
            bitrate,
            frames: VecDeque::new(),
            tap: None,
        })
    }

    /// A session on the simulator with this node.
    pub fn sim(node: SimNode, listen_only: bool) -> Session {
        match Session::open(Box::new(SimAdapter::new(node)), 0, Bitrate::K250, listen_only) {
            Ok(s) => s,
            Err(_) => unreachable!("the simulator always opens"),
        }
    }

    pub fn listen_only(&self) -> bool {
        self.listen_only
    }

    pub fn bitrate(&self) -> Bitrate {
        self.bitrate
    }

    pub fn adapter_name(&self) -> String {
        self.adapter.name()
    }

    pub fn now_us(&self) -> u64 {
        self.t0.elapsed().as_micros() as u64
    }

    pub fn now_ms(&self) -> u64 {
        self.now_us() / 1000
    }

    /// The simulated node, when the adapter is the simulator.
    pub fn sim_node(&mut self) -> Option<&mut SimNode> {
        self.adapter.as_sim().map(|a| a.node_mut())
    }

    pub fn close(&mut self) {
        self.adapter.close();
    }

    fn log(&mut self, f: CanFrame) {
        if let (Some(t), Dir::Rx) = (self.tap.as_mut(), f.dir) {
            t.push(f);
        }
        if self.frames.len() >= FRAME_LOG_CAP {
            self.frames.pop_front();
        }
        self.frames.push_back(f);
    }

    /// All frames sent and received since the last call.
    pub fn take_frames(&mut self) -> Vec<CanFrame> {
        self.frames.drain(..).collect()
    }

    fn guard(&self, what: &str) -> Result<(), SessionError> {
        if self.listen_only {
            return Err(SessionError::ListenOnly(what.to_string()));
        }
        Ok(())
    }

    /// Send one frame (the user asked for it).
    pub fn send(&mut self, mut f: CanFrame) -> Result<(), SessionError> {
        self.guard("send")?;
        self.adapter.send(&f).map_err(|e| SessionError::Adapter(e.to_string()))?;
        f.dir = Dir::Tx;
        f.timestamp_us = self.now_us();
        self.log(f);
        Ok(())
    }

    /// True when the adapter has no more frames to give (the end of a
    /// replayed log file). A live adapter never ends.
    pub fn finished(&self) -> bool {
        self.adapter.finished()
    }

    /// Receive one frame, waiting up to `wait`. A replayed log keeps its own
    /// time stamps and direction.
    pub fn receive(&mut self, wait: Duration) -> Result<Option<CanFrame>, SessionError> {
        match self.adapter.receive(wait).map_err(|e| SessionError::Adapter(e.to_string()))? {
            Some(f) if self.adapter.keeps_timestamps() => {
                self.log(f);
                Ok(Some(f))
            }
            Some(mut f) => {
                f.dir = Dir::Rx;
                f.timestamp_us = self.now_us();
                self.log(f);
                Ok(Some(f))
            }
            None => Ok(None),
        }
    }

    /// Receive for `dur`; `each` sees every frame and returns false to stop.
    pub fn listen(&mut self, dur: Duration, mut each: impl FnMut(&CanFrame) -> bool) -> Result<(), SessionError> {
        let end = Instant::now() + dur;
        while Instant::now() < end {
            if let Some(f) = self.receive(Duration::from_millis(1))? {
                if !each(&f) {
                    break;
                }
            }
        }
        Ok(())
    }

    fn send_out(&mut self, out: &SdoOutput) -> Result<(), SessionError> {
        for f in out.frames() {
            self.send(*f)?;
        }
        Ok(())
    }

    /// One SDO transfer to the end.
    pub fn sdo(&mut self, node: u8, req: SdoRequest, opts: StartOptions) -> Result<SdoOutcome, SessionError> {
        self.sdo_ctl(node, req, opts, &mut Ctl::default())
    }

    /// One SDO transfer with cancel and progress (bytes done / total).
    pub fn sdo_ctl(&mut self, node: u8, req: SdoRequest, opts: StartOptions, ctl: &mut Ctl) -> Result<SdoOutcome, SessionError> {
        self.guard("SDO")?;
        let (index, sub) = req.index_sub();
        let upload = req.is_upload();
        let first = match self.sdo.start_opts(node, req, self.now_ms(), opts) {
            Ok(f) => f,
            Err(e) => return Ok(SdoOutcome { node, index, sub, upload, result: Err(e) }),
        };
        if let Err(e) = self.send(first) {
            let _ = self.sdo.cancel();
            return Err(e);
        }
        loop {
            if ctl.cancelled() {
                let out = self.sdo.cancel();
                self.send_out(&out)?;
                if let Some(d) = out.done {
                    return Ok(d);
                }
            }
            let out = match self.receive(Duration::from_millis(1))? {
                Some(f) => self.sdo.on_frame(&f, self.now_ms()),
                None => self.sdo.poll(self.now_ms()),
            };
            self.send_out(&out)?;
            if let Some(p) = self.sdo.progress() {
                ctl.report(p.done, p.total);
            }
            if let Some(d) = out.done {
                return Ok(d);
            }
        }
    }

    /// Run an SDO task to the end (or to a cancel).
    pub fn run_task(&mut self, mut task: Box<dyn SdoTask>, ctl: &mut Ctl) -> Result<TaskResult, SessionError> {
        self.guard(&task.label())?;
        loop {
            if ctl.cancelled() {
                task.cancel();
                break;
            }
            task.on_time(self.now_ms());
            if task.waiting() {
                if let Some(f) = self.receive(Duration::from_millis(1))? {
                    task.on_frame(&f);
                }
                continue;
            }
            let Some(call) = task.next() else { break };
            self.tap = Some(Vec::new());
            let r = {
                let t = &mut task;
                let cancel = ctl.cancel;
                let outer = &mut ctl.progress;
                let mut prog = |d: usize, n: Option<usize>| {
                    t.on_sdo_progress(d, n);
                    if let Some(p) = outer.as_mut() {
                        let (d, n) = t.progress();
                        p(d, n);
                    }
                };
                self.sdo_ctl(call.node, call.req, StartOptions::mode(call.mode), &mut Ctl { cancel, progress: Some(&mut prog) })
            };
            // Frames received during the transfer (a boot-up message).
            for f in self.tap.take().unwrap_or_default() {
                task.on_frame(&f);
            }
            let o = r?;
            task.on_outcome(&o);
            let (d, t) = task.progress();
            ctl.report(d, t);
        }
        Ok(task.into_result())
    }

    fn send_lss(&mut self, out: &LssOutput) -> Result<(), SessionError> {
        for f in &out.tx {
            self.send(*f)?;
        }
        Ok(())
    }

    /// One LSS service to the end.
    pub fn lss(&mut self, cmd: LssCommand, ctl: &mut Ctl) -> Result<Result<LssResult, LssError>, SessionError> {
        self.guard("LSS")?;
        let out = self.lss.start(cmd, self.now_ms());
        self.send_lss(&out)?;
        if let Some(d) = out.done {
            return Ok(d);
        }
        loop {
            if ctl.cancelled() {
                let out = self.lss.cancel();
                self.send_lss(&out)?;
                if let Some(d) = out.done {
                    return Ok(d);
                }
            }
            let out = match self.receive(Duration::from_millis(1))? {
                Some(f) => self.lss.on_frame(&f, self.now_ms()),
                None => self.lss.poll(self.now_ms()),
            };
            self.send_lss(&out)?;
            if let Some((d, t)) = self.lss.progress() {
                ctl.report(d, Some(t));
            }
            if let Some(d) = out.done {
                return Ok(d);
            }
        }
    }
}

/// Error of an SDO file transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileTransferError {
    Session(SessionError),
    Sdo(crate::canopen::sdo::SdoError),
    Io(String),
}

impl fmt::Display for FileTransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileTransferError::Session(e) => write!(f, "{e}"),
            FileTransferError::Sdo(e) => write!(f, "{e}"),
            FileTransferError::Io(e) => write!(f, "file: {e}"),
        }
    }
}

/// Upload the raw bytes of an object into a file. Returns the byte count.
pub fn upload_to_file(s: &mut Session, node: u8, index: u16, sub: u8, opts: StartOptions, path: &Path, ctl: &mut Ctl) -> Result<usize, FileTransferError> {
    let o = s.sdo_ctl(node, SdoRequest::Upload { index, sub }, opts, ctl).map_err(FileTransferError::Session)?;
    let data = o.result.map_err(FileTransferError::Sdo)?;
    crate::library::write_atomic(path, &data).map_err(|e| FileTransferError::Io(e.to_string()))?;
    Ok(data.len())
}

/// Download the bytes of a file to an object. Returns the byte count.
pub fn download_from_file(s: &mut Session, node: u8, index: u16, sub: u8, opts: StartOptions, path: &Path, ctl: &mut Ctl) -> Result<usize, FileTransferError> {
    let data = std::fs::read(path).map_err(|e| FileTransferError::Io(e.to_string()))?;
    let n = data.len();
    let o = s.sdo_ctl(node, SdoRequest::Download { index, sub, data }, opts, ctl).map_err(FileTransferError::Session)?;
    o.result.map_err(FileTransferError::Sdo)?;
    Ok(n)
}
