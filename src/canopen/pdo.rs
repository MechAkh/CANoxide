//! PDO communication and mapping parameters, and bit packing.

use super::sdo::{SdoError, SdoOutcome, SdoRequest, TransferMode};
use super::task::{SdoCall, SdoStep, SdoTask, TaskResult};
use crate::eds::{eval_int, ObjectDictionary};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PdoKind {
    /// Sent by the node (0x1800 / 0x1A00).
    Tpdo,
    /// Received by the node (0x1400 / 0x1600).
    Rpdo,
}

impl PdoKind {
    pub fn label(self) -> &'static str {
        match self {
            PdoKind::Tpdo => "TPDO",
            PdoKind::Rpdo => "RPDO",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapEntry {
    pub index: u16,
    pub sub: u8,
    pub bits: u8,
}

impl MapEntry {
    pub fn from_u32(v: u32) -> MapEntry {
        MapEntry { index: (v >> 16) as u16, sub: (v >> 8) as u8, bits: v as u8 }
    }
    pub fn to_u32(self) -> u32 {
        ((self.index as u32) << 16) | ((self.sub as u32) << 8) | self.bits as u32
    }
    /// Dummy mapping entries (data type indices 0x0001..0x0007) are padding.
    pub fn is_dummy(self) -> bool {
        (0x0001..=0x0007).contains(&self.index)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdoConfig {
    pub kind: PdoKind,
    /// 1-based PDO number.
    pub number: u16,
    /// 11-bit or 29-bit identifier (flag bits removed).
    pub cob_id: u32,
    /// Bit 31 of the COB-ID entry clear.
    pub valid: bool,
    /// Bit 29 of the COB-ID entry set.
    pub extended: bool,
    /// Bit 30 of the COB-ID entry clear (RTR allowed).
    #[serde(default)]
    pub rtr_allowed: bool,
    pub trans_type: u8,
    /// Inhibit time in units of 100 us (sub 3).
    #[serde(default)]
    pub inhibit_100us: u16,
    pub event_timer_ms: u16,
    /// SYNC start value (sub 6), when the object has it.
    #[serde(default)]
    pub sync_start: Option<u8>,
    pub mapping: Vec<MapEntry>,
}

impl PdoConfig {
    /// The value of sub 1 of the communication object.
    pub fn raw_cob_id(&self) -> u32 {
        let mask = if self.extended { 0x1FFF_FFFF } else { 0x7FF };
        let mut v = self.cob_id & mask;
        if !self.valid {
            v |= 0x8000_0000;
        }
        if !self.rtr_allowed {
            v |= 0x4000_0000;
        }
        if self.extended {
            v |= 0x2000_0000;
        }
        v
    }
    /// Index of the communication parameter object (0x1400+n / 0x1800+n).
    pub fn comm_index(&self) -> u16 {
        comm_index(self.kind, self.number)
    }
    /// Index of the mapping parameter object (0x1600+n / 0x1A00+n).
    pub fn map_index(&self) -> u16 {
        self.comm_index() + 0x200
    }
    /// "tpdo1" / "rpdo3".
    pub fn short_name(&self) -> String {
        format!("{}{}", if self.kind == PdoKind::Tpdo { "tpdo" } else { "rpdo" }, self.number)
    }
    pub fn label(&self) -> String {
        format!("{}{}", self.kind.label(), self.number)
    }
    pub fn total_bits(&self) -> u32 {
        self.mapping.iter().map(|m| m.bits as u32).sum()
    }
}

fn read_u32(od: &ObjectDictionary, index: u16, sub: u8, node_id: u8) -> Option<u32> {
    let e = od.entry(index, sub)?;
    eval_int(e.effective_text()?, node_id).map(|v| v as u32)
}

/// Index of the communication object of a PDO (1-based number).
pub fn comm_index(kind: PdoKind, number: u16) -> u16 {
    let base = if kind == PdoKind::Tpdo { 0x1800 } else { 0x1400 };
    base + number.saturating_sub(1)
}

/// The PDO of a communication or mapping object index.
pub fn pdo_of_index(index: u16) -> Option<(PdoKind, u16, bool)> {
    match index {
        0x1400..=0x15FF => Some((PdoKind::Rpdo, index - 0x1400 + 1, false)),
        0x1600..=0x17FF => Some((PdoKind::Rpdo, index - 0x1600 + 1, true)),
        0x1800..=0x19FF => Some((PdoKind::Tpdo, index - 0x1800 + 1, false)),
        0x1A00..=0x1BFF => Some((PdoKind::Tpdo, index - 0x1A00 + 1, true)),
        _ => None,
    }
}

/// Parse "tpdo1", "RPDO12" (1-based number, 1..512).
pub fn parse_pdo_name(s: &str) -> Option<(PdoKind, u16)> {
    let l = s.trim().to_ascii_lowercase();
    let (kind, rest) = if let Some(r) = l.strip_prefix("tpdo") {
        (PdoKind::Tpdo, r)
    } else {
        (PdoKind::Rpdo, l.strip_prefix("rpdo")?)
    };
    let n: u16 = rest.parse().ok()?;
    (1..=512).contains(&n).then_some((kind, n))
}

/// PDO configurations from the defaults (or DCF values) of a dictionary.
pub fn configs_from_od(od: &ObjectDictionary, node_id: u8) -> Vec<PdoConfig> {
    configs_with(|index, sub| read_u32(od, index, sub, node_id), |index| od.objects.contains_key(&index))
}

/// PDO configurations from any value source (the simulator uses its live
/// values). `get(index, sub)` gives a value; `exists(index)` an object.
pub fn configs_with(get: impl Fn(u16, u8) -> Option<u32>, exists: impl Fn(u16) -> bool) -> Vec<PdoConfig> {
    let mut v = Vec::new();
    for (kind, comm_base, map_base) in [(PdoKind::Rpdo, 0x1400u16, 0x1600u16), (PdoKind::Tpdo, 0x1800, 0x1A00)] {
        for n in 0..512u16 {
            let comm = comm_base + n;
            if !exists(comm) {
                continue;
            }
            let Some(raw) = get(comm, 1) else { continue };
            let count = get(map_base + n, 0).unwrap_or(0).min(64) as u8;
            let mapping = (1..=count).filter_map(|s| get(map_base + n, s)).map(MapEntry::from_u32).collect();
            let extended = raw & 0x2000_0000 != 0;
            v.push(PdoConfig {
                kind,
                number: n + 1,
                cob_id: raw & if extended { 0x1FFF_FFFF } else { 0x7FF },
                valid: raw & 0x8000_0000 == 0,
                extended,
                rtr_allowed: raw & 0x4000_0000 == 0,
                trans_type: get(comm, 2).unwrap_or(255) as u8,
                inhibit_100us: get(comm, 3).unwrap_or(0) as u16,
                event_timer_ms: get(comm, 5).unwrap_or(0) as u16,
                sync_start: get(comm, 6).map(|v| v as u8),
                mapping,
            });
        }
    }
    v
}

/// Read `bits` bits starting at bit `offset` (little endian, LSB first).
pub fn extract_bits(data: &[u8], offset: u32, bits: u32) -> Option<u64> {
    if bits == 0 || bits > 64 || offset + bits > (data.len() as u32) * 8 {
        return None;
    }
    let mut v: u64 = 0;
    for i in 0..bits {
        let bit = offset + i;
        let b = (data[(bit / 8) as usize] >> (bit % 8)) & 1;
        v |= (b as u64) << i;
    }
    Some(v)
}

/// Write `bits` bits of `value` at bit `offset`. Returns false when it does not fit.
pub fn insert_bits(data: &mut [u8], offset: u32, bits: u32, value: u64) -> bool {
    if bits == 0 || bits > 64 || offset + bits > (data.len() as u32) * 8 {
        return false;
    }
    for i in 0..bits {
        let bit = offset + i;
        let byte = &mut data[(bit / 8) as usize];
        let mask = 1u8 << (bit % 8);
        if (value >> i) & 1 != 0 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
    true
}

/// Default number of consecutive missing PDO objects that ends the scan of
/// one direction.
pub const DEFAULT_PDO_SCAN_STOP: u16 = 4;
/// Most PDOs per direction (CiA 301).
pub const MAX_PDOS: u16 = 512;

/// The PDO configuration read from a node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodePdoConfig {
    pub node: u8,
    /// Read time, seconds since 1970-01-01 UTC.
    pub read_time: u64,
    /// RPDOs first, then TPDOs, by number.
    pub pdos: Vec<PdoConfig>,
    /// False when the read was cancelled or ended by a timeout.
    pub complete: bool,
    /// Errors met on the way (a timeout, an unexpected abort).
    pub errors: Vec<String>,
}

impl NodePdoConfig {
    pub fn get(&self, kind: PdoKind, number: u16) -> Option<&PdoConfig> {
        self.pdos.iter().find(|p| p.kind == kind && p.number == number)
    }
    /// The read time as "YYYY-MM-DD hh:mm:ss UTC".
    pub fn read_time_text(&self) -> String {
        unix_time_text(self.read_time)
    }
}

/// Seconds since 1970 as "YYYY-MM-DD hh:mm:ss UTC".
pub fn unix_time_text(t: u64) -> String {
    let days = (t / 86400) as i64;
    let rem = t % 86400;
    // Civil date from days (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rem / 3600, (rem / 60) % 60, rem % 60)
}

/// Seconds since 1970 now.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PdoReadOptions {
    /// Consecutive "object does not exist" answers that end one direction.
    pub stop_after_missing: u16,
    /// Highest PDO count per direction (at most 512).
    pub max: u16,
    /// Read only this PDO.
    pub only: Option<(PdoKind, u16)>,
}

impl Default for PdoReadOptions {
    fn default() -> Self {
        PdoReadOptions { stop_after_missing: DEFAULT_PDO_SCAN_STOP, max: MAX_PDOS, only: None }
    }
}

/// Reads the PDO configuration of a node over SDO (an `SdoTask`).
pub struct PdoReadTask {
    node: u8,
    opts: PdoReadOptions,
    /// 0 = RPDO, 1 = TPDO, 2 = done.
    dir: u8,
    n: u16,
    end: u16,
    missing: u16,
    queue: VecDeque<(u16, u8)>,
    current: Option<(u16, u8)>,
    values: BTreeMap<(u16, u8), u32>,
    found: BTreeSet<u16>,
    errors: Vec<String>,
    stopped: bool,
    cancelled: bool,
    timed_out: bool,
    steps: usize,
}

impl PdoReadTask {
    pub fn new(node: u8, opts: PdoReadOptions) -> PdoReadTask {
        let mut t = PdoReadTask {
            node,
            opts,
            dir: 0,
            n: 0,
            end: opts.max.clamp(1, MAX_PDOS),
            missing: 0,
            queue: VecDeque::new(),
            current: None,
            values: BTreeMap::new(),
            found: BTreeSet::new(),
            errors: Vec::new(),
            stopped: false,
            cancelled: false,
            timed_out: false,
            steps: 0,
        };
        if let Some((kind, number)) = opts.only {
            t.dir = if kind == PdoKind::Rpdo { 0 } else { 1 };
            t.n = number.clamp(1, MAX_PDOS) - 1;
            t.end = t.n + 1;
        }
        t
    }

    fn base(&self) -> u16 {
        if self.dir == 0 {
            0x1400
        } else {
            0x1800
        }
    }

    fn advance(&mut self) -> bool {
        loop {
            if self.dir > 1 {
                return false;
            }
            let stop = self.opts.stop_after_missing.max(1);
            if self.n >= self.end || self.missing >= stop {
                if self.opts.only.is_some() {
                    self.dir = 2;
                    return false;
                }
                self.dir += 1;
                self.n = 0;
                self.missing = 0;
                continue;
            }
            self.queue.push_back((self.base() + self.n, 0));
            self.n += 1;
            return true;
        }
    }

    pub fn result(&self) -> NodePdoConfig {
        let found = &self.found;
        let pdos = configs_with(|i, s| self.values.get(&(i, s)).copied(), |i| found.contains(&i));
        NodePdoConfig {
            node: self.node,
            read_time: unix_now(),
            pdos,
            complete: !self.cancelled && !self.timed_out,
            errors: self.errors.clone(),
        }
    }
}

impl SdoTask for PdoReadTask {
    fn label(&self) -> String {
        format!("read PDO configuration of node {}", self.node)
    }

    fn next(&mut self) -> Option<SdoCall> {
        if self.stopped {
            return None;
        }
        if self.queue.is_empty() && !self.advance() {
            return None;
        }
        let (index, sub) = self.queue.pop_front()?;
        self.current = Some((index, sub));
        Some(SdoCall { node: self.node, req: SdoRequest::Upload { index, sub }, mode: TransferMode::Auto })
    }

    fn on_outcome(&mut self, o: &SdoOutcome) {
        let Some((index, sub)) = self.current.take() else { return };
        self.steps += 1;
        let is_comm = (0x1400..=0x15FF).contains(&index) || (0x1800..=0x19FF).contains(&index);
        match &o.result {
            Ok(d) => {
                let v = super::types::le_u64(d) as u32;
                self.values.insert((index, sub), v);
                if is_comm && sub == 0 {
                    self.found.insert(index);
                    self.missing = 0;
                    let count = v as u8;
                    self.queue.push_back((index, 1));
                    self.queue.push_back((index, 2));
                    for s in [3u8, 5, 6] {
                        if count >= s {
                            self.queue.push_back((index, s));
                        }
                    }
                    self.queue.push_back((index + 0x200, 0));
                } else if !is_comm && sub == 0 {
                    for s in 1..=(v.min(64) as u8) {
                        self.queue.push_back((index, s));
                    }
                }
            }
            Err(SdoError::Cancelled) => {
                self.cancelled = true;
                self.stopped = true;
            }
            Err(SdoError::Timeout) => {
                self.errors.push(format!("0x{index:04X}:{sub:02X}: timeout, no answer; read stopped"));
                self.timed_out = true;
                self.stopped = true;
            }
            Err(e) => {
                let missing = matches!(e, SdoError::ServerAbort(0x0602_0000) | SdoError::ServerAbort(0x0609_0011));
                if is_comm && sub == 0 {
                    self.missing += 1;
                    if !missing {
                        self.errors.push(format!("0x{index:04X}:{sub:02X}: {e}"));
                    }
                } else if !(missing && sub > 2) {
                    self.errors.push(format!("0x{index:04X}:{sub:02X}: {e}"));
                }
            }
        }
    }

    fn progress(&self) -> (usize, Option<usize>) {
        (self.steps, None)
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.stopped = true;
    }

    fn into_result(self: Box<Self>) -> TaskResult {
        TaskResult::PdoConfig(self.result())
    }
}

/// A change of one PDO; `None` keeps the value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdoChange {
    /// New identifier (above 0x7FF: a 29-bit identifier).
    pub cob_id: Option<u32>,
    pub valid: Option<bool>,
    pub trans_type: Option<u8>,
    pub inhibit_100us: Option<u16>,
    pub event_timer_ms: Option<u16>,
    pub sync_start: Option<u8>,
    pub mapping: Option<Vec<MapEntry>>,
}

impl PdoChange {
    pub fn apply(&self, cfg: &PdoConfig) -> PdoConfig {
        let mut c = cfg.clone();
        if let Some(id) = self.cob_id {
            c.extended = id > 0x7FF;
            c.cob_id = id & 0x1FFF_FFFF;
        }
        if let Some(v) = self.valid {
            c.valid = v;
        }
        if let Some(v) = self.trans_type {
            c.trans_type = v;
        }
        if let Some(v) = self.inhibit_100us {
            c.inhibit_100us = v;
        }
        if let Some(v) = self.event_timer_ms {
            c.event_timer_ms = v;
        }
        if let Some(v) = self.sync_start {
            c.sync_start = Some(v);
        }
        if let Some(m) = &self.mapping {
            c.mapping = m.clone();
        }
        c
    }
}

/// The CiA 301 SDO sequence that changes a PDO from `old` to `new`: set
/// the COB-ID invalid, for a mapping change sub 0 = 0, the entries, sub 0
/// = count, the communication parameters, then the final COB-ID (with the
/// valid bit of `new`). Steps that change nothing are left out.
pub fn plan_apply(node: u8, old: &PdoConfig, new: &PdoConfig) -> Vec<SdoStep> {
    let comm = new.comm_index();
    let map = new.map_index();
    let label = new.label();
    let mut steps = Vec::new();
    let mapping_changed = old.mapping != new.mapping;
    let other_changed = mapping_changed
        || old.trans_type != new.trans_type
        || old.inhibit_100us != new.inhibit_100us
        || old.event_timer_ms != new.event_timer_ms
        || (new.sync_start.is_some() && old.sync_start != new.sync_start)
        || old.cob_id != new.cob_id
        || old.extended != new.extended
        || old.rtr_allowed != new.rtr_allowed;
    let mut cur = old.raw_cob_id();
    if old.valid && (other_changed || !new.valid) {
        cur |= 0x8000_0000;
        steps.push(SdoStep::write(node, comm, 1, cur.to_le_bytes().to_vec(), format!("set {label} invalid")));
    }
    if mapping_changed {
        steps.push(SdoStep::write(node, map, 0, vec![0], format!("{label}: clear the mapping (sub 0 = 0)")));
        for (i, m) in new.mapping.iter().enumerate() {
            let s = i as u8 + 1;
            steps.push(SdoStep::write(
                node,
                map,
                s,
                m.to_u32().to_le_bytes().to_vec(),
                format!("{label}: map entry {s} = 0x{:04X}:{:02X}, {} bits", m.index, m.sub, m.bits),
            ));
        }
        steps.push(SdoStep::write(node, map, 0, vec![new.mapping.len() as u8], format!("{label}: {} mapped objects", new.mapping.len())));
    }
    if old.trans_type != new.trans_type {
        steps.push(SdoStep::write(node, comm, 2, vec![new.trans_type], format!("{label}: transmission type {}", new.trans_type)));
    }
    if old.inhibit_100us != new.inhibit_100us {
        steps.push(SdoStep::write(node, comm, 3, new.inhibit_100us.to_le_bytes().to_vec(), format!("{label}: inhibit time {} x 100 us", new.inhibit_100us)));
    }
    if old.event_timer_ms != new.event_timer_ms {
        steps.push(SdoStep::write(node, comm, 5, new.event_timer_ms.to_le_bytes().to_vec(), format!("{label}: event timer {} ms", new.event_timer_ms)));
    }
    if let Some(s) = new.sync_start {
        if old.sync_start != Some(s) {
            steps.push(SdoStep::write(node, comm, 6, vec![s], format!("{label}: SYNC start value {s}")));
        }
    }
    let fin = new.raw_cob_id();
    if fin != cur {
        let what = if new.valid {
            format!("{label}: COB-ID 0x{:X}, valid", new.cob_id)
        } else {
            format!("{label}: COB-ID 0x{:X}, invalid", new.cob_id)
        };
        steps.push(SdoStep::write(node, comm, 1, fin.to_le_bytes().to_vec(), what));
    }
    steps
}

/// Enable or disable one PDO.
pub fn plan_enable(node: u8, cfg: &PdoConfig, enable: bool) -> Vec<SdoStep> {
    let new = PdoConfig { valid: enable, ..cfg.clone() };
    plan_apply(node, cfg, &new)
}

/// Enable or disable all PDOs (of one kind, or both with `None`).
pub fn plan_enable_all(node: u8, cfgs: &[PdoConfig], kind: Option<PdoKind>, enable: bool) -> Vec<SdoStep> {
    cfgs.iter().filter(|c| kind.is_none_or(|k| k == c.kind)).flat_map(|c| plan_enable(node, c, enable)).collect()
}

/// Apply a `PdoChange` to a PDO.
pub fn plan_change(node: u8, cfg: &PdoConfig, change: &PdoChange) -> Vec<SdoStep> {
    plan_apply(node, cfg, &change.apply(cfg))
}

/// Replace the mapping of a PDO.
pub fn plan_map(node: u8, cfg: &PdoConfig, mapping: Vec<MapEntry>) -> Vec<SdoStep> {
    plan_change(node, cfg, &PdoChange { mapping: Some(mapping), ..Default::default() })
}

/// Parse "0x6000:01:8" or "6000:1:8" (index hex, sub, bits decimal).
pub fn parse_map_entry(s: &str) -> Option<MapEntry> {
    let mut it = s.split(':');
    let ix = it.next()?.trim();
    let ix = u16::from_str_radix(ix.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()?;
    let sub = super::types::parse_int(it.next()?.trim(), false)? as u8;
    let bits: u8 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || bits == 0 || bits > 64 {
        return None;
    }
    Some(MapEntry { index: ix, sub, bits })
}
