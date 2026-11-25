//! Trace store: one ring buffer of decoded frames with several views
//! (filter and pause per view), the fixed (one row per COB-ID) view, CSV
//! export and the continuous CSV log. No UI code.

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

/// Identifier of a trace view. View 0 always exists; it is the view of the
/// single-view API (`set_filter`, `view_len`, `view_row`, `fixed_rows`,
/// `write_csv`).
pub type ViewId = usize;

/// One view of the shared ring: its own filter, pause state and cache of
/// the matching sequence numbers. A view costs one `u64` per matching row.
struct View {
    filter: TraceFilter,
    text_lower: String,
    paused: bool,
    filtered: VecDeque<u64>,
}

impl View {
    fn new() -> Self {
        View { filter: TraceFilter::default(), text_lower: String::new(), paused: false, filtered: VecDeque::new() }
    }

    fn rebuild(&mut self, ring: &VecDeque<TraceEntry>) {
        let (f, t) = (&self.filter, &self.text_lower);
        self.filtered = ring.iter().filter(|e| f.matches_lower(e, t)).map(|e| e.seq).collect();
    }
}

/// Ring buffer of decoded frames with any number of views (one shared
/// buffer, a per-view filter cache).
pub struct TraceStore {
    ring: VecDeque<TraceEntry>,
    capacity: usize,
    next_seq: u64,
    fixed: BTreeMap<(u32, bool), FixedRow>,
    /// Slot 0 is always `Some`. Removed views leave `None`.
    views: Vec<Option<View>>,
}

impl TraceStore {
    pub fn new(capacity: usize) -> Self {
        TraceStore {
            ring: VecDeque::with_capacity(capacity.min(1 << 16)),
            capacity: capacity.max(1),
            next_seq: 0,
            fixed: BTreeMap::new(),
            views: vec![Some(View::new())],
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

    /// The frames in the buffer, oldest first.
    pub fn frames(&self) -> impl DoubleEndedIterator<Item = &CanFrame> + ExactSizeIterator {
        self.ring.iter().map(|e| &e.frame)
    }

    pub fn clear(&mut self) {
        self.ring.clear();
        self.fixed.clear();
        for v in self.views.iter_mut().flatten() {
            v.filtered.clear();
        }
    }

    fn trim(&mut self) {
        while self.ring.len() > self.capacity {
            self.ring.pop_front();
        }
        let first = self.ring.front().map(|e| e.seq).unwrap_or(self.next_seq);
        for v in self.views.iter_mut().flatten() {
            while v.filtered.front().is_some_and(|&s| s < first) {
                v.filtered.pop_front();
            }
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
        for v in self.views.iter_mut().flatten() {
            if !v.paused && v.filter.matches_lower(&e, &v.text_lower) {
                v.filtered.push_back(e.seq);
            }
        }
        self.ring.push_back(e);
        self.trim();
    }

    // ---- views ----

    /// Add a view (no filter, running). Returns its id.
    pub fn add_view(&mut self) -> ViewId {
        let mut v = View::new();
        v.rebuild(&self.ring);
        if let Some(i) = self.views.iter().position(|v| v.is_none()) {
            self.views[i] = Some(v);
            i
        } else {
            self.views.push(Some(v));
            self.views.len() - 1
        }
    }

    /// Remove a view. View 0 cannot be removed; returns false for it and for
    /// an unknown id.
    pub fn remove_view(&mut self, id: ViewId) -> bool {
        if id == 0 || self.views.get(id).is_none_or(|v| v.is_none()) {
            return false;
        }
        self.views[id] = None;
        while self.views.len() > 1 && self.views.last().is_some_and(|v| v.is_none()) {
            self.views.pop();
        }
        true
    }

    /// Ids of the views that exist, in order.
    pub fn view_ids(&self) -> Vec<ViewId> {
        self.views.iter().enumerate().filter(|(_, v)| v.is_some()).map(|(i, _)| i).collect()
    }

    fn view(&self, id: ViewId) -> Option<&View> {
        self.views.get(id)?.as_ref()
    }

    /// Filter of a view (the default filter for an unknown id).
    pub fn view_filter(&self, id: ViewId) -> TraceFilter {
        self.view(id).map(|v| v.filter.clone()).unwrap_or_default()
    }

    /// Set the filter of a view; rebuilds its cache from the ring (also when
    /// paused: a paused view then shows the matching rows up to now).
    pub fn set_view_filter(&mut self, id: ViewId, f: TraceFilter) {
        let Some(Some(v)) = self.views.get_mut(id) else { return };
        if f == v.filter {
            return;
        }
        v.text_lower = f.text.to_lowercase();
        v.filter = f;
        v.rebuild(&self.ring);
    }

    /// A paused view keeps its rows and adds no new ones (rows still leave
    /// it when the ring drops them). Resuming catches up with the ring.
    pub fn set_view_paused(&mut self, id: ViewId, paused: bool) {
        let Some(Some(v)) = self.views.get_mut(id) else { return };
        if v.paused == paused {
            return;
        }
        v.paused = paused;
        if !paused {
            v.rebuild(&self.ring);
        }
    }

    pub fn view_paused(&self, id: ViewId) -> bool {
        self.view(id).is_some_and(|v| v.paused)
    }

    /// Number of rows of a view (0 for an unknown id).
    pub fn view_len_of(&self, id: ViewId) -> usize {
        self.view(id).map_or(0, |v| v.filtered.len())
    }

    /// Row `i` of a view (oldest first).
    pub fn view_row_of(&self, id: ViewId, i: usize) -> Option<&TraceEntry> {
        let seq = *self.view(id)?.filtered.get(i)?;
        self.by_seq(seq)
    }

    /// Index of the first row of a view whose sequence number is `seq` or
    /// later (the view length when there is none). A scrolled view keeps its
    /// place with it while the ring drops old rows.
    pub fn view_index_of_seq(&self, id: ViewId, seq: u64) -> usize {
        self.view(id).map_or(0, |v| v.filtered.partition_point(|&s| s < seq))
    }

    /// Rows of the fixed view that match the filter of view `id`, sorted by
    /// COB-ID.
    pub fn fixed_rows_of(&self, id: ViewId) -> Vec<&FixedRow> {
        let Some(v) = self.view(id) else { return Vec::new() };
        self.fixed.values().filter(|r| v.filter.matches_lower(&r.last, &v.text_lower)).collect()
    }

    /// Write the rows of view `id` as CSV.
    pub fn write_csv_of(&self, id: ViewId, w: &mut impl Write, mode: DisplayMode) -> std::io::Result<()> {
        writeln!(w, "{CSV_HEADER}")?;
        let mut prev: Option<u64> = None;
        for i in 0..self.view_len_of(id) {
            let Some(e) = self.view_row_of(id, i) else { continue };
            writeln!(w, "{}", csv_row(e, prev, mode))?;
            prev = Some(e.frame.timestamp_us);
        }
        Ok(())
    }

    /// The frames of view `id` (oldest first), for the `.trc` and `.asc`
    /// export.
    pub fn frames_of(&self, id: ViewId) -> Vec<crate::can::CanFrame> {
        (0..self.view_len_of(id)).filter_map(|i| self.view_row_of(id, i).map(|e| e.frame)).collect()
    }

    // ---- single-view API (view 0) ----

    pub fn filter(&self) -> &TraceFilter {
        &self.view(0).expect("view 0").filter
    }

    pub fn set_filter(&mut self, f: TraceFilter) {
        self.set_view_filter(0, f);
    }

    /// Number of rows of the chronological view.
    pub fn view_len(&self) -> usize {
        self.view_len_of(0)
    }

    /// Row `i` of the chronological view (oldest first).
    pub fn view_row(&self, i: usize) -> Option<&TraceEntry> {
        self.view_row_of(0, i)
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
        self.fixed_rows_of(0)
    }

    /// Write the filtered chronological view as CSV.
    pub fn write_csv(&self, w: &mut impl Write, mode: DisplayMode) -> std::io::Result<()> {
        self.write_csv_of(0, w, mode)
    }
}

/// Header line of the trace CSV (export and continuous log).
pub const CSV_HEADER: &str = "time_s,delta_ms,dir,id,ext,rtr,dlc,data,type,node,text";

/// One CSV line (no line end) of an entry; `prev_us` is the time stamp of
/// the previous written row (empty delta when `None`).
pub fn csv_row(e: &TraceEntry, prev_us: Option<u64>, mode: DisplayMode) -> String {
    let f = &e.frame;
    let delta = prev_us.map(|p| format!("{:.3}", (f.timestamp_us.saturating_sub(p)) as f64 / 1000.0)).unwrap_or_default();
    format!(
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
    )
}

/// Interval of the regular flush of the continuous log.
pub const LOG_FLUSH_MS: u64 = 500;

/// Continuous trace log: every frame given to it goes to a CSV file with
/// the export columns, independent of the ring buffer. Flushed at least
/// every `LOG_FLUSH_MS` (on `write` or `tick`) and on `stop`.
pub struct TraceLogger {
    path: std::path::PathBuf,
    w: std::io::BufWriter<std::fs::File>,
    mode: DisplayMode,
    prev_us: Option<u64>,
    rows: u64,
    last_flush_ms: u64,
    /// Optional filter: only matching frames are logged.
    filter: Option<(TraceFilter, String)>,
}

impl TraceLogger {
    /// Create (or truncate) the file and write the header.
    pub fn start(path: &std::path::Path, mode: DisplayMode, filter: Option<TraceFilter>, now_ms: u64) -> std::io::Result<Self> {
        let f = std::fs::File::create(path)?;
        let mut w = std::io::BufWriter::new(f);
        writeln!(w, "{CSV_HEADER}")?;
        w.flush()?;
        Ok(TraceLogger {
            path: path.to_path_buf(),
            w,
            mode,
            prev_us: None,
            rows: 0,
            last_flush_ms: now_ms,
            filter: filter.map(|f| {
                let t = f.text.to_lowercase();
                (f, t)
            }),
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Rows written so far.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// Log one entry (when it matches the filter); flushes when due.
    pub fn write(&mut self, e: &TraceEntry, now_ms: u64) -> std::io::Result<()> {
        if let Some((f, t)) = &self.filter {
            if !f.matches_lower(e, t) {
                return self.tick(now_ms);
            }
        }
        writeln!(self.w, "{}", csv_row(e, self.prev_us, self.mode))?;
        self.prev_us = Some(e.frame.timestamp_us);
        self.rows += 1;
        self.tick(now_ms)
    }

    /// Flush when the last flush is `LOG_FLUSH_MS` or more ago.
    pub fn tick(&mut self, now_ms: u64) -> std::io::Result<()> {
        if now_ms.saturating_sub(self.last_flush_ms) >= LOG_FLUSH_MS {
            self.last_flush_ms = now_ms;
            self.w.flush()?;
        }
        Ok(())
    }

    /// Flush and close. Returns the number of rows written.
    pub fn stop(mut self) -> std::io::Result<u64> {
        self.w.flush()?;
        Ok(self.rows)
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
