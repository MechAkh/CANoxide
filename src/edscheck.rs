//! EDS check: a conformance check of a node against its EDS. For each
//! object and sub-index of the EDS one SDO upload, one at a time,
//! cancellable, with progress. READ ONLY: the check never writes to the
//! device. The result of each entry is one of `CheckKind`; the report can be
//! written as CSV or as JUnit XML for a CI server.

use crate::canopen::sdo::{SdoError, SdoOutcome, SdoRequest, TransferMode};
use crate::canopen::task::{abort_line, error_text, SdoCall, SdoTask, TaskResult};
use crate::canopen::types::{hex_bytes, DataType};
use crate::dcf::{same_value, value_text};
use crate::eds::{value_bytes, AccessType, ObjectDictionary, OdEntry};
use crate::emcy::csv_cell;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

/// The result of one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Ok,
    /// Abort 0x06020000 (object) or 0x06090011 (sub-index) for an entry of the EDS.
    Missing,
    /// EDS access `wo` and abort 0x06010001: as expected.
    WriteOnlyOk,
    /// EDS readable and abort 0x06010001, or EDS `wo` and the read worked.
    Access,
    /// The data length does not fit the fixed-size EDS data type.
    Size,
    /// The value differs from the EDS (const entries, 0x1000, 0x1018:01..04).
    Value,
    /// Any other SDO abort.
    Abort,
    /// No answer.
    Timeout,
    /// Not read (after the time-out stop or a cancel).
    NotChecked,
}

impl CheckKind {
    pub const ALL: [CheckKind; 9] =
        [CheckKind::Ok, CheckKind::Missing, CheckKind::WriteOnlyOk, CheckKind::Access, CheckKind::Size, CheckKind::Value, CheckKind::Abort, CheckKind::Timeout, CheckKind::NotChecked];

    pub fn label(self) -> &'static str {
        match self {
            CheckKind::Ok => "ok",
            CheckKind::Missing => "missing",
            CheckKind::WriteOnlyOk => "write_only_ok",
            CheckKind::Access => "access",
            CheckKind::Size => "size",
            CheckKind::Value => "value",
            CheckKind::Abort => "abort",
            CheckKind::Timeout => "timeout",
            CheckKind::NotChecked => "not_checked",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Pass,
    Warning,
    Fail,
    /// Not checked.
    Skipped,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Pass => "pass",
            Severity::Warning => "warning",
            Severity::Fail => "fail",
            Severity::Skipped => "skipped",
        }
    }
}

/// The severity of a result: missing, access, size and time-out fail; a
/// value fails for 0x1000 and 0x1018, else it is a warning; another abort
/// is a warning.
pub fn severity(kind: CheckKind, index: u16) -> Severity {
    match kind {
        CheckKind::Ok | CheckKind::WriteOnlyOk => Severity::Pass,
        CheckKind::Missing | CheckKind::Access | CheckKind::Size | CheckKind::Timeout => Severity::Fail,
        CheckKind::Value if index == 0x1000 || index == 0x1018 => Severity::Fail,
        CheckKind::Value | CheckKind::Abort => Severity::Warning,
        CheckKind::NotChecked => Severity::Skipped,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckEntry {
    pub index: u16,
    pub sub: u8,
    pub name: String,
    /// EDS access ("ro").
    pub access: String,
    /// EDS data type name ("UNSIGNED32").
    pub data_type: Option<String>,
    pub kind: CheckKind,
    pub severity: Severity,
    pub detail: String,
    pub abort_code: Option<u32>,
}

impl CheckEntry {
    /// "1018:01".
    pub fn key(&self) -> String {
        format!("{:04X}:{:02X}", self.index, self.sub)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdsCheckOptions {
    /// Do not read DOMAIN objects (default on).
    pub skip_domain: bool,
    /// The index range (default all).
    pub from: u16,
    pub to: u16,
    /// Stop after this many time-outs in a row (0 = never); the rest is
    /// `not_checked`. Default 3.
    pub stop_after_timeouts: u32,
}

impl Default for EdsCheckOptions {
    fn default() -> Self {
        EdsCheckOptions { skip_domain: true, from: 0x0000, to: 0xFFFF, stop_after_timeouts: 3 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdsCheckReport {
    pub node: u8,
    /// The file name of the EDS.
    pub eds_file: String,
    /// Every entry in index order, the `ok` ones too.
    pub entries: Vec<CheckEntry>,
    /// DOMAIN entries not read (option "skip DOMAIN").
    pub skipped_domain: usize,
    pub duration_ms: u64,
    pub cancelled: bool,
    /// Stopped after the time-outs in a row.
    pub stopped_on_timeouts: bool,
}

impl EdsCheckReport {
    pub fn count(&self, k: CheckKind) -> usize {
        self.entries.iter().filter(|e| e.kind == k).count()
    }

    pub fn failed(&self) -> usize {
        self.entries.iter().filter(|e| e.severity == Severity::Fail).count()
    }

    pub fn warnings(&self) -> usize {
        self.entries.iter().filter(|e| e.severity == Severity::Warning).count()
    }

    /// Entries that were read (not `not_checked`).
    pub fn checked(&self) -> usize {
        self.entries.iter().filter(|e| e.kind != CheckKind::NotChecked).count()
    }

    /// No entry failed (warnings do not fail).
    pub fn passed(&self) -> bool {
        self.failed() == 0
    }

    /// "42 checked: 40 ok, 0 failed, 2 warnings (sim.eds, node 1, 0.12 s)".
    pub fn summary_line(&self) -> String {
        let ok = self.count(CheckKind::Ok) + self.count(CheckKind::WriteOnlyOk);
        let nc = self.count(CheckKind::NotChecked);
        let mut s = format!("{} checked: {ok} ok, {} failed, {} warning(s)", self.checked(), self.failed(), self.warnings());
        if nc > 0 {
            s.push_str(&format!(", {nc} not checked"));
        }
        if self.cancelled {
            s.push_str(" (cancelled)");
        } else if self.stopped_on_timeouts {
            s.push_str(" (stopped after time-outs: node gone?)");
        }
        s.push_str(&format!(" - {}, node {}, {:.2} s", self.eds_file, self.node, self.duration_ms as f64 / 1e3));
        s
    }

    /// CSV of the entries (all, or only those that are not ok).
    pub fn to_csv(&self, with_ok: bool) -> String {
        let mut s = String::from("index_sub,name,access,data_type,result,severity,detail,abort_code\n");
        for e in self.entries.iter().filter(|e| with_ok || e.severity != Severity::Pass) {
            let row = [
                e.key(),
                e.name.clone(),
                e.access.clone(),
                e.data_type.clone().unwrap_or_default(),
                e.kind.label().to_string(),
                e.severity.label().to_string(),
                e.detail.clone(),
                e.abort_code.map(|c| format!("0x{c:08X}")).unwrap_or_default(),
            ];
            s.push_str(&row.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
            s.push('\n');
        }
        s
    }

    /// JUnit XML: one testsuite, one testcase per entry, `failure` for a
    /// fail, `system-out` for a warning, `skipped` for an entry not checked.
    pub fn to_junit(&self) -> String {
        let tests = self.entries.len();
        let skipped = self.count(CheckKind::NotChecked);
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        s.push_str(&format!(
            "<testsuite name=\"{}\" tests=\"{tests}\" failures=\"{}\" errors=\"0\" skipped=\"{skipped}\" time=\"{:.3}\">\n",
            xml_escape(&format!("EDS check of node {} against {}", self.node, self.eds_file)),
            self.failed(),
            self.duration_ms as f64 / 1e3
        ));
        let class = xml_escape(&format!("eds_check.node{}", self.node));
        for e in &self.entries {
            let name = xml_escape(&format!("{} {}", e.key(), e.name));
            let what = xml_escape(&format!("{}: {}", e.kind.label(), e.detail));
            match e.severity {
                Severity::Pass => s.push_str(&format!("  <testcase classname=\"{class}\" name=\"{name}\"/>\n")),
                Severity::Fail => s.push_str(&format!(
                    "  <testcase classname=\"{class}\" name=\"{name}\">\n    <failure message=\"{what}\" type=\"{}\">{what}</failure>\n  </testcase>\n",
                    e.kind.label()
                )),
                Severity::Warning => s.push_str(&format!("  <testcase classname=\"{class}\" name=\"{name}\">\n    <system-out>warning: {what}</system-out>\n  </testcase>\n")),
                Severity::Skipped => s.push_str(&format!("  <testcase classname=\"{class}\" name=\"{name}\">\n    <skipped message=\"{what}\"/>\n  </testcase>\n")),
            }
        }
        s.push_str("</testsuite>\n");
        s
    }
}

/// Text for XML content and attributes: the five entities escaped, and the
/// characters that XML 1.0 does not allow dropped.
pub fn xml_escape(t: &str) -> String {
    let mut s = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&apos;"),
            '\t' | '\n' | '\r' => s.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => s.push(c),
        }
    }
    s
}

/// A data type with a fixed size that the size check covers.
fn fixed_size(dt: DataType) -> Option<usize> {
    let covered = dt == DataType::Boolean || dt == DataType::Real32 || dt == DataType::Real64 || dt.is_integer();
    if covered {
        dt.size()
    } else {
        None
    }
}

fn show(dt: Option<DataType>, b: &[u8]) -> String {
    dt.and_then(|t| value_text(t, b)).unwrap_or_else(|| hex_bytes(b))
}

/// The result of one read of an EDS entry (not for a cancel).
pub fn evaluate(e: &OdEntry, index: u16, sub: u8, node: u8, result: &Result<Vec<u8>, SdoError>) -> (CheckKind, String, Option<u32>) {
    let dt = e.dtype();
    match result {
        Ok(d) => {
            if e.access == AccessType::Wo {
                return (CheckKind::Access, format!("the EDS says wo, but the read worked ({} byte(s))", d.len()), None);
            }
            if let Some(n) = dt.and_then(fixed_size) {
                if d.len() != n {
                    let t = dt.map(|t| t.name()).unwrap_or("?");
                    return (CheckKind::Size, format!("{} byte(s) read, {t} has {n}", d.len()), None);
                }
            }
            let checked = (e.access == AccessType::Const && e.default.is_some()) || (index == 0x1000 && sub == 0) || (index == 0x1018 && (1..=4).contains(&sub));
            if checked {
                // A value that cannot be evaluated is not checked.
                if let (Some(t), Some(text)) = (dt, e.effective_text().filter(|x| !x.trim().is_empty())) {
                    if let Some(want) = value_bytes(t, text, node) {
                        if !same_value(dt, d, &want) {
                            return (CheckKind::Value, format!("node {}, EDS {}", show(dt, d), show(dt, &want)), None);
                        }
                    }
                }
            }
            (CheckKind::Ok, String::new(), None)
        }
        Err(SdoError::ServerAbort(c)) => {
            let c = *c;
            match c {
                0x0602_0000 => (CheckKind::Missing, format!("object not on the node ({})", abort_line(c)), Some(c)),
                0x0609_0011 => (CheckKind::Missing, format!("sub-index not on the node ({})", abort_line(c)), Some(c)),
                0x0601_0001 if e.access == AccessType::Wo => (CheckKind::WriteOnlyOk, "write only, as the EDS says".to_string(), Some(c)),
                0x0601_0001 => (CheckKind::Access, format!("the EDS says {}, the node refuses the read ({})", e.access.label(), abort_line(c)), Some(c)),
                _ => (CheckKind::Abort, abort_line(c), Some(c)),
            }
        }
        Err(SdoError::Timeout) => (CheckKind::Timeout, "no answer".to_string(), None),
        Err(other) => (CheckKind::Abort, error_text(other), other.abort_code()),
    }
}

/// The check as an `SdoTask`: one SDO upload per EDS entry, never a write.
pub struct EdsCheckTask {
    node: u8,
    od: Arc<ObjectDictionary>,
    opts: EdsCheckOptions,
    items: VecDeque<(u16, u8)>,
    total: usize,
    current: Option<(u16, u8)>,
    timeouts_in_row: u32,
    started: Option<Instant>,
    report: EdsCheckReport,
    stopped: bool,
}

impl EdsCheckTask {
    pub fn new(node: u8, od: Arc<ObjectDictionary>, eds_file: impl Into<String>, opts: EdsCheckOptions) -> EdsCheckTask {
        let mut items = VecDeque::new();
        let mut skipped_domain = 0;
        for (ix, o) in od.objects.range(opts.from..=opts.to.max(opts.from)) {
            for (sub, e) in &o.subs {
                if opts.skip_domain && e.dtype() == Some(DataType::Domain) {
                    skipped_domain += 1;
                    continue;
                }
                items.push_back((*ix, *sub));
            }
        }
        let total = items.len();
        EdsCheckTask {
            node,
            od,
            opts,
            items,
            total,
            current: None,
            timeouts_in_row: 0,
            started: None,
            report: EdsCheckReport { node, eds_file: eds_file.into(), skipped_domain, ..Default::default() },
            stopped: false,
        }
    }

    fn entry(&self, index: u16, sub: u8, kind: CheckKind, detail: String, abort_code: Option<u32>) -> CheckEntry {
        let e = self.od.entry(index, sub);
        CheckEntry {
            index,
            sub,
            name: self.od.name_of(index, sub).unwrap_or_default(),
            access: e.map(|e| e.access.label().to_string()).unwrap_or_default(),
            data_type: e.and_then(|e| e.dtype()).map(|t| t.name().to_string()),
            kind,
            severity: severity(kind, index),
            detail,
            abort_code,
        }
    }

    /// The report so far; the rest of the entries as not checked.
    pub fn into_report(mut self) -> EdsCheckReport {
        let why = if self.report.cancelled { "cancelled" } else { "stopped after time-outs in a row" };
        let rest: Vec<(u16, u8)> = self.current.take().into_iter().chain(self.items.drain(..)).collect();
        for (i, s) in rest {
            let e = self.entry(i, s, CheckKind::NotChecked, why.to_string(), None);
            self.report.entries.push(e);
        }
        self.report.entries.sort_by_key(|e| (e.index, e.sub));
        self.report.duration_ms = self.started.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
        self.report
    }
}

impl SdoTask for EdsCheckTask {
    fn label(&self) -> String {
        "EDS check".to_string()
    }

    fn next(&mut self) -> Option<SdoCall> {
        self.started.get_or_insert_with(Instant::now);
        if self.stopped {
            return None;
        }
        let (index, sub) = self.items.pop_front()?;
        self.current = Some((index, sub));
        Some(SdoCall { node: self.node, req: SdoRequest::Upload { index, sub }, mode: TransferMode::Auto })
    }

    fn on_outcome(&mut self, o: &SdoOutcome) {
        let Some((index, sub)) = self.current.take() else { return };
        if matches!(o.result, Err(SdoError::Cancelled)) {
            self.current = Some((index, sub));
            self.cancel();
            return;
        }
        let Some(e) = self.od.entry(index, sub) else { return };
        let (kind, detail, code) = evaluate(e, index, sub, self.node, &o.result);
        if kind == CheckKind::Timeout {
            self.timeouts_in_row += 1;
        } else {
            self.timeouts_in_row = 0;
        }
        let entry = self.entry(index, sub, kind, detail, code);
        self.report.entries.push(entry);
        let n = self.opts.stop_after_timeouts;
        if n > 0 && self.timeouts_in_row >= n && !self.items.is_empty() {
            self.stopped = true;
            self.report.stopped_on_timeouts = true;
        }
    }

    fn progress(&self) -> (usize, Option<usize>) {
        (self.report.entries.len(), Some(self.total))
    }

    fn cancel(&mut self) {
        self.stopped = true;
        self.report.cancelled = true;
    }

    fn into_result(self: Box<Self>) -> TaskResult {
        TaskResult::EdsCheck(self.into_report())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::sim::{builtin_dictionary, SimNode};
    use crate::eds;
    use crate::session::{Ctl, Session};

    const EDS: &str = "[1000]\nParameterName=Device type\nObjectType=0x7\nDataType=0x0007\nAccessType=ro\nDefaultValue=0x00000191\n\n\
[1008]\nParameterName=Name\nObjectType=0x7\nDataType=0x0009\nAccessType=const\nDefaultValue=Box\n\n\
[2000]\nParameterName=Plain\nObjectType=0x7\nDataType=0x0007\nAccessType=rw\n\n\
[2001]\nParameterName=Cmd\nObjectType=0x7\nDataType=0x0005\nAccessType=wo\n\n\
[2002]\nParameterName=Const node\nObjectType=0x7\nDataType=0x0005\nAccessType=const\nDefaultValue=$NODEID+1\n\n\
[2003]\nParameterName=Block\nObjectType=0x7\nDataType=0x000F\nAccessType=rw\n";

    fn od() -> Arc<ObjectDictionary> {
        Arc::new(eds::parse_str(EDS))
    }

    /// Drive the task with a fake SDO server: `answer` gives the result of
    /// each upload.
    fn run(task: &mut EdsCheckTask, answer: impl Fn(u16, u8) -> Result<Vec<u8>, SdoError>) {
        while let Some(c) = task.next() {
            let SdoRequest::Upload { index, sub } = c.req else { panic!("the EDS check must only read") };
            let o = SdoOutcome { node: c.node, index, sub, upload: true, result: answer(index, sub) };
            task.on_outcome(&o);
        }
    }

    fn kind_of(r: &EdsCheckReport, index: u16, sub: u8) -> CheckKind {
        r.entries.iter().find(|e| e.index == index && e.sub == sub).map(|e| e.kind).unwrap()
    }

    #[test]
    fn every_result_kind() {
        let e = |a: &str, dt: u16, default: Option<&str>| OdEntry {
            sub: 0,
            name: "x".into(),
            data_type: dt,
            access: AccessType::parse(a).unwrap(),
            default: default.map(str::to_string),
            param_value: None,
            low: None,
            high: None,
            pdo_mapping: false,
        };
        let ab = |c: u32| Err(SdoError::ServerAbort(c));
        let ro32 = e("ro", 0x0007, None);
        let wo8 = e("wo", 0x0005, None);
        assert_eq!(evaluate(&ro32, 0x2000, 0, 1, &Ok(vec![0; 4])).0, CheckKind::Ok);
        assert_eq!(evaluate(&ro32, 0x2000, 0, 1, &ab(0x0602_0000)).0, CheckKind::Missing);
        assert_eq!(evaluate(&ro32, 0x2000, 1, 1, &ab(0x0609_0011)).0, CheckKind::Missing);
        assert_eq!(evaluate(&wo8, 0x2001, 0, 1, &ab(0x0601_0001)).0, CheckKind::WriteOnlyOk);
        assert_eq!(evaluate(&ro32, 0x2000, 0, 1, &ab(0x0601_0001)).0, CheckKind::Access);
        assert_eq!(evaluate(&wo8, 0x2001, 0, 1, &Ok(vec![0])).0, CheckKind::Access);
        assert_eq!(evaluate(&ro32, 0x2000, 0, 1, &Ok(vec![0; 2])).0, CheckKind::Size);
        // No size check for a string or a DOMAIN.
        assert_eq!(evaluate(&e("ro", 0x0009, None), 0x2000, 0, 1, &Ok(vec![1, 2, 3])).0, CheckKind::Ok);
        let c = e("const", 0x0005, Some("$NODEID+1"));
        assert_eq!(evaluate(&c, 0x2002, 0, 4, &Ok(vec![5])).0, CheckKind::Ok);
        let (k, detail, _) = evaluate(&c, 0x2002, 0, 4, &Ok(vec![6]));
        assert_eq!(k, CheckKind::Value);
        assert!(detail.contains("0x06") && detail.contains("0x05"), "{detail}");
        assert_eq!(severity(k, 0x2002), Severity::Warning);
        // 0x1000 and 0x1018 are checked although ro, and fail.
        let dt = e("ro", 0x0007, Some("0x191"));
        assert_eq!(evaluate(&dt, 0x1000, 0, 1, &Ok(0x192u32.to_le_bytes().to_vec())).0, CheckKind::Value);
        assert_eq!(severity(CheckKind::Value, 0x1000), Severity::Fail);
        assert_eq!(severity(CheckKind::Value, 0x1018), Severity::Fail);
        // An rw entry with a default is not value checked.
        assert_eq!(evaluate(&e("rw", 0x0005, Some("3")), 0x2000, 0, 1, &Ok(vec![9])).0, CheckKind::Ok);
        // A value that cannot be evaluated is skipped.
        assert_eq!(evaluate(&e("const", 0x0005, Some("$FOO")), 0x2000, 0, 1, &Ok(vec![9])).0, CheckKind::Ok);
        let (k, d, code) = evaluate(&ro32, 0x2000, 0, 1, &ab(0x0800_0000));
        assert_eq!((k, code), (CheckKind::Abort, Some(0x0800_0000)));
        assert!(d.starts_with("0x08000000"), "{d}");
        assert_eq!(severity(k, 0x2000), Severity::Warning);
        assert_eq!(evaluate(&ro32, 0x2000, 0, 1, &Err(SdoError::Timeout)).0, CheckKind::Timeout);
        assert_eq!(severity(CheckKind::Timeout, 0x2000), Severity::Fail);
    }

    #[test]
    fn task_with_a_fake_server() {
        let mut t = EdsCheckTask::new(3, od(), "test.eds", EdsCheckOptions::default());
        run(&mut t, |i, _| match i {
            0x1000 => Ok(0x191u32.to_le_bytes().to_vec()),
            0x1008 => Ok(b"Box".to_vec()),
            0x2000 => Err(SdoError::ServerAbort(0x0602_0000)),
            0x2001 => Err(SdoError::ServerAbort(0x0601_0001)),
            0x2002 => Ok(vec![4]),
            _ => panic!("DOMAIN read although skipped"),
        });
        let r = t.into_report();
        assert_eq!(r.skipped_domain, 1);
        assert_eq!(kind_of(&r, 0x1000, 0), CheckKind::Ok);
        assert_eq!(kind_of(&r, 0x2000, 0), CheckKind::Missing);
        assert_eq!(kind_of(&r, 0x2001, 0), CheckKind::WriteOnlyOk);
        assert_eq!(kind_of(&r, 0x2002, 0), CheckKind::Ok);
        assert_eq!(r.failed(), 1);
        assert!(!r.passed());
        assert!(r.summary_line().contains("1 failed"), "{}", r.summary_line());
    }

    #[test]
    fn stops_after_timeouts_in_a_row() {
        let mut t = EdsCheckTask::new(3, od(), "test.eds", EdsCheckOptions { stop_after_timeouts: 2, ..Default::default() });
        run(&mut t, |_, _| Err(SdoError::Timeout));
        let r = t.into_report();
        assert!(r.stopped_on_timeouts);
        assert_eq!(r.count(CheckKind::Timeout), 2);
        assert_eq!(r.count(CheckKind::NotChecked), 3);
        assert_eq!(r.checked(), 2);
    }

    #[test]
    fn cancel_and_range() {
        let mut t = EdsCheckTask::new(3, od(), "test.eds", EdsCheckOptions { from: 0x2000, to: 0x2001, skip_domain: false, ..Default::default() });
        assert_eq!(t.progress(), (0, Some(2)));
        let c = t.next().unwrap();
        assert_eq!(c.req, SdoRequest::Upload { index: 0x2000, sub: 0 });
        t.cancel();
        assert!(t.next().is_none());
        let r = t.into_report();
        assert!(r.cancelled);
        assert_eq!(r.count(CheckKind::NotChecked), 2);
    }

    #[test]
    fn junit_is_well_formed_and_escaped() {
        let mut t = EdsCheckTask::new(3, od(), "a<b>&\"c\".eds", EdsCheckOptions::default());
        run(&mut t, |i, _| match i {
            0x1000 => Ok(0x191u32.to_le_bytes().to_vec()),
            0x1008 => Ok(b"Other".to_vec()),
            0x2000 => Err(SdoError::Timeout),
            _ => Err(SdoError::ServerAbort(0x0601_0001)),
        });
        let r = t.into_report();
        let x = r.to_junit();
        assert!(x.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite "));
        assert!(x.contains("a&lt;b&gt;&amp;&quot;c&quot;.eds"), "{x}");
        assert!(!x.contains("a<b>"));
        assert_eq!(x.matches("<testcase ").count(), r.entries.len());
        assert_eq!(x.matches("<failure ").count(), r.failed());
        assert_eq!(x.matches("<system-out>").count(), r.warnings());
        assert!(x.trim_end().ends_with("</testsuite>"));
        // Every testcase is closed or self-closing.
        let self_closed = x.lines().filter(|l| l.trim_start().starts_with("<testcase ") && l.ends_with("/>")).count();
        assert_eq!(x.matches("<testcase ").count(), x.matches("</testcase>").count() + self_closed);
        assert_eq!(xml_escape("x\u{1}y'z"), "xy&apos;z");
        let csv = r.to_csv(false);
        assert!(csv.lines().count() >= 3);
    }

    #[test]
    fn builtin_simulator_has_no_fail() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        let od = Arc::new(builtin_dictionary());
        let task = EdsCheckTask::new(1, od, "sim_builtin.eds", EdsCheckOptions::default());
        let r = match s.run_task(Box::new(task), &mut Ctl::default()).unwrap() {
            TaskResult::EdsCheck(r) => r,
            _ => panic!(),
        };
        let bad: Vec<String> = r.entries.iter().filter(|e| e.severity != Severity::Pass).map(|e| format!("{} {} {}", e.key(), e.kind.label(), e.detail)).collect();
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(r.count(CheckKind::WriteOnlyOk), 1);
        assert!(r.passed());
        assert_eq!(s.sim_node().unwrap().sdo_writes, 0, "the check must not write");
    }
}
