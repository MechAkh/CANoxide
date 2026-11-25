//! Signal store for the graph and the process-data view: named numeric
//! signals with time series in ring buffers.
//!
//! Sources: a mapped PDO object (updated from each matching frame by the
//! PDO configuration of the decoder), an SDO object polled at a period (the
//! bus worker runs the poll only while enabled, never in listen-only mode,
//! and user requests go first), or raw bits of a COB-ID. Each value is
//! `raw * scale + offset`.

use crate::can::CanFrame;
use crate::canopen::decode::DecodeContext;
use crate::canopen::pdo::extract_bits;
use crate::canopen::types::{le_u64, DataType};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;

pub const DEFAULT_SIGNAL_CAPACITY: usize = 500_000;
/// Shortest SDO poll period.
pub const MIN_POLL_MS: u64 = 50;
pub const SIGNALS_FILE: &str = "signals.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawKind {
    Unsigned,
    Signed,
    /// 32 bits: f32, 64 bits: f64.
    Float,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignalSource {
    /// A mapped object of a PDO of `node`.
    Pdo { node: u8, index: u16, sub: u8 },
    /// An SDO object polled every `period_ms` (at least 50 ms).
    SdoPoll { node: u8, index: u16, sub: u8, period_ms: u64, data_type: Option<u16> },
    /// Bits of the frames with this identifier.
    Raw { cob_id: u32, extended: bool, start_bit: u32, bits: u32, raw_kind: RawKind, big_endian: bool },
    /// Bits of the frames with the J1939 PGN of the 29-bit `cob_id` (any
    /// priority, any source address, PDU1 any destination), only from
    /// `source` when it is given. With J1939 off (setting "Extended IDs
    /// are": Raw frames) the exact 29-bit ID.
    J1939 { cob_id: u32, source: Option<u8>, start_bit: u32, bits: u32, raw_kind: RawKind, big_endian: bool },
}

/// True when frame `f_id`/`f_ext` belongs to a signal of `sig_id`: by the
/// J1939 PGN (and source address) when `pgn` is set and J1939 is on, else
/// by the exact ID and format.
pub fn id_matches(sig_id: u32, sig_ext: bool, pgn: Option<Option<u8>>, f_id: u32, f_ext: bool, j1939_on: bool) -> bool {
    match pgn {
        Some(sa) if j1939_on => sig_ext && f_ext && crate::j1939::pgn_matches(sig_id, sa, f_id),
        _ => sig_id == f_id && sig_ext == f_ext,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalDef {
    pub name: String,
    #[serde(default)]
    pub unit: String,
    pub scale: f64,
    pub offset: f64,
    pub source: SignalSource,
}

impl SignalDef {
    pub fn new(name: impl Into<String>, source: SignalSource) -> SignalDef {
        SignalDef { name: name.into(), unit: String::new(), scale: 1.0, offset: 0.0, source }
    }
}

#[derive(Clone, Debug)]
pub struct Signal {
    pub id: u64,
    pub def: SignalDef,
    /// (time s, value).
    points: VecDeque<(f64, f64)>,
    pub last: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub count: u64,
    next_poll_ms: u64,
    /// The Graph shows this signal.
    pub in_graph: bool,
    /// Dashboard widgets that show this signal.
    pub dash_refs: u32,
}

impl Signal {
    pub fn points(&self) -> &VecDeque<(f64, f64)> {
        &self.points
    }

    /// The newest point (time s, value).
    pub fn last_point(&self) -> Option<(f64, f64)> {
        self.points.back().copied()
    }

    /// The sample period in s: the SDO poll period, else the time between
    /// the last two points (None with fewer points).
    pub fn period_s(&self) -> Option<f64> {
        if let SignalSource::SdoPoll { period_ms, .. } = self.def.source {
            return Some(period_ms as f64 / 1000.0);
        }
        let n = self.points.len();
        if n < 2 {
            return None;
        }
        Some((self.points[n - 1].0 - self.points[n - 2].0).max(0.0))
    }
}

/// The source without the poll period (two signals with the same key read
/// the same value).
fn source_key(s: &SignalSource) -> SignalSource {
    match s {
        SignalSource::SdoPoll { node, index, sub, data_type, .. } => SignalSource::SdoPoll { node: *node, index: *index, sub: *sub, period_ms: 0, data_type: *data_type },
        other => other.clone(),
    }
}

/// Two definitions give the same values (the poll period aside).
pub fn same_signal(a: &SignalDef, b: &SignalDef) -> bool {
    a.name == b.name && a.unit == b.unit && a.scale == b.scale && a.offset == b.offset && source_key(&a.source) == source_key(&b.source)
}

/// One point of a decimated slice: the time of the bucket start, the
/// lowest and the highest value in the bucket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinMaxPoint {
    pub t: f64,
    pub min: f64,
    pub max: f64,
}

/// An SDO poll that is due: (signal id, node, index, sub).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollRequest {
    pub id: u64,
    pub node: u8,
    pub index: u16,
    pub sub: u8,
}

#[derive(Clone, Debug)]
pub struct SignalStore {
    signals: Vec<Signal>,
    capacity: usize,
    next_id: u64,
}

impl Default for SignalStore {
    fn default() -> Self {
        SignalStore::new(DEFAULT_SIGNAL_CAPACITY)
    }
}

fn interpret(data_type: Option<DataType>, raw: u64, bits: u32) -> f64 {
    match data_type {
        Some(DataType::Real32) if bits == 32 => f32::from_bits(raw as u32) as f64,
        Some(DataType::Real64) if bits == 64 => f64::from_bits(raw),
        Some(t) if t.is_signed() && bits > 0 && bits < 64 => {
            let s = 64 - bits;
            (((raw << s) as i64) >> s) as f64
        }
        Some(t) if t.is_signed() => raw as i64 as f64,
        _ => raw as f64,
    }
}

/// Bits of a frame: little endian = Intel (start bit = LSB), big endian =
/// Motorola (start bit = MSB, counted as in DBC files).
pub fn raw_bits(data: &[u8], start_bit: u32, bits: u32, big_endian: bool) -> Option<u64> {
    if !big_endian {
        return extract_bits(data, start_bit, bits);
    }
    if bits == 0 || bits > 64 {
        return None;
    }
    let mut v: u64 = 0;
    let mut pos = start_bit as i64;
    for _ in 0..bits {
        if pos < 0 || (pos / 8) as usize >= data.len() {
            return None;
        }
        let b = (data[(pos / 8) as usize] >> (pos % 8)) & 1;
        v = (v << 1) | b as u64;
        // Next lower bit: down in the byte, then to bit 7 of the next byte.
        if pos % 8 == 0 {
            pos += 15;
        } else {
            pos -= 1;
        }
    }
    Some(v)
}

/// The number of a raw bit field: unsigned, signed (two's complement of
/// `bits`) or float (32 or 64 bits; other sizes give `None`).
pub fn raw_number(raw: u64, bits: u32, kind: RawKind) -> Option<f64> {
    match kind {
        RawKind::Unsigned => Some(raw as f64),
        RawKind::Signed => Some(interpret(Some(DataType::Integer64), raw, bits)),
        RawKind::Float if bits == 32 => Some(f32::from_bits(raw as u32) as f64),
        RawKind::Float if bits == 64 => Some(f64::from_bits(raw)),
        RawKind::Float => None,
    }
}

/// The bit indices (k = byte * 8 + bit) of a field, from the start bit
/// on: little endian upwards, big endian (Motorola) as `raw_bits` walks.
/// `None` when the field does not fit in 8 bytes.
pub fn field_bits(start_bit: u32, bits: u32, big_endian: bool) -> Option<Vec<u32>> {
    if bits == 0 || bits > 64 {
        return None;
    }
    if !big_endian {
        return (start_bit + bits <= 64).then(|| (start_bit..start_bit + bits).collect());
    }
    let mut v = Vec::with_capacity(bits as usize);
    let mut pos = start_bit as i64;
    for _ in 0..bits {
        if !(0..64).contains(&pos) {
            return None;
        }
        v.push(pos as u32);
        if pos % 8 == 0 {
            pos += 15;
        } else {
            pos -= 1;
        }
    }
    Some(v)
}

impl SignalStore {
    pub fn new(capacity: usize) -> SignalStore {
        SignalStore { signals: Vec::new(), capacity: capacity.max(2), next_id: 1 }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn set_capacity(&mut self, c: usize) {
        self.capacity = c.max(2);
        for s in &mut self.signals {
            while s.points.len() > self.capacity {
                s.points.pop_front();
            }
        }
    }

    fn push_new(&mut self, mut def: SignalDef, in_graph: bool, dash_refs: u32) -> u64 {
        if let SignalSource::SdoPoll { period_ms, .. } = &mut def.source {
            *period_ms = (*period_ms).max(MIN_POLL_MS);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.signals.push(Signal { id, def, points: VecDeque::new(), last: None, min: None, max: None, count: 0, next_poll_ms: 0, in_graph, dash_refs });
        id
    }

    /// A signal that only the dashboard shows and that gives the same values
    /// as `def`; the shorter poll period wins.
    fn shared_with(&mut self, def: &SignalDef, graph: bool) -> Option<&mut Signal> {
        let s = self.signals.iter_mut().find(|s| same_signal(&s.def, def) && (!graph || !s.in_graph))?;
        if let (SignalSource::SdoPoll { period_ms: a, .. }, SignalSource::SdoPoll { period_ms: b, .. }) = (&mut s.def.source, &def.source) {
            *a = (*a).min((*b).max(MIN_POLL_MS));
        }
        Some(s)
    }

    /// Add a signal to the Graph; an SDO poll period below 50 ms is raised
    /// to 50 ms. A signal that the dashboard already samples is reused, so
    /// that it is polled once.
    pub fn add(&mut self, def: SignalDef) -> u64 {
        if let Some(s) = self.shared_with(&def, true) {
            s.in_graph = true;
            return s.id;
        }
        self.push_new(def, true, 0)
    }

    /// A dashboard widget needs this signal: reuse an equal one or add it
    /// (not shown in the Graph). Pair with `release_dash`.
    pub fn acquire_dash(&mut self, def: &SignalDef) -> u64 {
        if let Some(s) = self.shared_with(def, false) {
            s.dash_refs += 1;
            return s.id;
        }
        self.push_new(def.clone(), false, 1)
    }

    /// A dashboard widget no longer needs the signal.
    pub fn release_dash(&mut self, id: u64) {
        let Some(s) = self.signals.iter_mut().find(|s| s.id == id) else { return };
        s.dash_refs = s.dash_refs.saturating_sub(1);
        if s.dash_refs == 0 && !s.in_graph {
            self.signals.retain(|s| s.id != id);
        }
    }

    /// Remove a signal from the Graph; it stays sampled while a dashboard
    /// widget shows it.
    pub fn remove(&mut self, id: u64) -> bool {
        if let Some(s) = self.signals.iter_mut().find(|s| s.id == id && s.dash_refs > 0) {
            let was = s.in_graph;
            s.in_graph = false;
            return was;
        }
        let n = self.signals.len();
        self.signals.retain(|s| s.id != id);
        n != self.signals.len()
    }

    /// All sampled signals (Graph and dashboard).
    pub fn list(&self) -> &[Signal] {
        &self.signals
    }

    /// The signals of the Graph.
    pub fn graph_signals(&self) -> impl Iterator<Item = &Signal> {
        self.signals.iter().filter(|s| s.in_graph)
    }

    pub fn get(&self, id: u64) -> Option<&Signal> {
        self.signals.iter().find(|s| s.id == id)
    }

    pub fn find(&self, name: &str) -> Option<&Signal> {
        self.signals.iter().find(|s| s.def.name == name)
    }

    /// Clear all points and statistics; the definitions stay.
    pub fn clear(&mut self) {
        for s in &mut self.signals {
            s.points.clear();
            s.last = None;
            s.min = None;
            s.max = None;
            s.count = 0;
        }
    }

    /// Add a raw value (before scale and offset) to a signal.
    pub fn push_raw(&mut self, id: u64, t_s: f64, raw: f64) {
        let cap = self.capacity;
        if let Some(s) = self.signals.iter_mut().find(|s| s.id == id) {
            let v = raw * s.def.scale + s.def.offset;
            if s.points.len() >= cap {
                s.points.pop_front();
            }
            s.points.push_back((t_s, v));
            s.last = Some(v);
            s.min = Some(s.min.map_or(v, |m| m.min(v)));
            s.max = Some(s.max.map_or(v, |m| m.max(v)));
            s.count += 1;
        }
    }

    /// Update the PDO and raw signals from a frame.
    pub fn on_frame(&mut self, f: &CanFrame, ctx: &DecodeContext) {
        if f.rtr {
            return;
        }
        let t = f.timestamp_us as f64 / 1e6;
        let data = f.payload();
        let mut updates: Vec<(u64, f64)> = Vec::new();
        let pdo = if f.extended { None } else { ctx.pdo(f.id) };
        for s in &self.signals {
            match &s.def.source {
                SignalSource::Pdo { node, index, sub } => {
                    let Some((pn, cfg)) = pdo else { continue };
                    if pn != node {
                        continue;
                    }
                    let mut off = 0u32;
                    for m in &cfg.mapping {
                        let b = m.bits as u32;
                        if !m.is_dummy() && m.index == *index && m.sub == *sub {
                            if let Some(raw) = extract_bits(data, off, b) {
                                updates.push((s.id, interpret(ctx.entry_type(*node, *index, *sub), raw, b)));
                            }
                            break;
                        }
                        off += b;
                    }
                }
                SignalSource::Raw { cob_id, extended, start_bit, bits, raw_kind, big_endian } => {
                    if *cob_id != f.id || *extended != f.extended {
                        continue;
                    }
                    let Some(raw) = raw_bits(data, *start_bit, *bits, *big_endian) else { continue };
                    let Some(v) = raw_number(raw, *bits, *raw_kind) else { continue };
                    updates.push((s.id, v));
                }
                SignalSource::J1939 { cob_id, source, start_bit, bits, raw_kind, big_endian } => {
                    if !id_matches(*cob_id, true, Some(*source), f.id, f.extended, ctx.j1939()) {
                        continue;
                    }
                    let Some(raw) = raw_bits(data, *start_bit, *bits, *big_endian) else { continue };
                    let Some(v) = raw_number(raw, *bits, *raw_kind) else { continue };
                    updates.push((s.id, v));
                }
                SignalSource::SdoPoll { .. } => {}
            }
        }
        for (id, v) in updates {
            self.push_raw(id, t, v);
        }
    }

    /// The SDO polls due at `now_ms` (their next time is set).
    pub fn polls_due(&mut self, now_ms: u64) -> Vec<PollRequest> {
        let mut v = Vec::new();
        for s in &mut self.signals {
            if let SignalSource::SdoPoll { node, index, sub, period_ms, .. } = s.def.source {
                if now_ms >= s.next_poll_ms {
                    s.next_poll_ms = now_ms + period_ms.max(MIN_POLL_MS);
                    v.push(PollRequest { id: s.id, node, index, sub });
                }
            }
        }
        v
    }

    /// The answer to an SDO poll (raw object bytes).
    pub fn on_poll_value(&mut self, id: u64, t_s: f64, data: &[u8], ctx: &DecodeContext) {
        let Some(s) = self.get(id) else { return };
        let SignalSource::SdoPoll { node, index, sub, data_type, .. } = s.def.source else { return };
        let dt = data_type.and_then(DataType::from_code).or_else(|| ctx.entry_type(node, index, sub));
        let n = dt.and_then(|d| d.size()).unwrap_or(data.len()).min(data.len()).min(8);
        let raw = le_u64(&data[..n]);
        let v = interpret(dt, raw, (n * 8) as u32);
        self.push_raw(id, t_s, v);
    }

    pub fn has_polls(&self) -> bool {
        self.signals.iter().any(|s| matches!(s.def.source, SignalSource::SdoPoll { .. }))
    }

    /// Points of `id` in [t0, t1] decimated to at most `max_points`
    /// buckets with min and max (for plotting).
    pub fn slice(&self, id: u64, t0: f64, t1: f64, max_points: usize) -> Vec<MinMaxPoint> {
        let Some(s) = self.get(id) else { return Vec::new() };
        let pts: Vec<(f64, f64)> = s.points.iter().copied().filter(|(t, _)| *t >= t0 && *t <= t1).collect();
        let max_points = max_points.max(1);
        if pts.len() <= max_points {
            return pts.into_iter().map(|(t, v)| MinMaxPoint { t, min: v, max: v }).collect();
        }
        let width = (t1 - t0) / max_points as f64;
        let mut out: Vec<MinMaxPoint> = Vec::with_capacity(max_points);
        for (t, v) in pts {
            let b = (((t - t0) / width) as usize).min(max_points - 1);
            let bt = t0 + b as f64 * width;
            match out.last_mut() {
                Some(p) if (p.t - bt).abs() < width * 1e-9 => {
                    p.min = p.min.min(v);
                    p.max = p.max.max(v);
                }
                _ => out.push(MinMaxPoint { t: bt, min: v, max: v }),
            }
        }
        out
    }

    /// All points as CSV: `signal,unit,time_s,value`.
    pub fn write_csv(&self, w: &mut impl Write) -> std::io::Result<()> {
        writeln!(w, "signal,unit,time_s,value")?;
        for s in self.graph_signals() {
            for (t, v) in &s.points {
                writeln!(w, "{},{},{t:.6},{v}", crate::trace::csv_field(&s.def.name), crate::trace::csv_field(&s.def.unit))?;
            }
        }
        Ok(())
    }

    /// The points of the signals `ids` in [t0, t1] as one table: a header
    /// `time_s,<name> [<unit>],...`, then one row per distinct time (sorted),
    /// with an empty cell where a signal has no sample at that time (the
    /// last sample wins when a signal has two at the same time). Returns the
    /// number of rows.
    pub fn write_range_csv(&self, w: &mut impl Write, ids: &[u64], t0: f64, t1: f64) -> std::io::Result<usize> {
        let sigs: Vec<&Signal> = ids.iter().filter_map(|id| self.get(*id)).collect();
        let mut head = String::from("time_s");
        for s in &sigs {
            let n = if s.def.unit.is_empty() { s.def.name.clone() } else { format!("{} [{}]", s.def.name, s.def.unit) };
            head.push(',');
            head.push_str(&crate::trace::csv_field(&n));
        }
        writeln!(w, "{head}")?;
        let mut pts: Vec<(f64, usize, f64)> = Vec::new();
        for (i, s) in sigs.iter().enumerate() {
            pts.extend(s.points.iter().filter(|(t, _)| *t >= t0 && *t <= t1).map(|(t, v)| (*t, i, *v)));
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut rows = 0;
        let mut cells: Vec<Option<f64>> = vec![None; sigs.len()];
        let mut i = 0;
        while i < pts.len() {
            let t = pts[i].0;
            cells.iter_mut().for_each(|c| *c = None);
            while i < pts.len() && pts[i].0 == t {
                cells[pts[i].1] = Some(pts[i].2);
                i += 1;
            }
            let mut line = format!("{t:.6}");
            for c in &cells {
                line.push(',');
                if let Some(v) = c {
                    line.push_str(&v.to_string());
                }
            }
            writeln!(w, "{line}")?;
            rows += 1;
        }
        Ok(rows)
    }

    /// The definitions of the Graph signals.
    pub fn defs(&self) -> Vec<SignalDef> {
        self.graph_signals().map(|s| s.def.clone()).collect()
    }

    /// Save the definitions as JSON.
    pub fn save_defs(&self, path: &Path) -> std::io::Result<()> {
        let s = serde_json::to_vec_pretty(&self.defs()).map_err(|e| std::io::Error::other(e.to_string()))?;
        crate::library::write_atomic(path, &s).map_err(|e| std::io::Error::other(e.0))
    }

    /// Load definitions (a missing file gives none) and add them.
    pub fn load_defs(&mut self, path: &Path) -> std::io::Result<usize> {
        let b = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let defs: Vec<SignalDef> = serde_json::from_slice(&b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let n = defs.len();
        for d in defs {
            self.add(d);
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_csv_layout_with_empty_cells() {
        let mut s = SignalStore::new(100);
        let a = s.add(SignalDef { unit: "V".to_string(), ..SignalDef::new("a", SignalSource::Pdo { node: 1, index: 0x6000, sub: 1 }) });
        let b = s.add(SignalDef::new("b,c", SignalSource::Pdo { node: 1, index: 0x6000, sub: 2 }));
        for (t, v) in [(0.5, 1.0), (1.0, 2.0), (2.0, 3.0), (3.5, 9.0)] {
            s.push_raw(a, t, v);
        }
        for (t, v) in [(1.0, 10.0), (1.5, 11.0), (4.0, 12.0)] {
            s.push_raw(b, t, v);
        }
        let mut out = Vec::new();
        let n = s.write_range_csv(&mut out, &[a, b], 1.0, 3.5).unwrap();
        let t = String::from_utf8(out).unwrap();
        assert_eq!(t, "time_s,a [V],\"b,c\"\n1.000000,2,10\n1.500000,,11\n2.000000,3,\n3.500000,9,\n");
        assert_eq!(n, 4);
        // Unknown ids are skipped; an empty range gives the header only.
        let mut out = Vec::new();
        assert_eq!(s.write_range_csv(&mut out, &[99, a], 10.0, 20.0).unwrap(), 0);
        assert_eq!(String::from_utf8(out).unwrap(), "time_s,a [V]\n");
    }

    #[test]
    fn raw_number_and_field_bits() {
        assert_eq!(raw_number(0xFF, 8, RawKind::Signed), Some(-1.0));
        assert_eq!(raw_number(0xFF, 8, RawKind::Unsigned), Some(255.0));
        assert_eq!(raw_number(1.5f32.to_bits() as u64, 32, RawKind::Float), Some(1.5));
        assert_eq!(raw_number(0, 16, RawKind::Float), None);
        assert_eq!(field_bits(6, 4, false), Some(vec![6, 7, 8, 9]));
        assert_eq!(field_bits(1, 4, true), Some(vec![1, 0, 15, 14]));
        assert_eq!(field_bits(62, 4, false), None);
    }
}
