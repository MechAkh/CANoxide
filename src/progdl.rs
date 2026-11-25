//! Program download to a node with a CANopen bootloader (CiA 302-3):
//! read the program identification 0x1F56, stop the program (0x1F51 = 0),
//! optionally clear it (0x1F51 = 3), download the file to the program data
//! 0x1F50 (a DOMAIN), wait while the flash status 0x1F57 says "in
//! progress", check its error code, start the program (0x1F51 = 1), wait
//! for the boot-up message and read 0x1F56 again.
//!
//! The sequence is a background task (`ProgDlTask`), driven by the bus
//! worker (GUI) or by `Session::run_task` (CLI, tests). Each step can be
//! switched off. A failed step stops the sequence; nothing is retried.

use crate::can::CanFrame;
use crate::canopen::sdo::{SdoError, SdoOutcome, SdoRequest, TransferMode};
use crate::canopen::task::{abort_line, error_text, SdoCall, SdoTask, TaskResult};
use crate::canopen::COB_HEARTBEAT;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub const PROGRAM_DATA: u16 = 0x1F50;
pub const PROGRAM_CONTROL: u16 = 0x1F51;
pub const PROGRAM_ID: u16 = 0x1F56;
pub const FLASH_STATUS: u16 = 0x1F57;

/// Values of the program control 0x1F51.
pub const CONTROL_STOP: u8 = 0;
pub const CONTROL_START: u8 = 1;
pub const CONTROL_RESET: u8 = 2;
pub const CONTROL_CLEAR: u8 = 3;

/// Flash status bit 0: the flash is in progress.
pub const FLASH_IN_PROGRESS: u32 = 1;

/// The SDO abort codes that say "the object does not exist".
const NO_OBJECT: [u32; 2] = [0x0602_0000, 0x0609_0011];

// ------------------------------------------------------------------ checksums

/// CRC-32 (IEEE 802.3, reflected, polynomial 0xEDB88320, the zip / PNG CRC).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// SHA-256 (FIPS 180-4) of the data.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, c) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*c);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// SHA-256 as 64 lowercase hex digits.
pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The text of a flash status error code (bits 1..7 of 0x1F57), after the
/// codes that bootloaders publish for CiA 302-3.
pub fn flash_error_text(code: u8) -> &'static str {
    match code {
        0 => "no error",
        1 => "no valid program",
        2 => "data format unknown",
        3 => "data format error or checksum error",
        4 => "flash not cleared before write",
        5 => "flash write error",
        6 => "general address error",
        7 => "flash secured (write access denied)",
        _ => "manufacturer-specific error",
    }
}

/// The error code of a flash status value.
pub fn flash_error_code(status: u32) -> u8 {
    ((status >> 1) & 0x7F) as u8
}

// ------------------------------------------------------------------ the steps

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgStep {
    ReadOldId,
    Stop,
    Clear,
    Download,
    WaitFlash,
    CheckError,
    Start,
    WaitBoot,
    ReadNewId,
}

impl ProgStep {
    pub const ALL: [ProgStep; 9] = [
        ProgStep::ReadOldId,
        ProgStep::Stop,
        ProgStep::Clear,
        ProgStep::Download,
        ProgStep::WaitFlash,
        ProgStep::CheckError,
        ProgStep::Start,
        ProgStep::WaitBoot,
        ProgStep::ReadNewId,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ProgStep::ReadOldId => "Read the program identification (0x1F56)",
            ProgStep::Stop => "Stop the program (0x1F51 = 0)",
            ProgStep::Clear => "Clear the program (0x1F51 = 3)",
            ProgStep::Download => "Download the file (0x1F50)",
            ProgStep::WaitFlash => "Wait for the flash (0x1F57)",
            ProgStep::CheckError => "Check the flash error code",
            ProgStep::Start => "Start the program (0x1F51 = 1)",
            ProgStep::WaitBoot => "Wait for the boot-up message",
            ProgStep::ReadNewId => "Read the new program identification",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Running,
    Ok,
    Skipped,
    Failed,
    Cancelled,
    NotRun,
}

impl StepState {
    pub fn name(self) -> &'static str {
        match self {
            StepState::Pending => "pending",
            StepState::Running => "running",
            StepState::Ok => "ok",
            StepState::Skipped => "skipped",
            StepState::Failed => "failed",
            StepState::Cancelled => "cancelled",
            StepState::NotRun => "not run",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepReport {
    pub step: ProgStep,
    pub state: StepState,
    /// What happened (a value, the reason for a skip, the error text).
    pub detail: Option<String>,
    pub abort_code: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgDlReport {
    pub node: u8,
    pub program: u8,
    /// Size of the file in bytes.
    pub size: usize,
    pub crc32: u32,
    pub steps: Vec<StepReport>,
    pub ok: bool,
    pub cancelled: bool,
    pub old_id: Option<u32>,
    pub new_id: Option<u32>,
    pub flash_status: Option<u32>,
    pub bytes_done: usize,
    /// True when the task has ended.
    pub finished: bool,
}

impl ProgDlReport {
    pub fn step(&self, s: ProgStep) -> &StepReport {
        &self.steps[ProgStep::ALL.iter().position(|x| *x == s).unwrap_or(0)]
    }

    /// The failed step, if any.
    pub fn failed(&self) -> Option<&StepReport> {
        self.steps.iter().find(|s| s.state == StepState::Failed)
    }
}

/// The settings of a program download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgDlOptions {
    pub node: u8,
    /// Program number = sub-index of 0x1F50, 0x1F51, 0x1F56, 0x1F57.
    pub program: u8,
    pub mode: TransferMode,
    pub read_old_id: bool,
    pub stop: bool,
    pub clear: bool,
    pub download: bool,
    pub wait_flash: bool,
    pub check_error: bool,
    pub start: bool,
    pub wait_boot: bool,
    pub read_new_id: bool,
    pub flash_timeout_ms: u64,
    pub boot_timeout_ms: u64,
    pub poll_period_ms: u64,
}

impl ProgDlOptions {
    pub fn new(node: u8) -> ProgDlOptions {
        ProgDlOptions {
            node,
            program: 1,
            mode: TransferMode::Auto,
            read_old_id: true,
            stop: true,
            clear: false,
            download: true,
            wait_flash: true,
            check_error: true,
            start: true,
            wait_boot: true,
            read_new_id: true,
            flash_timeout_ms: 30_000,
            boot_timeout_ms: 10_000,
            poll_period_ms: 100,
        }
    }

    pub fn enabled(&self, s: ProgStep) -> bool {
        match s {
            ProgStep::ReadOldId => self.read_old_id,
            ProgStep::Stop => self.stop,
            ProgStep::Clear => self.clear,
            ProgStep::Download => self.download,
            ProgStep::WaitFlash => self.wait_flash,
            ProgStep::CheckError => self.check_error,
            ProgStep::Start => self.start,
            // The boot-up follows a start only.
            ProgStep::WaitBoot => self.wait_boot && self.start,
            ProgStep::ReadNewId => self.read_new_id,
        }
    }

    pub fn set(&mut self, s: ProgStep, on: bool) {
        match s {
            ProgStep::ReadOldId => self.read_old_id = on,
            ProgStep::Stop => self.stop = on,
            ProgStep::Clear => self.clear = on,
            ProgStep::Download => self.download = on,
            ProgStep::WaitFlash => self.wait_flash = on,
            ProgStep::CheckError => self.check_error = on,
            ProgStep::Start => self.start = on,
            ProgStep::WaitBoot => self.wait_boot = on,
            ProgStep::ReadNewId => self.read_new_id = on,
        }
    }

    /// One line per step that would run, for a dry run.
    pub fn plan(&self, size: usize) -> Vec<String> {
        let p = self.program;
        ProgStep::ALL
            .iter()
            .filter(|s| self.enabled(**s))
            .map(|s| match s {
                ProgStep::Download => format!("{} sub {p}: {size} bytes ({:?} transfer)", s.name(), self.mode),
                ProgStep::WaitFlash => format!("{} sub {p}, time-out {} ms", s.name(), self.flash_timeout_ms),
                ProgStep::WaitBoot => format!("{}, time-out {} ms", s.name(), self.boot_timeout_ms),
                _ => format!("{} sub {p}", s.name()),
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    /// Poll 0x1F57 at `next_ms` (None: at once); fail at `deadline`.
    Flash { next_ms: Option<u64>, deadline: Option<u64> },
    /// Wait for the boot-up until `deadline`.
    Boot { deadline: Option<u64> },
}

/// The program download as an SDO task.
pub struct ProgDlTask {
    opts: ProgDlOptions,
    data: Vec<u8>,
    idx: usize,
    in_flight: bool,
    wait: Option<Wait>,
    now: u64,
    stopped: bool,
    flash_missing: bool,
    start_sent: bool,
    boot_seen: bool,
    report: ProgDlReport,
    shared: Option<Arc<Mutex<ProgDlReport>>>,
}

impl ProgDlTask {
    pub fn new(opts: ProgDlOptions, data: Vec<u8>) -> ProgDlTask {
        let report = ProgDlReport {
            node: opts.node,
            program: opts.program,
            size: data.len(),
            crc32: crc32(&data),
            steps: ProgStep::ALL.iter().map(|s| StepReport { step: *s, state: StepState::Pending, detail: None, abort_code: None }).collect(),
            ok: false,
            cancelled: false,
            old_id: None,
            new_id: None,
            flash_status: None,
            bytes_done: 0,
            finished: false,
        };
        let mut t = ProgDlTask {
            opts,
            data,
            idx: 0,
            in_flight: false,
            wait: None,
            now: 0,
            stopped: false,
            flash_missing: false,
            start_sent: false,
            boot_seen: false,
            report,
            shared: None,
        };
        t.prepare();
        t
    }

    /// A copy of the report that the task keeps up to date (for a live
    /// step list).
    pub fn shared(&mut self) -> Arc<Mutex<ProgDlReport>> {
        let a = Arc::new(Mutex::new(self.report.clone()));
        self.shared = Some(a.clone());
        a
    }

    pub fn report(&self) -> &ProgDlReport {
        &self.report
    }

    fn publish(&self) {
        if let Some(s) = &self.shared {
            if let Ok(mut g) = s.lock() {
                *g = self.report.clone();
            }
        }
    }

    fn step(&self) -> Option<ProgStep> {
        ProgStep::ALL.get(self.idx).copied()
    }

    fn set(&mut self, state: StepState, detail: Option<String>, abort_code: Option<u32>) {
        if let Some(r) = self.report.steps.get_mut(self.idx) {
            r.state = state;
            r.detail = detail;
            r.abort_code = abort_code;
        }
    }

    /// The current step is done: go to the next one.
    fn done(&mut self, state: StepState, detail: Option<String>) {
        self.set(state, detail, None);
        self.idx += 1;
        self.wait = None;
        self.prepare();
    }

    /// The current step failed: stop the sequence.
    fn fail(&mut self, detail: String, abort_code: Option<u32>) {
        self.set(StepState::Failed, Some(detail), abort_code);
        self.stop_rest();
        self.report.ok = false;
    }

    fn stop_rest(&mut self) {
        self.stopped = true;
        self.wait = None;
        for r in self.report.steps.iter_mut().skip(self.idx + 1) {
            if r.state == StepState::Pending {
                r.state = StepState::NotRun;
            }
        }
    }

    /// Skip the steps that are off; set up a wait step; evaluate a known
    /// flash status.
    fn prepare(&mut self) {
        while let Some(s) = self.step() {
            if !self.opts.enabled(s) {
                let why = if s == ProgStep::WaitBoot && self.opts.wait_boot { "the program is not started" } else { "switched off" };
                self.set(StepState::Skipped, Some(why.to_string()), None);
                self.idx += 1;
                continue;
            }
            match s {
                ProgStep::WaitFlash if self.flash_missing => {
                    self.set(StepState::Skipped, Some("0x1F57 does not exist".into()), None);
                    self.idx += 1;
                    continue;
                }
                ProgStep::WaitFlash => self.wait = Some(Wait::Flash { next_ms: None, deadline: None }),
                ProgStep::CheckError if self.flash_missing => {
                    self.set(StepState::Skipped, Some("0x1F57 does not exist".into()), None);
                    self.idx += 1;
                    continue;
                }
                ProgStep::CheckError => {
                    if let Some(st) = self.report.flash_status.filter(|st| st & FLASH_IN_PROGRESS == 0) {
                        if !self.check(st) {
                            return;
                        }
                        self.idx += 1;
                        continue;
                    }
                }
                ProgStep::WaitBoot => {
                    if self.boot_seen {
                        self.set(StepState::Ok, Some("boot-up received".into()), None);
                        self.idx += 1;
                        continue;
                    }
                    self.wait = Some(Wait::Boot { deadline: None });
                }
                _ => {}
            }
            self.set(StepState::Running, None, None);
            break;
        }
        if self.step().is_none() && !self.stopped {
            self.report.ok = true;
        }
    }

    /// Evaluate a flash status for the error check. False: failed.
    fn check(&mut self, status: u32) -> bool {
        if status & FLASH_IN_PROGRESS != 0 {
            self.fail(format!("status 0x{status:08X}: the flash is still in progress"), None);
            return false;
        }
        let code = flash_error_code(status);
        if code != 0 {
            self.fail(format!("status 0x{status:08X}: error {code}, {}", flash_error_text(code)), None);
            return false;
        }
        self.set(StepState::Ok, Some(format!("status 0x{status:08X}: no error")), None);
        true
    }

    fn sub(&self) -> u8 {
        self.opts.program
    }

    fn call(&self, req: SdoRequest) -> SdoCall {
        SdoCall { node: self.opts.node, req, mode: TransferMode::Auto }
    }

    fn le32(d: &[u8]) -> u32 {
        let mut b = [0u8; 4];
        let n = d.len().min(4);
        b[..n].copy_from_slice(&d[..n]);
        u32::from_le_bytes(b)
    }
}

impl SdoTask for ProgDlTask {
    fn label(&self) -> String {
        format!("Program download node {}", self.opts.node)
    }

    fn next(&mut self) -> Option<SdoCall> {
        if self.stopped || self.in_flight {
            return None;
        }
        let s = self.step()?;
        let sub = self.sub();
        let call = match s {
            ProgStep::ReadOldId | ProgStep::ReadNewId => self.call(SdoRequest::Upload { index: PROGRAM_ID, sub }),
            ProgStep::Stop => self.call(SdoRequest::Download { index: PROGRAM_CONTROL, sub, data: vec![CONTROL_STOP] }),
            ProgStep::Clear => self.call(SdoRequest::Download { index: PROGRAM_CONTROL, sub, data: vec![CONTROL_CLEAR] }),
            ProgStep::Start => {
                self.start_sent = true;
                self.boot_seen = false;
                self.call(SdoRequest::Download { index: PROGRAM_CONTROL, sub, data: vec![CONTROL_START] })
            }
            ProgStep::Download => SdoCall { node: self.opts.node, req: SdoRequest::Download { index: PROGRAM_DATA, sub, data: self.data.clone() }, mode: self.opts.mode },
            ProgStep::WaitFlash | ProgStep::CheckError => self.call(SdoRequest::Upload { index: FLASH_STATUS, sub }),
            ProgStep::WaitBoot => return None,
        };
        self.in_flight = true;
        self.publish();
        Some(call)
    }

    fn on_outcome(&mut self, o: &SdoOutcome) {
        if !self.in_flight {
            return;
        }
        self.in_flight = false;
        let Some(s) = self.step() else { return };
        let res = match &o.result {
            Err(SdoError::Cancelled) => {
                self.cancel();
                return;
            }
            r => r,
        };
        match (s, res) {
            (ProgStep::ReadOldId, Ok(d)) => {
                let v = Self::le32(d);
                self.report.old_id = Some(v);
                self.done(StepState::Ok, Some(format!("0x{v:08X}")));
            }
            (ProgStep::ReadNewId, Ok(d)) => {
                let v = Self::le32(d);
                self.report.new_id = Some(v);
                let was = self.report.old_id.map(|o| format!(" (was 0x{o:08X})")).unwrap_or_default();
                self.done(StepState::Ok, Some(format!("0x{v:08X}{was}")));
            }
            (ProgStep::Stop | ProgStep::Clear, Ok(_)) => self.done(StepState::Ok, None),
            (ProgStep::Start, Ok(_)) => self.done(StepState::Ok, None),
            // A bootloader may jump to the program before it answers.
            (ProgStep::Start, Err(SdoError::Timeout)) if self.opts.enabled(ProgStep::WaitBoot) => {
                self.done(StepState::Ok, Some("no SDO answer (the node may have restarted at once)".into()))
            }
            (ProgStep::Download, Ok(_)) => {
                self.report.bytes_done = self.data.len();
                let n = self.data.len();
                self.done(StepState::Ok, Some(format!("{n} bytes, CRC-32 0x{:08X}", self.report.crc32)));
            }
            (ProgStep::WaitFlash, Ok(d)) => {
                let st = Self::le32(d);
                self.report.flash_status = Some(st);
                if st & FLASH_IN_PROGRESS == 0 {
                    self.done(StepState::Ok, Some(format!("status 0x{st:08X}")));
                } else {
                    self.wait = Some(Wait::Flash { next_ms: Some(self.now + self.opts.poll_period_ms.max(1)), deadline: match self.wait {
                        Some(Wait::Flash { deadline, .. }) => deadline,
                        _ => None,
                    } });
                }
            }
            (ProgStep::WaitFlash | ProgStep::CheckError, Err(e)) if e.abort_code().is_some_and(|c| NO_OBJECT.contains(&c)) => {
                self.flash_missing = true;
                let c = e.abort_code().unwrap_or(0);
                self.set(StepState::Skipped, Some(format!("0x1F57 does not exist ({})", abort_line(c))), Some(c));
                self.idx += 1;
                self.wait = None;
                self.prepare();
            }
            (ProgStep::CheckError, Ok(d)) => {
                let st = Self::le32(d);
                self.report.flash_status = Some(st);
                if self.check(st) {
                    self.idx += 1;
                    self.prepare();
                }
            }
            (_, Err(e)) => self.fail(error_text(e), e.abort_code()),
            (ProgStep::WaitBoot, Ok(_)) => {}
        }
        self.publish();
    }

    fn progress(&self) -> (usize, Option<usize>) {
        (self.report.bytes_done, Some(self.data.len()))
    }

    fn cancel(&mut self) {
        if self.stopped && !self.in_flight {
            // Already ended (failed or cancelled).
            return;
        }
        self.in_flight = false;
        self.report.cancelled = true;
        self.report.ok = false;
        if self.step().is_some() {
            self.set(StepState::Cancelled, Some("cancelled".into()), None);
        }
        self.stop_rest();
        self.publish();
    }

    fn into_result(mut self: Box<Self>) -> TaskResult {
        if !self.stopped && self.step().is_some() {
            // Ended by the driver before the last step.
            self.stop_rest();
            self.set(StepState::NotRun, None, None);
            self.report.ok = false;
        }
        self.report.finished = true;
        self.publish();
        TaskResult::ProgramDownload(self.report)
    }

    fn on_time(&mut self, now_ms: u64) {
        self.now = now_ms;
        match self.wait {
            Some(Wait::Flash { next_ms, deadline }) => {
                let deadline = deadline.unwrap_or(now_ms + self.opts.flash_timeout_ms);
                self.wait = Some(Wait::Flash { next_ms, deadline: Some(deadline) });
                if now_ms >= deadline && !self.in_flight {
                    let st = self.report.flash_status.map(|s| format!(", last status 0x{s:08X}")).unwrap_or_default();
                    self.fail(format!("time-out: the flash was still in progress after {} ms{st}", self.opts.flash_timeout_ms), None);
                    self.publish();
                }
            }
            Some(Wait::Boot { deadline }) => {
                let deadline = deadline.unwrap_or(now_ms + self.opts.boot_timeout_ms);
                self.wait = Some(Wait::Boot { deadline: Some(deadline) });
                if now_ms >= deadline {
                    self.fail(format!("time-out: no boot-up message within {} ms", self.opts.boot_timeout_ms), None);
                    self.publish();
                }
            }
            None => {}
        }
    }

    fn on_frame(&mut self, f: &CanFrame) {
        if !self.start_sent || f.extended || f.rtr || f.id != COB_HEARTBEAT + self.opts.node as u32 || f.payload() != [0] {
            return;
        }
        self.boot_seen = true;
        if matches!(self.wait, Some(Wait::Boot { .. })) && !self.stopped {
            self.done(StepState::Ok, Some("boot-up received".into()));
            self.publish();
        }
    }

    fn waiting(&self) -> bool {
        if self.stopped || self.in_flight {
            return false;
        }
        match self.wait {
            Some(Wait::Flash { next_ms, .. }) => next_ms.is_some_and(|t| self.now < t),
            Some(Wait::Boot { .. }) => true,
            None => false,
        }
    }

    fn on_sdo_progress(&mut self, done: usize, _total: Option<usize>) {
        if self.step() == Some(ProgStep::Download) && self.in_flight {
            self.report.bytes_done = done;
            self.publish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::sim::SimNode;
    use crate::session::{Ctl, Session};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(sha256_hex(long), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    }

    fn image(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + 3) as u8).collect()
    }

    fn run(s: &mut Session, opts: ProgDlOptions, data: Vec<u8>) -> ProgDlReport {
        let r = s.run_task(Box::new(ProgDlTask::new(opts, data)), &mut Ctl::default()).unwrap();
        let TaskResult::ProgramDownload(r) = r else { panic!("wrong result") };
        r
    }

    fn states(r: &ProgDlReport) -> Vec<StepState> {
        r.steps.iter().map(|s| s.state).collect()
    }

    #[test]
    fn good_run_on_the_simulator() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        let data = image(3000);
        let crc = crc32(&data);
        let r = run(&mut s, ProgDlOptions::new(1), data.clone());
        assert!(r.ok, "{r:#?}");
        use StepState::*;
        assert_eq!(states(&r), vec![Ok, Ok, Skipped, Ok, Ok, Ok, Ok, Ok, Ok]);
        assert_eq!(r.new_id, Some(crc));
        assert_ne!(r.old_id, r.new_id);
        assert_eq!(r.bytes_done, 3000);
        assert!(r.finished);
        let n = s.sim_node().unwrap();
        assert_eq!(n.program_data(1), Some(&data[..]));
        assert!(n.program_running());
    }

    #[test]
    fn block_transfer_and_clear() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        let mut o = ProgDlOptions::new(1);
        o.mode = TransferMode::Block;
        o.clear = true;
        let r = run(&mut s, o, image(5000));
        assert!(r.ok, "{r:#?}");
        assert_eq!(r.step(ProgStep::Clear).state, StepState::Ok);
        assert_eq!(r.new_id, Some(crc32(&image(5000))));
    }

    #[test]
    fn node_not_stopped_is_refused() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        let mut o = ProgDlOptions::new(1);
        o.stop = false;
        let r = run(&mut s, o, image(100));
        assert!(!r.ok);
        let f = r.failed().unwrap();
        assert_eq!(f.step, ProgStep::Download);
        assert_eq!(f.abort_code, Some(0x0800_0022));
        assert_eq!(r.step(ProgStep::Stop).state, StepState::Skipped);
        assert_eq!(r.step(ProgStep::Start).state, StepState::NotRun);
        assert!(s.sim_node().unwrap().program_running(), "the program still runs");
    }

    #[test]
    fn flash_error_stops_before_the_start() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        s.sim_node().unwrap().set_flash_error(Some(3));
        let r = run(&mut s, ProgDlOptions::new(1), image(200));
        assert!(!r.ok);
        let f = r.failed().unwrap();
        assert_eq!(f.step, ProgStep::CheckError);
        assert!(f.detail.as_deref().unwrap_or("").contains("checksum"), "{f:?}");
        assert_eq!(r.step(ProgStep::Start).state, StepState::NotRun);
        assert!(!s.sim_node().unwrap().program_running());
    }

    #[test]
    fn cancel_before_the_download() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        let stop = AtomicBool::new(true);
        let r = s.run_task(Box::new(ProgDlTask::new(ProgDlOptions::new(1), image(100))), &mut Ctl { cancel: Some(&stop), progress: None }).unwrap();
        let TaskResult::ProgramDownload(r) = r else { panic!() };
        assert!(r.cancelled && !r.ok);
        assert_eq!(r.step(ProgStep::ReadOldId).state, StepState::Cancelled);
        assert!(r.steps.iter().skip(1).all(|x| matches!(x.state, StepState::NotRun | StepState::Skipped)), "{r:#?}");
        assert!(s.sim_node().unwrap().program_running());
    }

    #[test]
    fn cancel_during_the_transfer() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        let stop = AtomicBool::new(false);
        let mut t = ProgDlTask::new(ProgDlOptions::new(1), image(20_000));
        let shared = t.shared();
        // Cancel as soon as the download has moved some bytes.
        let stop_ref = &stop;
        let mut cancel_on_bytes = move |d: usize, _n: Option<usize>| {
            if d > 0 {
                stop_ref.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        };
        let r = s.run_task(Box::new(t), &mut Ctl { cancel: Some(&stop), progress: Some(&mut cancel_on_bytes) });
        let TaskResult::ProgramDownload(r) = r.unwrap() else { panic!() };
        assert!(r.cancelled, "{r:#?}");
        assert_eq!(r.step(ProgStep::Download).state, StepState::Cancelled);
        assert!(r.bytes_done > 0 && r.bytes_done < 20_000, "{}", r.bytes_done);
        assert_eq!(r.step(ProgStep::Start).state, StepState::NotRun);
        assert!(shared.lock().unwrap().finished);
    }

    #[test]
    fn each_step_can_be_skipped() {
        for step in ProgStep::ALL {
            let mut s = Session::sim(SimNode::builtin(1), false);
            let mut o = ProgDlOptions::new(1);
            if step == ProgStep::Clear {
                o.clear = true;
            }
            o.set(step, false);
            // Without a stop the node refuses the data: download nothing, and
            // the start of a running program sends no boot-up.
            if step == ProgStep::Stop {
                o.download = false;
                o.wait_boot = false;
            }
            // Without the wait the error check must see a finished flash.
            if step == ProgStep::WaitFlash {
                s.sim_node().unwrap().set_flash_ms(0);
            }
            let r = run(&mut s, o, image(64));
            assert_eq!(r.step(step).state, StepState::Skipped, "{step:?}: {r:#?}");
            assert!(r.ok, "{step:?}: {r:#?}");
            if step == ProgStep::Start {
                assert_eq!(r.step(ProgStep::WaitBoot).state, StepState::Skipped);
                assert!(!s.sim_node().unwrap().program_running());
            }
        }
    }

    #[test]
    fn missing_flash_status_is_skipped() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        s.sim_node().unwrap().hide_object(FLASH_STATUS);
        let r = run(&mut s, ProgDlOptions::new(1), image(64));
        assert!(r.ok, "{r:#?}");
        assert_eq!(r.step(ProgStep::WaitFlash).state, StepState::Skipped);
        assert_eq!(r.step(ProgStep::CheckError).state, StepState::Skipped);
    }

    #[test]
    fn boot_time_out() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        s.sim_node().unwrap().set_boot_on_start(false);
        let mut o = ProgDlOptions::new(1);
        o.boot_timeout_ms = 200;
        let r = run(&mut s, o, image(64));
        assert!(!r.ok);
        assert_eq!(r.failed().unwrap().step, ProgStep::WaitBoot);
        assert_eq!(r.step(ProgStep::ReadNewId).state, StepState::NotRun);
    }

    #[test]
    fn plan_lists_the_enabled_steps() {
        let mut o = ProgDlOptions::new(5);
        o.start = false;
        let p = o.plan(10);
        assert_eq!(p.len(), 6, "{p:?}");
        assert!(p.iter().any(|l| l.contains("10 bytes")));
    }
}
