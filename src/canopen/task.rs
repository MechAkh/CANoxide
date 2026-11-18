//! Multi-step SDO work as small state machines with no I/O: a task hands
//! out one SDO request at a time and takes its outcome. The bus worker
//! (`bus`) runs tasks between user requests; `session::Session` runs them
//! to the end (CLI, tests). One request at a time, cancellable, with
//! progress.

use super::abort_text;
use super::pdo::NodePdoConfig;
use super::sdo::{SdoError, SdoOutcome, SdoRequest, TransferMode};
use crate::dcf::{DcfCompareReport, DcfRestoreReport, DcfSaveReport};
use crate::netinfo::NodeInfo;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// One SDO request of a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdoCall {
    pub node: u8,
    pub req: SdoRequest,
    pub mode: TransferMode,
}

/// The result of a finished task.
#[derive(Clone, Debug)]
pub enum TaskResult {
    PdoConfig(NodePdoConfig),
    Steps(StepsReport),
    NodeInfo(Vec<NodeInfo>),
    Reads(ReadsReport),
    DcfSave(DcfSaveReport),
    DcfRestore(DcfRestoreReport),
    DcfCompare(DcfCompareReport),
}

/// A multi-step SDO job. The driver loop: `next()` gives a request (or
/// `None` at the end), the driver runs it and passes the outcome to
/// `on_outcome()`. `cancel()` ends the task early; `into_result()` gives
/// what was done.
pub trait SdoTask: Send {
    fn label(&self) -> String;
    fn next(&mut self) -> Option<SdoCall>;
    fn on_outcome(&mut self, outcome: &SdoOutcome);
    /// (steps done, steps total when known).
    fn progress(&self) -> (usize, Option<usize>);
    fn cancel(&mut self);
    fn into_result(self: Box<Self>) -> TaskResult;
}

/// Short text of an SDO error, with the abort text.
pub fn error_text(e: &SdoError) -> String {
    e.to_string()
}

/// One write (or read) step of a planned sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdoStep {
    pub node: u8,
    pub index: u16,
    pub sub: u8,
    /// Data to download. Empty with `read: true`.
    pub data: Vec<u8>,
    /// What the step does, for the report ("set TPDO1 invalid").
    pub what: String,
}

impl SdoStep {
    pub fn write(node: u8, index: u16, sub: u8, data: Vec<u8>, what: impl Into<String>) -> SdoStep {
        SdoStep { node, index, sub, data, what: what.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    pub step: SdoStep,
    pub ok: bool,
    /// Error text (with the abort text) when the step failed.
    pub error: Option<String>,
    pub abort_code: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepsReport {
    /// The steps that ran, in order (the failed one last when it stopped).
    pub done: Vec<StepResult>,
    /// Steps not run (after a stop or a cancel).
    pub not_run: Vec<SdoStep>,
    pub ok: bool,
    pub cancelled: bool,
}

impl StepsReport {
    /// The first failed step.
    pub fn first_error(&self) -> Option<&StepResult> {
        self.done.iter().find(|s| !s.ok)
    }
}

/// Runs a list of SDO downloads in order. With `stop_on_error` it stops at
/// the first abort (the PDO sequences); else it goes on.
pub struct StepsTask {
    label: String,
    steps: VecDeque<SdoStep>,
    total: usize,
    current: Option<SdoStep>,
    stop_on_error: bool,
    report: StepsReport,
    stopped: bool,
}

impl StepsTask {
    pub fn new(label: impl Into<String>, steps: Vec<SdoStep>, stop_on_error: bool) -> StepsTask {
        let total = steps.len();
        StepsTask {
            label: label.into(),
            steps: steps.into(),
            total,
            current: None,
            stop_on_error,
            report: StepsReport { ok: true, ..Default::default() },
            stopped: false,
        }
    }

    pub fn report(&self) -> &StepsReport {
        &self.report
    }

    fn finish_report(mut self) -> StepsReport {
        self.report.not_run.extend(self.steps.drain(..));
        self.report
    }
}

impl SdoTask for StepsTask {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn next(&mut self) -> Option<SdoCall> {
        if self.stopped {
            return None;
        }
        let s = self.steps.pop_front()?;
        let call = SdoCall { node: s.node, req: SdoRequest::Download { index: s.index, sub: s.sub, data: s.data.clone() }, mode: TransferMode::Auto };
        self.current = Some(s);
        Some(call)
    }

    fn on_outcome(&mut self, o: &SdoOutcome) {
        let Some(step) = self.current.take() else { return };
        let (ok, error, abort_code) = match &o.result {
            Ok(_) => (true, None, None),
            Err(SdoError::Cancelled) => {
                self.report.cancelled = true;
                self.stopped = true;
                self.report.ok = false;
                self.report.not_run.push(step);
                return;
            }
            Err(e) => (false, Some(error_text(e)), e.abort_code()),
        };
        if !ok {
            self.report.ok = false;
            if self.stop_on_error {
                self.stopped = true;
            }
        }
        self.report.done.push(StepResult { step, ok, error, abort_code });
    }

    fn progress(&self) -> (usize, Option<usize>) {
        (self.report.done.len(), Some(self.total))
    }

    fn cancel(&mut self) {
        self.stopped = true;
        self.report.cancelled = true;
        self.report.ok = false;
        if let Some(s) = self.current.take() {
            self.report.not_run.push(s);
        }
    }

    fn into_result(self: Box<Self>) -> TaskResult {
        TaskResult::Steps(self.finish_report())
    }
}

/// One object read by a `ReadsTask`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResult {
    pub index: u16,
    pub sub: u8,
    /// The data, or the error text.
    pub data: Option<Vec<u8>>,
    pub error: Option<String>,
    pub abort_code: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadsReport {
    pub node: u8,
    pub reads: Vec<ReadResult>,
    pub cancelled: bool,
    /// Objects not read (after a cancel).
    pub not_read: Vec<(u16, u8)>,
}

impl ReadsReport {
    pub fn value(&self, index: u16, sub: u8) -> Option<&[u8]> {
        self.reads.iter().find(|r| r.index == index && r.sub == sub)?.data.as_deref()
    }
}

/// Reads a list of objects one at a time; an abort is recorded and the
/// task goes on (`od dump`, DCF save and compare).
pub struct ReadsTask {
    label: String,
    node: u8,
    items: VecDeque<(u16, u8)>,
    total: usize,
    current: Option<(u16, u8)>,
    report: ReadsReport,
    stopped: bool,
    /// Transfer mode for the reads.
    pub mode: TransferMode,
}

impl ReadsTask {
    pub fn new(label: impl Into<String>, node: u8, items: Vec<(u16, u8)>) -> ReadsTask {
        let total = items.len();
        ReadsTask {
            label: label.into(),
            node,
            items: items.into(),
            total,
            current: None,
            report: ReadsReport { node, ..Default::default() },
            stopped: false,
            mode: TransferMode::Auto,
        }
    }

    pub fn into_report(mut self) -> ReadsReport {
        self.report.not_read.extend(self.items.drain(..));
        self.report
    }
}

impl SdoTask for ReadsTask {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn next(&mut self) -> Option<SdoCall> {
        if self.stopped {
            return None;
        }
        let (index, sub) = self.items.pop_front()?;
        self.current = Some((index, sub));
        Some(SdoCall { node: self.node, req: SdoRequest::Upload { index, sub }, mode: self.mode })
    }

    fn on_outcome(&mut self, o: &SdoOutcome) {
        let Some((index, sub)) = self.current.take() else { return };
        match &o.result {
            Ok(d) => self.report.reads.push(ReadResult { index, sub, data: Some(d.clone()), error: None, abort_code: None }),
            Err(SdoError::Cancelled) => {
                self.stopped = true;
                self.report.cancelled = true;
                self.report.not_read.push((index, sub));
            }
            Err(e) => self.report.reads.push(ReadResult { index, sub, data: None, error: Some(error_text(e)), abort_code: e.abort_code() }),
        }
    }

    fn progress(&self) -> (usize, Option<usize>) {
        (self.report.reads.len(), Some(self.total))
    }

    fn cancel(&mut self) {
        self.stopped = true;
        self.report.cancelled = true;
        if let Some(c) = self.current.take() {
            self.report.not_read.push(c);
        }
    }

    fn into_result(self: Box<Self>) -> TaskResult {
        TaskResult::Reads(self.into_report())
    }
}

/// "0x06020000: Object does not exist in the object dictionary".
pub fn abort_line(code: u32) -> String {
    format!("0x{code:08X}: {}", abort_text(code))
}
