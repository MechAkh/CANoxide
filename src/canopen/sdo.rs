//! SDO client (expedited and segmented, no block transfer) as a state
//! machine: frames and time go in, frames to send and results come out.

use super::{abort_text, COB_SDO_RX, COB_SDO_TX};
use crate::can::CanFrame;
use std::fmt;

pub const ABORT_TOGGLE: u32 = 0x0503_0000;
pub const ABORT_TIMEOUT: u32 = 0x0504_0000;
pub const ABORT_COMMAND: u32 = 0x0504_0001;
pub const ABORT_LENGTH: u32 = 0x0607_0010;
pub const ABORT_LENGTH_HIGH: u32 = 0x0607_0012;
pub const ABORT_GENERAL: u32 = 0x0800_0000;

pub const DEFAULT_TIMEOUT_MS: u64 = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdoRequest {
    Upload { index: u16, sub: u8 },
    Download { index: u16, sub: u8, data: Vec<u8> },
}

impl SdoRequest {
    pub fn index_sub(&self) -> (u16, u8) {
        match self {
            SdoRequest::Upload { index, sub } | SdoRequest::Download { index, sub, .. } => (*index, *sub),
        }
    }
    pub fn is_upload(&self) -> bool {
        matches!(self, SdoRequest::Upload { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdoError {
    /// The server sent an abort.
    ServerAbort(u32),
    /// The client found a protocol error and sent this abort code.
    ClientAbort(u32),
    /// No answer in time.
    Timeout,
    /// A transfer is already running.
    Busy,
    InvalidNode,
    /// The transfer was cancelled by the user (abort sent).
    Cancelled,
}

impl fmt::Display for SdoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdoError::ServerAbort(c) => write!(f, "abort 0x{c:08X}: {}", abort_text(*c)),
            SdoError::ClientAbort(c) => write!(f, "protocol error, client abort 0x{c:08X}: {}", abort_text(*c)),
            SdoError::Timeout => write!(f, "timeout, no answer"),
            SdoError::Busy => write!(f, "SDO client busy"),
            SdoError::InvalidNode => write!(f, "node ID must be 1..127"),
            SdoError::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdoOutcome {
    pub node: u8,
    pub index: u16,
    pub sub: u8,
    pub upload: bool,
    /// Upload: the data. Download: empty.
    pub result: Result<Vec<u8>, SdoError>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SdoOutput {
    pub tx: Option<CanFrame>,
    pub done: Option<SdoOutcome>,
}

#[derive(Clone, Debug)]
enum Phase {
    InitUpload,
    UploadSeg { toggle: bool, size: Option<usize>, buf: Vec<u8> },
    InitDownload { expedited: bool },
    DownloadSeg { toggle: bool, pos: usize, last: bool },
}

#[derive(Clone, Debug)]
struct Transfer {
    node: u8,
    index: u16,
    sub: u8,
    data: Vec<u8>,
    upload: bool,
    deadline_ms: u64,
    timeout_ms: u64,
    abort_on_timeout: bool,
    phase: Phase,
}

#[derive(Clone, Debug)]
pub struct SdoClient {
    pub timeout_ms: u64,
    t: Option<Transfer>,
}

impl Default for SdoClient {
    fn default() -> Self {
        SdoClient::new(DEFAULT_TIMEOUT_MS)
    }
}

pub fn abort_frame(node: u8, index: u16, sub: u8, code: u32) -> CanFrame {
    let i = index.to_le_bytes();
    let c = code.to_le_bytes();
    CanFrame::new(COB_SDO_RX + node as u32, &[0x80, i[0], i[1], sub, c[0], c[1], c[2], c[3]])
}

impl SdoClient {
    pub fn new(timeout_ms: u64) -> Self {
        SdoClient { timeout_ms, t: None }
    }

    pub fn is_busy(&self) -> bool {
        self.t.is_some()
    }

    /// Node of the running transfer.
    pub fn node(&self) -> Option<u8> {
        self.t.as_ref().map(|t| t.node)
    }

    /// Start a transfer with the default timeout. Returns the first request frame.
    pub fn start(&mut self, node: u8, req: SdoRequest, now_ms: u64) -> Result<CanFrame, SdoError> {
        self.start_with(node, req, now_ms, self.timeout_ms, true)
    }

    /// Start with an own timeout; `abort_on_timeout = false` sends no abort
    /// after a timeout (used by the node scan).
    pub fn start_with(&mut self, node: u8, req: SdoRequest, now_ms: u64, timeout_ms: u64, abort_on_timeout: bool) -> Result<CanFrame, SdoError> {
        if self.t.is_some() {
            return Err(SdoError::Busy);
        }
        if !(1..=127).contains(&node) {
            return Err(SdoError::InvalidNode);
        }
        let (index, sub) = req.index_sub();
        let ib = index.to_le_bytes();
        let (frame, phase, data, upload) = match req {
            SdoRequest::Upload { .. } => (self.frame(node, [0x40, ib[0], ib[1], sub, 0, 0, 0, 0]), Phase::InitUpload, Vec::new(), true),
            SdoRequest::Download { data, .. } => {
                if (1..=4).contains(&data.len()) {
                    let n = 4 - data.len() as u8;
                    let mut d = [0x23 | (n << 2), ib[0], ib[1], sub, 0, 0, 0, 0];
                    d[4..4 + data.len()].copy_from_slice(&data);
                    (self.frame(node, d), Phase::InitDownload { expedited: true }, data, false)
                } else {
                    let s = (data.len() as u32).to_le_bytes();
                    (self.frame(node, [0x21, ib[0], ib[1], sub, s[0], s[1], s[2], s[3]]), Phase::InitDownload { expedited: false }, data, false)
                }
            }
        };
        self.t = Some(Transfer {
            node,
            index,
            sub,
            data,
            upload,
            deadline_ms: now_ms + timeout_ms,
            timeout_ms,
            abort_on_timeout,
            phase,
        });
        Ok(frame)
    }

    fn frame(&self, node: u8, d: [u8; 8]) -> CanFrame {
        CanFrame::new(COB_SDO_RX + node as u32, &d)
    }

    fn finish(&mut self, result: Result<Vec<u8>, SdoError>, tx: Option<CanFrame>) -> SdoOutput {
        let t = self.t.take().expect("transfer");
        SdoOutput { tx, done: Some(SdoOutcome { node: t.node, index: t.index, sub: t.sub, upload: t.upload, result }) }
    }

    fn client_abort(&mut self, code: u32) -> SdoOutput {
        let (node, index, sub) = {
            let t = self.t.as_ref().expect("transfer");
            (t.node, t.index, t.sub)
        };
        self.finish(Err(SdoError::ClientAbort(code)), Some(abort_frame(node, index, sub, code)))
    }

    /// Cancel the running transfer (sends an abort).
    pub fn cancel(&mut self) -> SdoOutput {
        let Some(t) = self.t.as_ref() else { return SdoOutput::default() };
        let f = abort_frame(t.node, t.index, t.sub, ABORT_GENERAL);
        self.finish(Err(SdoError::Cancelled), Some(f))
    }

    /// Check the timeout.
    pub fn poll(&mut self, now_ms: u64) -> SdoOutput {
        let Some(t) = self.t.as_ref() else { return SdoOutput::default() };
        if now_ms < t.deadline_ms {
            return SdoOutput::default();
        }
        let tx = if t.abort_on_timeout { Some(abort_frame(t.node, t.index, t.sub, ABORT_TIMEOUT)) } else { None };
        self.finish(Err(SdoError::Timeout), tx)
    }

    /// Feed a received frame. Frames that are not SDO responses of the
    /// active node are ignored.
    pub fn on_frame(&mut self, f: &CanFrame, now_ms: u64) -> SdoOutput {
        let Some(t) = self.t.as_mut() else { return SdoOutput::default() };
        if f.extended || f.rtr || f.id != COB_SDO_TX + t.node as u32 {
            return SdoOutput::default();
        }
        let d = f.data;
        let scs = d[0] >> 5;
        if scs == 4 {
            let code = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
            return self.finish(Err(SdoError::ServerAbort(code)), None);
        }
        let idx = u16::from_le_bytes([d[1], d[2]]);
        t.deadline_ms = now_ms + t.timeout_ms;
        let node = t.node;
        match t.phase.clone() {
            Phase::InitUpload => {
                if scs != 2 {
                    return self.client_abort(ABORT_COMMAND);
                }
                if idx != t.index || d[3] != t.sub {
                    // Answer to another request; not ours.
                    return SdoOutput::default();
                }
                let e = d[0] & 0x02 != 0;
                let s = d[0] & 0x01 != 0;
                if e {
                    let len = if s { 4 - ((d[0] >> 2) & 0x03) as usize } else { 4 };
                    return self.finish(Ok(d[4..4 + len].to_vec()), None);
                }
                let size = if s { Some(u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize) } else { None };
                t.phase = Phase::UploadSeg { toggle: false, size, buf: Vec::new() };
                SdoOutput { tx: Some(self.frame(node, [0x60, 0, 0, 0, 0, 0, 0, 0])), done: None }
            }
            Phase::UploadSeg { toggle, size, mut buf } => {
                if scs != 0 {
                    return self.client_abort(ABORT_COMMAND);
                }
                let t_bit = d[0] & 0x10 != 0;
                if t_bit != toggle {
                    return self.client_abort(ABORT_TOGGLE);
                }
                let n = ((d[0] >> 1) & 0x07) as usize;
                let last = d[0] & 0x01 != 0;
                buf.extend_from_slice(&d[1..8 - n]);
                if let Some(sz) = size {
                    if buf.len() > sz {
                        return self.client_abort(ABORT_LENGTH_HIGH);
                    }
                }
                if last {
                    if let Some(sz) = size {
                        if buf.len() != sz {
                            return self.client_abort(ABORT_LENGTH);
                        }
                    }
                    return self.finish(Ok(buf), None);
                }
                let next = !toggle;
                let t = self.t.as_mut().expect("transfer");
                t.phase = Phase::UploadSeg { toggle: next, size, buf };
                SdoOutput { tx: Some(self.frame(node, [0x60 | ((next as u8) << 4), 0, 0, 0, 0, 0, 0, 0])), done: None }
            }
            Phase::InitDownload { expedited } => {
                if scs != 3 {
                    return self.client_abort(ABORT_COMMAND);
                }
                if idx != t.index || d[3] != t.sub {
                    return SdoOutput::default();
                }
                if expedited {
                    return self.finish(Ok(Vec::new()), None);
                }
                let (frame, pos, last) = segment(node, &t.data, 0, false);
                t.phase = Phase::DownloadSeg { toggle: false, pos, last };
                SdoOutput { tx: Some(frame), done: None }
            }
            Phase::DownloadSeg { toggle, pos, last } => {
                if scs != 1 {
                    return self.client_abort(ABORT_COMMAND);
                }
                if (d[0] & 0x10 != 0) != toggle {
                    return self.client_abort(ABORT_TOGGLE);
                }
                if last {
                    return self.finish(Ok(Vec::new()), None);
                }
                let next = !toggle;
                let (frame, pos, last) = segment(node, &t.data, pos, next);
                t.phase = Phase::DownloadSeg { toggle: next, pos, last };
                SdoOutput { tx: Some(frame), done: None }
            }
        }
    }
}

/// Download segment at `pos`; returns the frame, the new position and the last flag.
fn segment(node: u8, data: &[u8], pos: usize, toggle: bool) -> (CanFrame, usize, bool) {
    let end = (pos + 7).min(data.len());
    let chunk = &data[pos..end];
    let n = 7 - chunk.len() as u8;
    let last = end >= data.len();
    let mut d = [0u8; 8];
    d[0] = ((toggle as u8) << 4) | (n << 1) | last as u8;
    d[1..1 + chunk.len()].copy_from_slice(chunk);
    (CanFrame::new(COB_SDO_RX + node as u32, &d), end, last)
}
