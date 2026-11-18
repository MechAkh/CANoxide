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

pub const DEFAULT_SIGNAL_CAPACITY: usize = 20_000;
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
}

impl Signal {
    pub fn points(&self) -> &VecDeque<(f64, f64)> {
        &self.points
    }
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

    /// Add a signal; an SDO poll period below 50 ms is raised to 50 ms.
    pub fn add(&mut self, mut def: SignalDef) -> u64 {
        if let SignalSource::SdoPoll { period_ms, .. } = &mut def.source {
            *period_ms = (*period_ms).max(MIN_POLL_MS);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.signals.push(Signal { id, def, points: VecDeque::new(), last: None, min: None, max: None, count: 0, next_poll_ms: 0 });
        id
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let n = self.signals.len();
        self.signals.retain(|s| s.id != id);
        n != self.signals.len()
    }

    pub fn list(&self) -> &[Signal] {
        &self.signals
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
                    let v = match raw_kind {
                        RawKind::Unsigned => raw as f64,
                        RawKind::Signed => interpret(Some(DataType::Integer64), raw, *bits),
                        RawKind::Float if *bits == 32 => f32::from_bits(raw as u32) as f64,
                        RawKind::Float if *bits == 64 => f64::from_bits(raw),
                        RawKind::Float => continue,
                    };
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
        for s in &self.signals {
            for (t, v) in &s.points {
                writeln!(w, "{},{},{t:.6},{v}", crate::trace::csv_field(&s.def.name), crate::trace::csv_field(&s.def.unit))?;
            }
        }
        Ok(())
    }

    pub fn defs(&self) -> Vec<SignalDef> {
        self.signals.iter().map(|s| s.def.clone()).collect()
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
