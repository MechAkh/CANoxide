//! Trace store: ring buffer of decoded frames, the fixed (one row per
//! COB-ID) view, filters and CSV export. No UI code.

use crate::can::{CanFrame, Dir};
use crate::canopen::decode::{Decoded, MsgClass};
use crate::canopen::types::{format_bytes, DisplayMode};
use std::collections::{BTreeMap, VecDeque};
use std::io::Write;

#[derive(Clone, Debug)]
pub struct TraceEntry {
    pub seq: u64,
    pub frame: CanFrame,
    pub decoded: Decoded,
    /// Lower-case text for the text filter (made once on receive).
    search: String,
}

impl TraceEntry {
    pub fn new(seq: u64, frame: CanFrame, decoded: Decoded) -> Self {
        let search = format!("{:03x} {} {}", frame.id, decoded.label, decoded.text).to_lowercase();
        TraceEntry { seq, frame, decoded, search }
    }
}

#[derive(Clone, Debug)]
pub struct FixedRow {
    pub last: TraceEntry,
    pub count: u64,
    pub cycle_us: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceFilter {
    pub node: Option<u8>,
    /// Bit set of `MsgClass::bit()`.
    pub classes: u16,
    pub id_min: u32,
    pub id_max: u32,
    pub text: String,
}

impl Default for TraceFilter {
    fn default() -> Self {
        TraceFilter { node: None, classes: 0xFFFF, id_min: 0, id_max: 0x1FFF_FFFF, text: String::new() }
    }
}

impl TraceFilter {
    pub fn is_empty(&self) -> bool {
        *self == TraceFilter::default()
    }

    pub fn matches(&self, e: &TraceEntry) -> bool {
        self.matches_lower(e, &self.text.to_lowercase())
    }

    /// `matches` with the filter text already in lower case.
    fn matches_lower(&self, e: &TraceEntry, text_lower: &str) -> bool {
        if let Some(n) = self.node {
            if e.decoded.node != Some(n) {
                return false;
            }
        }
        if self.classes & e.decoded.class.bit() == 0 {
            return false;
        }
        if e.frame.id < self.id_min || e.frame.id > self.id_max {
            return false;
        }
        if !text_lower.is_empty() && !e.search.contains(text_lower) {
            return false;
        }
        true
    }

    pub fn class_enabled(&self, c: MsgClass) -> bool {
        self.classes & c.bit() != 0
    }

    pub fn set_class(&mut self, c: MsgClass, on: bool) {
        if on {
            self.classes |= c.bit();
        } else {
            self.classes &= !c.bit();
        }
    }
}

pub struct TraceStore {
    ring: VecDeque<TraceEntry>,
    capacity: usize,
    next_seq: u64,
    fixed: BTreeMap<(u32, bool), FixedRow>,
    filter: TraceFilter,
    text_lower: String,
    /// Sequence numbers of the entries that match the filter.
    filtered: VecDeque<u64>,
}

impl TraceStore {
    pub fn new(capacity: usize) -> Self {
        TraceStore {
            ring: VecDeque::with_capacity(capacity.min(1 << 16)),
            capacity: capacity.max(1),
            next_seq: 0,
            fixed: BTreeMap::new(),
            filter: TraceFilter::default(),
            text_lower: String::new(),
            filtered: VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn set_capacity(&mut self, c: usize) {
        self.capacity = c.max(1);
        self.trim();
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    pub fn total(&self) -> u64 {
        self.next_seq
    }

    pub fn clear(&mut self) {
        self.ring.clear();
        self.fixed.clear();
        self.filtered.clear();
    }

    fn trim(&mut self) {
        while self.ring.len() > self.capacity {
            self.ring.pop_front();
        }
        let first = self.ring.front().map(|e| e.seq).unwrap_or(self.next_seq);
        while self.filtered.front().is_some_and(|&s| s < first) {
            self.filtered.pop_front();
        }
    }

    pub fn push(&mut self, frame: CanFrame, decoded: Decoded) {
        let e = TraceEntry::new(self.next_seq, frame, decoded);
        self.next_seq += 1;
        let key = (frame.id, frame.extended);
        match self.fixed.get_mut(&key) {
            Some(r) => {
                r.cycle_us = Some(frame.timestamp_us.saturating_sub(r.last.frame.timestamp_us));
                r.count += 1;
                r.last = e.clone();
            }
            None => {
                self.fixed.insert(key, FixedRow { last: e.clone(), count: 1, cycle_us: None });
            }
        }
        if self.filter.matches_lower(&e, &self.text_lower) {
            self.filtered.push_back(e.seq);
        }
        self.ring.push_back(e);
        self.trim();
    }

    pub fn filter(&self) -> &TraceFilter {
        &self.filter
    }

    pub fn set_filter(&mut self, f: TraceFilter) {
        if f == self.filter {
            return;
        }
        self.filter = f;
        self.text_lower = self.filter.text.to_lowercase();
        self.filtered = self.ring.iter().filter(|e| self.filter.matches_lower(e, &self.text_lower)).map(|e| e.seq).collect();
    }

    /// Number of rows of the chronological view.
    pub fn view_len(&self) -> usize {
        self.filtered.len()
    }

    /// Row `i` of the chronological view (oldest first).
    pub fn view_row(&self, i: usize) -> Option<&TraceEntry> {
        let seq = *self.filtered.get(i)?;
        self.by_seq(seq)
    }

    pub fn by_seq(&self, seq: u64) -> Option<&TraceEntry> {
        let first = self.ring.front()?.seq;
        self.ring.get(seq.checked_sub(first)? as usize)
    }

    /// The entry before `e` in the full ring (for delta times).
    pub fn previous(&self, e: &TraceEntry) -> Option<&TraceEntry> {
        self.by_seq(e.seq.checked_sub(1)?)
    }

    /// Rows of the fixed view that match the filter, sorted by COB-ID.
    pub fn fixed_rows(&self) -> Vec<&FixedRow> {
        self.fixed.values().filter(|r| self.filter.matches_lower(&r.last, &self.text_lower)).collect()
    }

    /// Write the filtered chronological view as CSV.
    pub fn write_csv(&self, w: &mut impl Write, mode: DisplayMode) -> std::io::Result<()> {
        writeln!(w, "time_s,delta_ms,dir,id,ext,rtr,dlc,data,type,node,text")?;
        let mut prev: Option<u64> = None;
        for i in 0..self.view_len() {
            let Some(e) = self.view_row(i) else { continue };
            let f = &e.frame;
            let delta = prev.map(|p| format!("{:.3}", (f.timestamp_us.saturating_sub(p)) as f64 / 1000.0)).unwrap_or_default();
            prev = Some(f.timestamp_us);
            writeln!(
                w,
                "{:.6},{},{},{},{},{},{},{},{},{},{}",
                f.timestamp_us as f64 / 1e6,
                delta,
                f.dir.label(),
                id_text(f),
                f.extended as u8,
                f.rtr as u8,
                f.dlc,
                csv_field(&format_bytes(f.payload(), mode)),
                csv_field(&e.decoded.label),
                e.decoded.node.map(|n| n.to_string()).unwrap_or_default(),
                csv_field(&e.decoded.text)
            )?;
        }
        Ok(())
    }
}

pub fn id_text(f: &CanFrame) -> String {
    if f.extended {
        format!("{:08X}", f.id)
    } else {
        format!("{:03X}", f.id)
    }
}

pub fn dir_label(d: Dir) -> &'static str {
    d.label()
}

/// Quote a CSV field when needed (RFC 4180).
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
