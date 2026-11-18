//! SDO client (expedited, segmented and block transfer, CiA 301) as a state
//! machine: frames and time go in, frames to send and results come out. No
//! I/O in here.
//!
//! Block transfer: the client offers CRC-16-CCITT, negotiates the block
//! size, retransmits from the last good sequence number, ends with the
//! "bytes unused" field and accepts the protocol switch of a server on
//! upload (expedited or segmented answer to a block upload request).

use super::{abort_text, COB_SDO_RX, COB_SDO_TX};
use crate::can::CanFrame;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const ABORT_TOGGLE: u32 = 0x0503_0000;
pub const ABORT_TIMEOUT: u32 = 0x0504_0000;
pub const ABORT_COMMAND: u32 = 0x0504_0001;
pub const ABORT_BLOCK_SIZE: u32 = 0x0504_0002;
pub const ABORT_SEQUENCE: u32 = 0x0504_0003;
pub const ABORT_CRC: u32 = 0x0504_0004;
pub const ABORT_LENGTH: u32 = 0x0607_0010;
pub const ABORT_LENGTH_HIGH: u32 = 0x0607_0012;
pub const ABORT_GENERAL: u32 = 0x0800_0000;

pub const DEFAULT_TIMEOUT_MS: u64 = 1000;
/// Largest block size of CiA 301 (segments per block).
pub const MAX_BLOCK_SIZE: u8 = 127;
/// Largest object the client accepts by upload (protects the memory).
pub const MAX_UPLOAD: usize = 16 * 1024 * 1024;

/// How a transfer is done. `Auto` is expedited for 1..4 bytes, else
/// segmented; it never picks block transfer on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TransferMode {
    #[default]
    Auto,
    /// Segmented, also for a download of 1..4 bytes. An upload is answered
    /// by the server as it likes (expedited or segmented).
    Segmented,
    Block,
}

impl TransferMode {
    pub const ALL: [TransferMode; 3] = [TransferMode::Auto, TransferMode::Segmented, TransferMode::Block];

    pub fn label(self) -> &'static str {
        match self {
            TransferMode::Auto => "auto",
            TransferMode::Segmented => "segmented",
            TransferMode::Block => "block",
        }
    }

    /// "auto", "segmented" / "seg", "block" (case-insensitive).
    pub fn parse(s: &str) -> Option<TransferMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(TransferMode::Auto),
            "segmented" | "seg" => Some(TransferMode::Segmented),
            "block" | "blk" => Some(TransferMode::Block),
            _ => None,
        }
    }
}

/// CRC-16-CCITT of the SDO block transfer (polynomial 0x1021, start 0).
pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

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

impl SdoError {
    /// The abort code of a server or client abort.
    pub fn abort_code(&self) -> Option<u32> {
        match self {
            SdoError::ServerAbort(c) | SdoError::ClientAbort(c) => Some(*c),
            _ => None,
        }
    }
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

/// What one input gave: frames to send (in this order: `tx`, then
/// `burst`) and maybe the end of the transfer. `burst` holds the further
/// segments of a block download sub-block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SdoOutput {
    pub tx: Option<CanFrame>,
    pub burst: Vec<CanFrame>,
    pub done: Option<SdoOutcome>,
}

impl SdoOutput {
    /// All frames to send, in order.
    pub fn frames(&self) -> impl Iterator<Item = &CanFrame> {
        self.tx.iter().chain(self.burst.iter())
    }
}

/// Bytes done of the running transfer; `total` when the size is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdoProgress {
    pub done: usize,
    pub total: Option<usize>,
}

/// Per-transfer options of `SdoClient::start_opts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartOptions {
    pub mode: TransferMode,
    /// `None`: the client timeout.
    pub timeout_ms: Option<u64>,
    /// false: no abort frame after a timeout (node scan).
    pub abort_on_timeout: bool,
}

impl Default for StartOptions {
    fn default() -> Self {
        StartOptions { mode: TransferMode::Auto, timeout_ms: None, abort_on_timeout: true }
    }
}

impl StartOptions {
    pub fn mode(mode: TransferMode) -> Self {
        StartOptions { mode, ..Default::default() }
    }
}

#[derive(Clone, Debug)]
enum Phase {
    InitUpload,
    UploadSeg { toggle: bool, size: Option<usize>, buf: Vec<u8> },
    InitDownload { expedited: bool },
    DownloadSeg { toggle: bool, pos: usize, last: bool },
    /// Block upload initiate sent; the answer may be a protocol switch.
    BlkUpInit,
    /// Receiving a sub-block. `expected` is the next sequence number.
    BlkUpSub { expected: u8, blksize: u8, size: Option<usize>, crc: bool, buf: Vec<u8>, complete: bool },
    /// The last sub-block is acknowledged; waiting for the end frame.
    BlkUpEnd { size: Option<usize>, crc: bool, buf: Vec<u8> },
    BlkDnInit,
    /// A sub-block is sent from segment `start` (`sent` segments); waiting for the ack.
    BlkDnSub { crc: bool, start: usize, sent: u8 },
    BlkDnEnd,
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
    /// Bytes done (progress).
    done: usize,
    total: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct SdoClient {
    pub timeout_ms: u64,
    /// Block size the client asks for on a block upload (1..127).
    pub block_size: u8,
    /// The client offers CRC on block transfers.
    pub block_crc: bool,
    /// Protocol switch threshold of a block upload: the server may answer
    /// an object of up to `pst` bytes with an expedited or segmented
    /// upload. 0 = no switch allowed.
    pub pst: u8,
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

/// Number of 7-byte segments of a block download (at least one).
fn seg_count(len: usize) -> usize {
    len.div_ceil(7).max(1)
}

impl SdoClient {
    pub fn new(timeout_ms: u64) -> Self {
        SdoClient { timeout_ms, block_size: MAX_BLOCK_SIZE, block_crc: true, pst: 0, t: None }
    }

    pub fn is_busy(&self) -> bool {
        self.t.is_some()
    }

    /// Node of the running transfer.
    pub fn node(&self) -> Option<u8> {
        self.t.as_ref().map(|t| t.node)
    }

    /// Progress of the running transfer.
    pub fn progress(&self) -> Option<SdoProgress> {
        self.t.as_ref().map(|t| SdoProgress { done: t.done, total: t.total })
    }

    /// Start a transfer with the default timeout. Returns the first request frame.
    pub fn start(&mut self, node: u8, req: SdoRequest, now_ms: u64) -> Result<CanFrame, SdoError> {
        self.start_opts(node, req, now_ms, StartOptions::default())
    }

    /// Start with an own timeout; `abort_on_timeout = false` sends no abort
    /// after a timeout (used by the node scan).
    pub fn start_with(&mut self, node: u8, req: SdoRequest, now_ms: u64, timeout_ms: u64, abort_on_timeout: bool) -> Result<CanFrame, SdoError> {
        self.start_opts(node, req, now_ms, StartOptions { mode: TransferMode::Auto, timeout_ms: Some(timeout_ms), abort_on_timeout })
    }

    /// Start with a transfer mode and options.
    pub fn start_opts(&mut self, node: u8, req: SdoRequest, now_ms: u64, opts: StartOptions) -> Result<CanFrame, SdoError> {
        if self.t.is_some() {
            return Err(SdoError::Busy);
        }
        if !(1..=127).contains(&node) {
            return Err(SdoError::InvalidNode);
        }
        let (index, sub) = req.index_sub();
        let ib = index.to_le_bytes();
        let cc = (self.block_crc as u8) << 2;
        let blksize = self.block_size.clamp(1, MAX_BLOCK_SIZE);
        let (frame, phase, data, upload) = match (req, opts.mode) {
            (SdoRequest::Upload { .. }, TransferMode::Block) => {
                (self.frame(node, [0xA0 | cc, ib[0], ib[1], sub, blksize, self.pst, 0, 0]), Phase::BlkUpInit, Vec::new(), true)
            }
            (SdoRequest::Upload { .. }, _) => (self.frame(node, [0x40, ib[0], ib[1], sub, 0, 0, 0, 0]), Phase::InitUpload, Vec::new(), true),
            (SdoRequest::Download { data, .. }, TransferMode::Block) => {
                let s = (data.len() as u32).to_le_bytes();
                (self.frame(node, [0xC2 | cc, ib[0], ib[1], sub, s[0], s[1], s[2], s[3]]), Phase::BlkDnInit, data, false)
            }
            (SdoRequest::Download { data, .. }, mode) => {
                if mode == TransferMode::Auto && (1..=4).contains(&data.len()) {
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
        let timeout_ms = opts.timeout_ms.unwrap_or(self.timeout_ms);
        let total = if upload { None } else { Some(data.len()) };
        self.t = Some(Transfer {
            node,
            index,
            sub,
            data,
            upload,
            deadline_ms: now_ms + timeout_ms,
            timeout_ms,
            abort_on_timeout: opts.abort_on_timeout,
            phase,
            done: 0,
            total,
        });
        Ok(frame)
    }

    fn frame(&self, node: u8, d: [u8; 8]) -> CanFrame {
        CanFrame::new(COB_SDO_RX + node as u32, &d)
    }

    fn finish(&mut self, result: Result<Vec<u8>, SdoError>, tx: Option<CanFrame>) -> SdoOutput {
        let t = self.t.take().expect("transfer");
        SdoOutput { tx, burst: Vec::new(), done: Some(SdoOutcome { node: t.node, index: t.index, sub: t.sub, upload: t.upload, result }) }
    }

    fn client_abort(&mut self, code: u32) -> SdoOutput {
        let (node, index, sub) = {
            let t = self.t.as_ref().expect("transfer");
            (t.node, t.index, t.sub)
        };
        self.finish(Err(SdoError::ClientAbort(code)), Some(abort_frame(node, index, sub, code)))
    }

    fn send(tx: CanFrame) -> SdoOutput {
        SdoOutput { tx: Some(tx), burst: Vec::new(), done: None }
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
        // In a block upload sub-block, byte 0 is "c + sequence number"; only
        // 0x80 (sequence 0 is invalid) is an abort there.
        let in_sub = matches!(t.phase, Phase::BlkUpSub { .. });
        if (in_sub && d[0] == 0x80) || (!in_sub && scs == 4) {
            let code = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
            return self.finish(Err(SdoError::ServerAbort(code)), None);
        }
        let idx = u16::from_le_bytes([d[1], d[2]]);
        t.deadline_ms = now_ms + t.timeout_ms;
        let node = t.node;
        match t.phase.clone() {
            Phase::InitUpload => self.upload_init_response(&d, idx, scs),
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
                if buf.len() > MAX_UPLOAD {
                    return self.client_abort(0x0504_0005);
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
                t.done = buf.len();
                t.phase = Phase::UploadSeg { toggle: next, size, buf };
                Self::send(self.frame(node, [0x60 | ((next as u8) << 4), 0, 0, 0, 0, 0, 0, 0]))
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
                Self::send(frame)
            }
            Phase::DownloadSeg { toggle, pos, last } => {
                if scs != 1 {
                    return self.client_abort(ABORT_COMMAND);
                }
                if (d[0] & 0x10 != 0) != toggle {
                    return self.client_abort(ABORT_TOGGLE);
                }
                t.done = pos;
                if last {
                    return self.finish(Ok(Vec::new()), None);
                }
                let next = !toggle;
                let (frame, pos, last) = segment(node, &t.data, pos, next);
                t.phase = Phase::DownloadSeg { toggle: next, pos, last };
                Self::send(frame)
            }
            Phase::BlkUpInit => {
                if scs == 2 {
                    // Protocol switch: the server answers as a normal upload.
                    return self.upload_init_response(&d, idx, scs);
                }
                if scs != 6 || d[0] & 0x01 != 0 {
                    return self.client_abort(ABORT_COMMAND);
                }
                if idx != t.index || d[3] != t.sub {
                    return SdoOutput::default();
                }
                let crc = d[0] & 0x04 != 0 && self.block_crc;
                let size = if d[0] & 0x02 != 0 { Some(u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize) } else { None };
                if size.is_some_and(|s| s > MAX_UPLOAD) {
                    return self.client_abort(0x0504_0005);
                }
                let blksize = self.block_size.clamp(1, MAX_BLOCK_SIZE);
                let t = self.t.as_mut().expect("transfer");
                t.total = size;
                t.phase = Phase::BlkUpSub { expected: 1, blksize, size, crc, buf: Vec::new(), complete: false };
                Self::send(self.frame(node, [0xA3, 0, 0, 0, 0, 0, 0, 0]))
            }
            Phase::BlkUpSub { expected, blksize, size, crc, mut buf, complete } => {
                let c = d[0] & 0x80 != 0;
                let seq = d[0] & 0x7F;
                if seq == 0 || seq > blksize {
                    return self.client_abort(ABORT_SEQUENCE);
                }
                let mut ack_seq = expected - 1;
                let mut complete = complete;
                if seq == expected {
                    buf.extend_from_slice(&d[1..8]);
                    ack_seq = seq;
                    if c {
                        complete = true;
                    }
                }
                if size.is_some_and(|s| buf.len() > s + 7) || buf.len() > MAX_UPLOAD + 7 {
                    return self.client_abort(ABORT_LENGTH_HIGH);
                }
                let t = self.t.as_mut().expect("transfer");
                t.done = match size {
                    Some(s) => buf.len().min(s),
                    None => buf.len(),
                };
                // The sub-block ends with its last sequence number or with the
                // last segment of the transfer (also when it came out of order).
                if seq == blksize || c {
                    let ack = self.frame(node, [0xA2, ack_seq, blksize, 0, 0, 0, 0, 0]);
                    let t = self.t.as_mut().expect("transfer");
                    t.phase = if complete {
                        Phase::BlkUpEnd { size, crc, buf }
                    } else {
                        Phase::BlkUpSub { expected: 1, blksize, size, crc, buf, complete: false }
                    };
                    return Self::send(ack);
                }
                t.phase = Phase::BlkUpSub { expected: ack_seq + 1, blksize, size, crc, buf, complete };
                SdoOutput::default()
            }
            Phase::BlkUpEnd { size, crc, mut buf } => {
                if scs != 6 || d[0] & 0x03 != 1 {
                    // A repeated segment after the last ack: ignore it.
                    return SdoOutput::default();
                }
                let n = ((d[0] >> 2) & 0x07) as usize;
                if n > buf.len() {
                    return self.client_abort(ABORT_LENGTH);
                }
                buf.truncate(buf.len() - n);
                if let Some(s) = size {
                    if buf.len() != s {
                        return self.client_abort(ABORT_LENGTH);
                    }
                }
                if crc && crc16_ccitt(&buf) != u16::from_le_bytes([d[1], d[2]]) {
                    return self.client_abort(ABORT_CRC);
                }
                let end = self.frame(node, [0xA1, 0, 0, 0, 0, 0, 0, 0]);
                self.finish(Ok(buf), Some(end))
            }
            Phase::BlkDnInit => {
                if scs != 5 || d[0] & 0x03 != 0 {
                    return self.client_abort(ABORT_COMMAND);
                }
                if idx != t.index || d[3] != t.sub {
                    return SdoOutput::default();
                }
                let blksize = d[4];
                if blksize == 0 || blksize > MAX_BLOCK_SIZE {
                    return self.client_abort(ABORT_BLOCK_SIZE);
                }
                let crc = d[0] & 0x04 != 0 && self.block_crc;
                self.send_block(crc, 0, blksize)
            }
            Phase::BlkDnSub { crc, start, sent } => {
                if scs != 5 || d[0] & 0x03 != 2 {
                    return self.client_abort(ABORT_COMMAND);
                }
                let ack = d[1];
                let blksize = d[2];
                if ack > sent {
                    return self.client_abort(ABORT_SEQUENCE);
                }
                let next = start + ack as usize;
                let total = seg_count(t.data.len());
                t.done = (next * 7).min(t.data.len());
                if next >= total {
                    let n = (total * 7 - t.data.len()) as u8;
                    let c = if crc { crc16_ccitt(&t.data) } else { 0 }.to_le_bytes();
                    t.phase = Phase::BlkDnEnd;
                    return Self::send(self.frame(node, [0xC1 | (n << 2), c[0], c[1], 0, 0, 0, 0, 0]));
                }
                if blksize == 0 || blksize > MAX_BLOCK_SIZE {
                    return self.client_abort(ABORT_BLOCK_SIZE);
                }
                self.send_block(crc, next, blksize)
            }
            Phase::BlkDnEnd => {
                if scs != 5 || d[0] & 0x03 != 1 {
                    return self.client_abort(ABORT_COMMAND);
                }
                self.finish(Ok(Vec::new()), None)
            }
        }
    }

    /// The answer to an upload initiate (also after a protocol switch).
    fn upload_init_response(&mut self, d: &[u8; 8], idx: u16, scs: u8) -> SdoOutput {
        let t = self.t.as_mut().expect("transfer");
        let node = t.node;
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
        t.total = size;
        t.phase = Phase::UploadSeg { toggle: false, size, buf: Vec::new() };
        Self::send(self.frame(node, [0x60, 0, 0, 0, 0, 0, 0, 0]))
    }

    /// Send one sub-block of a block download from segment `start`.
    fn send_block(&mut self, crc: bool, start: usize, blksize: u8) -> SdoOutput {
        let t = self.t.as_mut().expect("transfer");
        let total = seg_count(t.data.len());
        let count = (total - start).min(blksize as usize);
        let mut frames = Vec::with_capacity(count);
        for k in 0..count {
            let i = start + k;
            let from = (i * 7).min(t.data.len());
            let to = (from + 7).min(t.data.len());
            let mut b = [0u8; 8];
            b[0] = (k as u8 + 1) | if i == total - 1 { 0x80 } else { 0 };
            b[1..1 + (to - from)].copy_from_slice(&t.data[from..to]);
            frames.push(CanFrame::new(COB_SDO_RX + t.node as u32, &b));
        }
        t.phase = Phase::BlkDnSub { crc, start, sent: count as u8 };
        let mut it = frames.into_iter();
        SdoOutput { tx: it.next(), burst: it.collect(), done: None }
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
