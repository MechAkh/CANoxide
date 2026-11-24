//! DCF save, compare and restore (CiA 306).
//!
//! Save: read every readable object of the assigned EDS over SDO (one at a
//! time; an abort is recorded and the read goes on) and write a DCF: the
//! EDS text with a `ParameterValue` per value read, a
//! `[DeviceComissioning]` section (that spelling, as CiA 306 has it) and
//! updated file info. Compare: read the node and list the differences.
//! Restore: write the `ParameterValue` (optionally also `DefaultValue`) of
//! each writable object; read-only objects and objects without a value are
//! skipped; PDO communication and mapping objects go through the CiA 301
//! sequence of `canopen::pdo::plan_apply`. All three are `SdoTask`s, so
//! the bus worker and `session::Session` run them alike.

use crate::canopen::pdo::{configs_with, pdo_of_index, plan_apply, PdoConfig, PdoKind};
use crate::canopen::sdo::SdoOutcome;
use crate::canopen::task::{ReadsReport, ReadsTask, SdoCall, SdoStep, SdoTask, StepsReport, StepsTask, TaskResult};
use crate::canopen::types::{self, le_u64, DataType, Value};
use crate::eds::{value_bytes, AccessType, ObjectDictionary};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// "save" as the u32 of 0x1010 (store parameters).
pub const STORE_SIGNATURE: u32 = 0x6576_6173;
/// "load" as the u32 of 0x1011 (restore default parameters).
pub const RESTORE_SIGNATURE: u32 = 0x6461_6F6C;

/// The `[DeviceComissioning]` values of a DCF.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DcfCommissioning {
    pub node_id: u8,
    pub baudrate_kbit: u32,
    pub node_name: String,
    pub net_number: u32,
    pub network_name: String,
}

/// A value as DCF text that `eds::value_bytes` reads back to the same bytes.
pub fn value_text(dt: DataType, b: &[u8]) -> Option<String> {
    use DataType::*;
    match dt {
        VisibleString => Some(String::from_utf8_lossy(b).trim_end_matches('\0').to_string()),
        UnicodeString => match types::decode(dt, b).ok()? {
            Value::Str(s) => Some(s),
            _ => None,
        },
        OctetString | Domain | TimeOfDay | TimeDifference => Some(b.iter().map(|x| format!("{x:02X}")).collect()),
        Boolean => Some(if b.first().copied().unwrap_or(0) != 0 { "1" } else { "0" }.to_string()),
        Real32 if b.len() >= 4 => Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string()),
        Real64 if b.len() >= 8 => Some(f64::from_le_bytes(b[..8].try_into().ok()?).to_string()),
        _ if dt.is_integer() => {
            let n = dt.size()?;
            if b.len() < n {
                return None;
            }
            Some(format!("0x{:0w$X}", le_u64(&b[..n]), w = n * 2))
        }
        _ => None,
    }
}

fn section_key(name: &str, od: &ObjectDictionary) -> Option<(u16, u8)> {
    let up = name.trim().to_ascii_uppercase();
    let hex4 = |s: &str| s.len() == 4 && s.chars().all(|c| c.is_ascii_hexdigit());
    if hex4(&up) {
        let ix = u16::from_str_radix(&up, 16).ok()?;
        return od.objects.get(&ix).filter(|o| o.is_var()).map(|_| (ix, 0));
    }
    if up.len() > 7 && hex4(&up[..4]) && up[4..].starts_with("SUB") {
        let ix = u16::from_str_radix(&up[..4], 16).ok()?;
        let sub = u8::from_str_radix(&up[7..], 16).ok()?;
        return Some((ix, sub));
    }
    None
}

fn key_of(line: &str) -> Option<String> {
    let l = line.trim();
    if l.starts_with(';') || l.starts_with('[') {
        return None;
    }
    l.find('=').map(|i| l[..i].trim().to_ascii_lowercase())
}

/// Build the DCF text: the EDS text with `ParameterValue` lines for
/// `values` ((index, sub) -> text), file info and commissioning. Values of
/// objects without their own section (compact sub-objects) are not stored;
/// their keys are returned.
pub fn build_dcf(
    eds_text: &str,
    od: &ObjectDictionary,
    values: &BTreeMap<(u16, u8), String>,
    comm: &DcfCommissioning,
    file_name: &str,
    now_unix: u64,
) -> (String, Vec<(u16, u8)>) {
    // Split into sections: (header, body lines).
    let mut sections: Vec<(Option<String>, Vec<String>)> = vec![(None, Vec::new())];
    for line in eds_text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.contains(']') {
            sections.push((Some(t[1..t.find(']').unwrap_or(t.len())].trim().to_string()), Vec::new()));
        } else {
            if let Some(s) = sections.last_mut() {
                s.1.push(line.to_string());
            }
        }
    }
    let mut stored: BTreeSet<(u16, u8)> = BTreeSet::new();
    let time = crate::canopen::pdo::unix_time_text(now_unix);
    // "YYYY-MM-DD hh:mm:ss UTC" -> DCF date "MM-DD-YYYY" and time "hh:mmAM".
    let (date, clock) = {
        let y = &time[0..4];
        let m = &time[5..7];
        let d = &time[8..10];
        let h: u32 = time[11..13].parse().unwrap_or(0);
        let mi = &time[14..16];
        let (h12, ap) = match h {
            0 => (12, "AM"),
            1..=11 => (h, "AM"),
            12 => (12, "PM"),
            _ => (h - 12, "PM"),
        };
        (format!("{m}-{d}-{y}"), format!("{h12:02}:{mi}{ap}"))
    };
    let mut has_comm = false;
    for (head, body) in sections.iter_mut() {
        let Some(name) = head else { continue };
        let up = name.to_ascii_uppercase();
        if up == "FILEINFO" {
            let set = |body: &mut Vec<String>, key: &str, v: &str| {
                let lk = key.to_ascii_lowercase();
                body.retain(|l| key_of(l).as_deref() != Some(lk.as_str()));
                let at = body.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(body.len());
                body.insert(at, format!("{key}={v}"));
            };
            let last_eds = od.file_info_value("FileName").unwrap_or("").to_string();
            set(body, "FileName", file_name);
            if !last_eds.is_empty() && !od.is_dcf {
                set(body, "LastEDS", &last_eds);
            }
            set(body, "ModifiedBy", crate::PRODUCT_NAME);
            set(body, "ModificationTime", &clock);
            set(body, "ModificationDate", &date);
            continue;
        }
        if up == "DEVICECOMISSIONING" || up == "DEVICECOMMISSIONING" {
            has_comm = true;
            *name = "DeviceComissioning".to_string();
            *body = commissioning_lines(comm);
            continue;
        }
        let Some(key) = section_key(name, od) else { continue };
        let Some(v) = values.get(&key) else { continue };
        body.retain(|l| key_of(l).as_deref() != Some("parametervalue"));
        let at = body.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(body.len());
        body.insert(at, format!("ParameterValue={v}"));
        stored.insert(key);
    }
    if !has_comm {
        let mut lines = commissioning_lines(comm);
        lines.push(String::new());
        sections.push((Some("DeviceComissioning".to_string()), lines));
    }
    let mut out = String::new();
    for (head, body) in &sections {
        if let Some(h) = head {
            out.push_str(&format!("[{h}]\r\n"));
        }
        for l in body {
            out.push_str(l);
            out.push_str("\r\n");
        }
    }
    let not_stored = values.keys().filter(|k| !stored.contains(k)).copied().collect();
    (out, not_stored)
}

fn commissioning_lines(c: &DcfCommissioning) -> Vec<String> {
    vec![
        format!("NodeID={}", c.node_id),
        format!("NodeName={}", c.node_name),
        format!("Baudrate={}", c.baudrate_kbit),
        format!("NetNumber={}", c.net_number),
        format!("NetworkName={}", c.network_name),
        "CANopenManager=0".to_string(),
    ]
}

/// All readable (index, sub) of a dictionary, in order.
pub fn readable_objects(od: &ObjectDictionary) -> Vec<(u16, u8)> {
    od.objects.values().flat_map(|o| o.subs.values().filter(|e| e.access.readable()).map(move |e| (o.index, e.sub))).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DcfSaveReport {
    pub node: u8,
    /// The DCF text (CRLF line endings).
    pub text: String,
    /// Values written into the DCF.
    pub values: usize,
    /// Objects that could not be read: (index, sub, error).
    pub failed: Vec<(u16, u8, String)>,
    /// Values without a section of their own (compact sub-objects).
    pub not_stored: Vec<(u16, u8)>,
    pub cancelled: bool,
}

/// DCF save as a task.
pub struct DcfSaveTask {
    reads: ReadsTask,
    od: Arc<ObjectDictionary>,
    eds_text: String,
    comm: DcfCommissioning,
    file_name: String,
}

impl DcfSaveTask {
    pub fn new(node: u8, od: Arc<ObjectDictionary>, eds_text: String, comm: DcfCommissioning, file_name: String) -> DcfSaveTask {
        let items = readable_objects(&od);
        DcfSaveTask { reads: ReadsTask::new("DCF save", node, items), od, eds_text, comm, file_name }
    }
}

/// Make the DCF from a finished read.
pub fn save_report(rep: ReadsReport, od: &ObjectDictionary, eds_text: &str, comm: &DcfCommissioning, file_name: &str) -> DcfSaveReport {
    let mut values = BTreeMap::new();
    let mut failed = Vec::new();
    for r in &rep.reads {
        match (&r.data, od.entry(r.index, r.sub).and_then(|e| e.dtype())) {
            (Some(d), Some(dt)) => {
                if let Some(t) = value_text(dt, d) {
                    values.insert((r.index, r.sub), t);
                }
            }
            (Some(_), None) => {}
            (None, _) => failed.push((r.index, r.sub, r.error.clone().unwrap_or_default())),
        }
    }
    let (text, not_stored) = build_dcf(eds_text, od, &values, comm, file_name, crate::canopen::pdo::unix_now());
    DcfSaveReport { node: rep.node, text, values: values.len() - not_stored.len(), failed, not_stored, cancelled: rep.cancelled }
}

impl SdoTask for DcfSaveTask {
    fn label(&self) -> String {
        "DCF save".to_string()
    }
    fn next(&mut self) -> Option<SdoCall> {
        self.reads.next()
    }
    fn on_outcome(&mut self, o: &SdoOutcome) {
        self.reads.on_outcome(o)
    }
    fn progress(&self) -> (usize, Option<usize>) {
        self.reads.progress()
    }
    fn cancel(&mut self) {
        self.reads.cancel()
    }
    fn into_result(self: Box<Self>) -> TaskResult {
        let s = *self;
        let rep = s.reads.into_report();
        TaskResult::DcfSave(save_report(rep, &s.od, &s.eds_text, &s.comm, &s.file_name))
    }
}

/// What happened to one object in a restore.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ObjectAction {
    Written,
    /// Dry run: it would be written.
    WouldWrite,
    Unchanged,
    Skipped { reason: String },
    Failed { error: String, abort_code: Option<u32> },
    /// Not run after a stop or cancel.
    NotRun,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectReport {
    pub index: u16,
    pub sub: u8,
    pub name: String,
    /// The DCF value text.
    pub dcf_value: Option<String>,
    /// The node value before the restore (as DCF text).
    pub node_value: Option<String>,
    #[serde(flatten)]
    pub action: ObjectAction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreOptions {
    pub dry_run: bool,
    /// Also restore objects that have only a DefaultValue.
    pub use_default_values: bool,
    pub from: Option<u16>,
    pub to: Option<u16>,
    /// Go on after a failed write (default: stop at the first error).
    pub continue_on_error: bool,
    /// Write 0x1010:01 = "save" at the end.
    pub store: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DcfRestoreReport {
    pub node: u8,
    pub dry_run: bool,
    pub objects: Vec<ObjectReport>,
    /// The PDO sequences (or the planned ones in a dry run).
    pub pdo_steps: Vec<SdoStep>,
    /// The SDO writes done, in order.
    pub steps: StepsReport,
    pub stored: Option<bool>,
    pub cancelled: bool,
    pub ok: bool,
}

impl DcfRestoreReport {
    pub fn count(&self, f: impl Fn(&ObjectAction) -> bool) -> usize {
        self.objects.iter().filter(|o| f(&o.action)).count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Difference {
    pub index: u16,
    pub sub: u8,
    pub name: String,
    pub dcf_value: String,
    /// None when the node did not answer the read.
    pub node_value: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DcfCompareReport {
    pub node: u8,
    pub compared: usize,
    pub differences: Vec<Difference>,
    pub cancelled: bool,
}

/// The value of each object of a DCF to restore or compare: (index, sub)
/// -> (bytes, text). `node_id` evaluates `$NODEID`.
fn dcf_values(dcf: &ObjectDictionary, node_id: u8, use_default: bool, from: u16, to: u16) -> BTreeMap<(u16, u8), (Vec<u8>, String)> {
    let mut m = BTreeMap::new();
    for o in dcf.objects.range(from..=to).map(|(_, o)| o) {
        for e in o.subs.values() {
            let text = match (&e.param_value, use_default) {
                (Some(v), _) => v.clone(),
                (None, true) => match &e.default {
                    Some(d) => d.clone(),
                    None => continue,
                },
                (None, false) => continue,
            };
            let Some(dt) = e.dtype() else { continue };
            if let Some(b) = value_bytes(dt, &text, node_id) {
                let t = value_text(dt, &b).unwrap_or(text);
                m.insert((o.index, e.sub), (b, t));
            }
        }
    }
    m
}

fn name_of(dcf: &ObjectDictionary, index: u16, sub: u8) -> String {
    dcf.name_of(index, sub).unwrap_or_default()
}

/// DCF compare as a task: reads every readable object that has a value in
/// the DCF; no writes.
pub struct DcfCompareTask {
    reads: ReadsTask,
    dcf: Arc<ObjectDictionary>,
    want: BTreeMap<(u16, u8), (Vec<u8>, String)>,
}

impl DcfCompareTask {
    pub fn new(node: u8, dcf: Arc<ObjectDictionary>, use_default: bool) -> DcfCompareTask {
        let want: BTreeMap<_, _> = dcf_values(&dcf, node, use_default, 0, 0xFFFF)
            .into_iter()
            .filter(|(k, _)| dcf.entry(k.0, k.1).is_some_and(|e| e.access.readable()))
            .collect();
        let items = want.keys().copied().collect();
        DcfCompareTask { reads: ReadsTask::new("DCF compare", node, items), dcf, want }
    }
}

fn compare_report(rep: &ReadsReport, dcf: &ObjectDictionary, want: &BTreeMap<(u16, u8), (Vec<u8>, String)>) -> DcfCompareReport {
    let mut differences = Vec::new();
    for r in &rep.reads {
        let Some((b, t)) = want.get(&(r.index, r.sub)) else { continue };
        let dt = dcf.entry(r.index, r.sub).and_then(|e| e.dtype());
        match &r.data {
            Some(d) if same_value(dt, d, b) => {}
            Some(d) => differences.push(Difference {
                index: r.index,
                sub: r.sub,
                name: name_of(dcf, r.index, r.sub),
                dcf_value: t.clone(),
                node_value: dt.and_then(|dt| value_text(dt, d)).or_else(|| Some(types::hex_bytes(d))),
                error: None,
            }),
            None => differences.push(Difference {
                index: r.index,
                sub: r.sub,
                name: name_of(dcf, r.index, r.sub),
                dcf_value: t.clone(),
                node_value: None,
                error: r.error.clone(),
            }),
        }
    }
    DcfCompareReport { node: rep.node, compared: rep.reads.len(), differences, cancelled: rep.cancelled }
}

/// Equal values; a string is compared without trailing NULs.
pub(crate) fn same_value(dt: Option<DataType>, a: &[u8], b: &[u8]) -> bool {
    match dt {
        Some(DataType::VisibleString) => {
            let t = |x: &[u8]| x.iter().rposition(|&c| c != 0).map(|i| x[..=i].to_vec()).unwrap_or_default();
            t(a) == t(b)
        }
        Some(dt) if dt.size().is_some() => {
            let n = dt.size().unwrap_or(0);
            a.len() >= n && b.len() >= n && a[..n] == b[..n]
        }
        _ => a == b,
    }
}

impl SdoTask for DcfCompareTask {
    fn label(&self) -> String {
        "DCF compare".to_string()
    }
    fn next(&mut self) -> Option<SdoCall> {
        self.reads.next()
    }
    fn on_outcome(&mut self, o: &SdoOutcome) {
        self.reads.on_outcome(o)
    }
    fn progress(&self) -> (usize, Option<usize>) {
        self.reads.progress()
    }
    fn cancel(&mut self) {
        self.reads.cancel()
    }
    fn into_result(self: Box<Self>) -> TaskResult {
        let s = *self;
        let rep = s.reads.into_report();
        TaskResult::DcfCompare(compare_report(&rep, &s.dcf, &s.want))
    }
}

enum Phase {
    Read(ReadsTask),
    Write(StepsTask),
    Done,
}

/// DCF restore as a task: reads the current values, plans the writes,
/// then (unless dry run) writes them.
pub struct DcfRestoreTask {
    node: u8,
    dcf: Arc<ObjectDictionary>,
    opts: RestoreOptions,
    want: BTreeMap<(u16, u8), (Vec<u8>, String)>,
    phase: Phase,
    report: DcfRestoreReport,
    /// Step index -> object position in `report.objects` (plain writes).
    step_obj: Vec<Option<usize>>,
    pdo_objects: BTreeMap<(PdoKind, u16), Vec<usize>>,
    step_pdo: Vec<Option<(PdoKind, u16)>>,
}

impl DcfRestoreTask {
    pub fn new(node: u8, dcf: Arc<ObjectDictionary>, opts: RestoreOptions) -> DcfRestoreTask {
        let from = opts.from.unwrap_or(0);
        let to = opts.to.unwrap_or(0xFFFF);
        let want = dcf_values(&dcf, node, opts.use_default_values, from, to);
        let mut report = DcfRestoreReport { node, dry_run: opts.dry_run, ok: true, ..Default::default() };
        let mut items = BTreeSet::new();
        // Objects without a value are reported as skipped.
        for o in dcf.objects.range(from..=to).map(|(_, o)| o) {
            for e in o.subs.values() {
                let key = (o.index, e.sub);
                let has = want.contains_key(&key);
                let skip = if !e.access.writable() {
                    Some(match e.access {
                        AccessType::Const => "const".to_string(),
                        _ => "read-only".to_string(),
                    })
                } else if !has {
                    Some("no value in the DCF".to_string())
                } else {
                    None
                };
                if let Some(reason) = skip {
                    if pdo_of_index(o.index).is_none() || !e.access.writable() {
                        report.objects.push(ObjectReport {
                            index: o.index,
                            sub: e.sub,
                            name: name_of(&dcf, o.index, e.sub),
                            dcf_value: want.get(&key).map(|v| v.1.clone()),
                            node_value: None,
                            action: ObjectAction::Skipped { reason },
                        });
                    }
                    continue;
                }
                if e.access.readable() {
                    items.insert(key);
                }
            }
        }
        // The current PDO configuration is needed for the sequence.
        for &(ix, _) in want.keys() {
            if let Some((_, _, _)) = pdo_of_index(ix) {
                let comm = if (0x1600..=0x17FF).contains(&ix) || (0x1A00..=0x1BFF).contains(&ix) { ix - 0x200 } else { ix };
                let map = comm + 0x200;
                for s in 0..=6u8 {
                    if dcf.entry(comm, s).is_some_and(|e| e.access.readable()) {
                        items.insert((comm, s));
                    }
                }
                if let Some(o) = dcf.objects.get(&map) {
                    for e in o.subs.values().filter(|e| e.access.readable()) {
                        items.insert((map, e.sub));
                    }
                }
            }
        }
        let reads = ReadsTask::new("DCF restore: read", node, items.into_iter().collect());
        DcfRestoreTask {
            node,
            dcf,
            opts,
            want,
            phase: Phase::Read(reads),
            report,
            step_obj: Vec::new(),
            pdo_objects: BTreeMap::new(),
            step_pdo: Vec::new(),
        }
    }

    /// Plan the writes from the read values.
    fn plan(&mut self, rep: ReadsReport) {
        let cur = |k: &(u16, u8)| rep.value(k.0, k.1).map(|d| d.to_vec());
        let mut steps: Vec<SdoStep> = Vec::new();
        let mut pdos: BTreeSet<(PdoKind, u16)> = BTreeSet::new();
        for (key, (bytes, text)) in &self.want {
            let e = match self.dcf.entry(key.0, key.1) {
                Some(e) if e.access.writable() => e,
                _ => continue,
            };
            let dt = e.dtype();
            let old = cur(key);
            let node_value = old.as_ref().and_then(|d| dt.and_then(|dt| value_text(dt, d)));
            if let Some((kind, number, _)) = pdo_of_index(key.0) {
                pdos.insert((kind, number));
                let pos = self.report.objects.len();
                self.pdo_objects.entry((kind, number)).or_default().push(pos);
                let unchanged = old.as_ref().is_some_and(|d| same_value(dt, d, bytes));
                self.report.objects.push(ObjectReport {
                    index: key.0,
                    sub: key.1,
                    name: name_of(&self.dcf, key.0, key.1),
                    dcf_value: Some(text.clone()),
                    node_value,
                    action: if unchanged { ObjectAction::Unchanged } else { ObjectAction::NotRun },
                });
                continue;
            }
            let unchanged = old.as_ref().is_some_and(|d| same_value(dt, d, bytes));
            let pos = self.report.objects.len();
            self.report.objects.push(ObjectReport {
                index: key.0,
                sub: key.1,
                name: name_of(&self.dcf, key.0, key.1),
                dcf_value: Some(text.clone()),
                node_value,
                action: if unchanged {
                    ObjectAction::Unchanged
                } else if self.opts.dry_run {
                    ObjectAction::WouldWrite
                } else {
                    ObjectAction::NotRun
                },
            });
            if !unchanged {
                steps.push(SdoStep::write(self.node, key.0, key.1, bytes.clone(), format!("write {}", name_of(&self.dcf, key.0, key.1))));
                self.step_obj.push(Some(pos));
                self.step_pdo.push(None);
            }
        }
        // PDOs: current config from the node, wanted = DCF value else current.
        for (kind, number) in pdos {
            let comm = crate::canopen::pdo::comm_index(kind, number);
            let read_get = |i: u16, s: u8| rep.value(i, s).map(|d| le_u64(d) as u32);
            let want_get = |i: u16, s: u8| self.want.get(&(i, s)).map(|(b, _)| le_u64(b) as u32).or_else(|| read_get(i, s));
            let exists = |i: u16| i == comm;
            let old: Option<PdoConfig> = configs_with(read_get, exists).into_iter().next();
            let new: Option<PdoConfig> = configs_with(want_get, exists).into_iter().next();
            let (Some(old), Some(mut new)) = (old, new) else {
                for &p in self.pdo_objects.get(&(kind, number)).into_iter().flatten() {
                    self.report.objects[p].action = ObjectAction::Skipped { reason: "PDO configuration of the node not readable".to_string() };
                }
                continue;
            };
            if !self.want.contains_key(&(comm, 6)) {
                new.sync_start = old.sync_start;
            }
            let plan = plan_apply(self.node, &old, &new);
            let changed = !plan.is_empty();
            for &p in self.pdo_objects.get(&(kind, number)).into_iter().flatten() {
                let o = &mut self.report.objects[p];
                if o.action == ObjectAction::NotRun {
                    o.action = if !changed {
                        ObjectAction::Unchanged
                    } else if self.opts.dry_run {
                        ObjectAction::WouldWrite
                    } else {
                        ObjectAction::NotRun
                    };
                }
            }
            self.report.pdo_steps.extend(plan.iter().cloned());
            for s in plan {
                steps.push(s);
                self.step_obj.push(None);
                self.step_pdo.push(Some((kind, number)));
            }
        }
        if self.opts.store && !self.opts.dry_run && !steps.is_empty() {
            steps.push(SdoStep::write(self.node, 0x1010, 1, STORE_SIGNATURE.to_le_bytes().to_vec(), "store parameters (0x1010:01 = \"save\")"));
            self.step_obj.push(None);
            self.step_pdo.push(None);
        }
        self.report.objects.sort_by_key(|o| (o.index, o.sub));
        // Positions changed by the sort: map them again.
        let pos_of: BTreeMap<(u16, u8), usize> = self.report.objects.iter().enumerate().map(|(i, o)| ((o.index, o.sub), i)).collect();
        let mut fix = Vec::new();
        for s in &steps {
            fix.push(pos_of.get(&(s.index, s.sub)).copied());
        }
        for (i, p) in self.step_obj.iter_mut().enumerate() {
            if p.is_some() {
                *p = fix[i];
            }
        }
        for v in self.pdo_objects.values_mut() {
            v.clear();
        }
        for (i, o) in self.report.objects.iter().enumerate() {
            if let Some((k, n, _)) = pdo_of_index(o.index) {
                self.pdo_objects.entry((k, n)).or_default().push(i);
            }
        }
        if self.opts.dry_run || steps.is_empty() {
            self.phase = Phase::Done;
        } else {
            self.phase = Phase::Write(StepsTask::new("DCF restore: write", steps, !self.opts.continue_on_error));
        }
    }

    fn finish_write(&mut self, rep: StepsReport) {
        let done = rep.done.len();
        for (i, r) in rep.done.iter().enumerate() {
            let action = if r.ok {
                ObjectAction::Written
            } else {
                ObjectAction::Failed { error: r.error.clone().unwrap_or_default(), abort_code: r.abort_code }
            };
            if r.step.index == 0x1010 && r.step.sub == 1 && self.step_obj.get(i).copied().flatten().is_none() && self.step_pdo.get(i).copied().flatten().is_none() {
                self.report.stored = Some(r.ok);
                continue;
            }
            if let Some(Some(p)) = self.step_obj.get(i) {
                self.report.objects[*p].action = action;
            } else if let Some(Some(pdo)) = self.step_pdo.get(i) {
                for &p in self.pdo_objects.get(pdo).into_iter().flatten() {
                    let o = &mut self.report.objects[p];
                    match (&o.action, &action) {
                        (ObjectAction::Unchanged, _) | (ObjectAction::Failed { .. }, _) => {}
                        (_, ObjectAction::Failed { .. }) => o.action = action.clone(),
                        _ => o.action = ObjectAction::Written,
                    }
                }
            }
        }
        let _ = done;
        self.report.ok = rep.ok;
        self.report.cancelled |= rep.cancelled;
        self.report.steps = rep;
    }
}

impl SdoTask for DcfRestoreTask {
    fn label(&self) -> String {
        if self.opts.dry_run {
            "DCF restore (dry run)".to_string()
        } else {
            "DCF restore".to_string()
        }
    }

    fn next(&mut self) -> Option<SdoCall> {
        loop {
            match &mut self.phase {
                Phase::Read(r) => {
                    if let Some(c) = r.next() {
                        return Some(c);
                    }
                    let Phase::Read(r) = std::mem::replace(&mut self.phase, Phase::Done) else { return None };
                    let rep = r.into_report();
                    if rep.cancelled {
                        self.report.cancelled = true;
                        self.report.ok = false;
                        return None;
                    }
                    self.plan(rep);
                }
                Phase::Write(w) => {
                    if let Some(c) = w.next() {
                        return Some(c);
                    }
                    let Phase::Write(w) = std::mem::replace(&mut self.phase, Phase::Done) else { return None };
                    let TaskResult::Steps(rep) = Box::new(w).into_result() else { return None };
                    self.finish_write(rep);
                    return None;
                }
                Phase::Done => return None,
            }
        }
    }

    fn on_outcome(&mut self, o: &SdoOutcome) {
        match &mut self.phase {
            Phase::Read(r) => r.on_outcome(o),
            Phase::Write(w) => w.on_outcome(o),
            Phase::Done => {}
        }
    }

    fn progress(&self) -> (usize, Option<usize>) {
        match &self.phase {
            Phase::Read(r) => r.progress(),
            Phase::Write(w) => w.progress(),
            Phase::Done => (1, Some(1)),
        }
    }

    fn cancel(&mut self) {
        self.report.cancelled = true;
        self.report.ok = false;
        match &mut self.phase {
            Phase::Read(r) => r.cancel(),
            Phase::Write(w) => w.cancel(),
            Phase::Done => {}
        }
    }

    fn into_result(self: Box<Self>) -> TaskResult {
        let mut s = *self;
        if let Phase::Write(w) = std::mem::replace(&mut s.phase, Phase::Done) {
            if let TaskResult::Steps(rep) = Box::new(w).into_result() {
                s.finish_write(rep);
            }
        }
        TaskResult::DcfRestore(s.report)
    }
}

