//! Signal analysis (0.3): find bits that follow a reference bit, narrow
//! down an unknown signal by steps (Find signal), the per-COB-ID history
//! with a filter and CSV export, and sparkline ring buffers. All of it
//! works on frames in time order (the trace buffer or the loaded log up to
//! the replay cursor); the UI only draws the results.

use crate::can::CanFrame;
use crate::msgstats::MsgKey;
use crate::signals::{field_bits, raw_bits, raw_number, RawKind};
use crate::usersig::{fmt_value, UserSignal};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn data_word(f: &CanFrame) -> (u64, u32) {
    let d = f.payload();
    let n = d.len().min(8);
    let mut b = [0u8; 8];
    b[..n].copy_from_slice(&d[..n]);
    (u64::from_le_bytes(b), n as u32 * 8)
}

// ---------------------------------------------------------------- similar bits

/// One bit of one message (k = byte * 8 + bit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitRef {
    pub key: MsgKey,
    pub bit: u32,
}

/// How one bit compares with the reference bit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BitMatch {
    pub key: MsgKey,
    pub bit: u32,
    /// Reference frames at which the bit was equal to the reference bit.
    pub equal: u64,
    /// Reference frames at which the bit had a value.
    pub compared: u64,
    /// Changes of the bit over the frames.
    pub toggles: u64,
}

impl BitMatch {
    pub fn match_pct(&self) -> f64 {
        if self.compared == 0 {
            0.0
        } else {
            self.equal as f64 * 100.0 / self.compared as f64
        }
    }

    pub fn inverted_pct(&self) -> f64 {
        if self.compared == 0 {
            0.0
        } else {
            100.0 - self.match_pct()
        }
    }

    /// The better of match and inverted match.
    pub fn best_pct(&self) -> f64 {
        self.match_pct().max(self.inverted_pct())
    }
}

struct BitState {
    cur: u64,
    width: u32,
    seen: bool,
    toggles: [u64; 64],
    equal: [u64; 64],
    /// Reference frames counted while the message had `n` data bytes.
    cmp_by_len: [u64; 9],
}

impl BitState {
    fn new() -> Self {
        BitState { cur: 0, width: 0, seen: false, toggles: [0; 64], equal: [0; 64], cmp_by_len: [0; 9] }
    }
}

/// Rank every bit of every message by how well it follows the reference
/// bit. The frames are stepped through in time order with the current
/// value of every bit; at each frame of the reference message each bit
/// that has a value is counted equal or not equal. Bits that never toggled
/// (and the reference bit itself) are left out. Sorted best first.
/// `None` when `cancel` was set.
pub fn similar_bits<'a>(frames: impl IntoIterator<Item = &'a CanFrame>, reference: BitRef, cancel: &AtomicBool, progress: Option<&AtomicUsize>) -> Option<Vec<BitMatch>> {
    let mut states: HashMap<MsgKey, BitState> = HashMap::new();
    for (i, f) in frames.into_iter().enumerate() {
        if i % 4096 == 0 {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(p) = progress {
                p.store(i, Ordering::Relaxed);
            }
        }
        if f.rtr {
            continue;
        }
        let key = MsgKey::of(f);
        let (word, width) = data_word(f);
        let st = states.entry(key).or_insert_with(BitState::new);
        if st.seen {
            let mut diff = (st.cur ^ word) & mask(width.min(st.width));
            while diff != 0 {
                let b = diff.trailing_zeros();
                st.toggles[b as usize] += 1;
                diff &= diff - 1;
            }
        }
        st.cur = word;
        st.width = width;
        st.seen = true;
        if key != reference.key || reference.bit >= width {
            continue;
        }
        let r = (word >> reference.bit) & 1;
        let rmask = if r == 1 { u64::MAX } else { 0 };
        for st in states.values_mut() {
            st.cmp_by_len[(st.width / 8) as usize] += 1;
            let mut eq = !(st.cur ^ rmask) & mask(st.width);
            while eq != 0 {
                let b = eq.trailing_zeros();
                st.equal[b as usize] += 1;
                eq &= eq - 1;
            }
        }
    }
    let mut out = Vec::new();
    for (key, st) in &states {
        for bit in 0..64u32 {
            if st.toggles[bit as usize] == 0 || (*key == reference.key && bit == reference.bit) {
                continue;
            }
            let compared: u64 = st.cmp_by_len[(bit / 8 + 1) as usize..].iter().sum();
            if compared == 0 {
                continue;
            }
            out.push(BitMatch { key: *key, bit, equal: st.equal[bit as usize], compared, toggles: st.toggles[bit as usize] });
        }
    }
    out.sort_by(|a, b| b.best_pct().total_cmp(&a.best_pct()).then(b.toggles.cmp(&a.toggles)).then(a.key.cmp(&b.key)).then(a.bit.cmp(&b.bit)));
    Some(out)
}

fn mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

// ----------------------------------------------------------------- find signal

/// Most candidates kept; more are cut (the UI says so).
pub const MAX_CANDIDATES: usize = 200_000;
/// Undo depth of Find signal.
pub const MAX_UNDO: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct FindSettings {
    /// Field sizes in bits (1 = single bits).
    pub sizes: Vec<u32>,
    /// Multi-bit fields only at byte borders (else at every start bit).
    pub byte_aligned: bool,
    pub big_endian: bool,
    pub signed: bool,
    /// Only these COB-IDs (empty: every COB-ID seen).
    pub ids: Vec<MsgKey>,
}

impl Default for FindSettings {
    fn default() -> Self {
        FindSettings { sizes: vec![1, 8, 16, 32], byte_aligned: true, big_endian: false, signed: false, ids: Vec::new() }
    }
}

/// One step of Find signal; the relative steps compare with the values at
/// the previous step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FindStep {
    Eq(f64),
    Ne(f64),
    Gt(f64),
    Lt(f64),
    Between(f64, f64),
    Increased,
    Decreased,
    Changed,
    Unchanged,
}

impl FindStep {
    pub fn text(&self) -> String {
        match self {
            FindStep::Eq(x) => format!("= {}", fmt_value(*x)),
            FindStep::Ne(x) => format!("!= {}", fmt_value(*x)),
            FindStep::Gt(x) => format!("> {}", fmt_value(*x)),
            FindStep::Lt(x) => format!("< {}", fmt_value(*x)),
            FindStep::Between(a, b) => format!("between {} and {}", fmt_value(*a), fmt_value(*b)),
            FindStep::Increased => "increased".to_string(),
            FindStep::Decreased => "decreased".to_string(),
            FindStep::Changed => "changed".to_string(),
            FindStep::Unchanged => "unchanged".to_string(),
        }
    }

    fn keep(&self, prev: f64, v: f64) -> bool {
        match *self {
            FindStep::Eq(x) => v == x,
            FindStep::Ne(x) => v != x,
            FindStep::Gt(x) => v > x,
            FindStep::Lt(x) => v < x,
            FindStep::Between(a, b) => v >= a.min(b) && v <= a.max(b),
            FindStep::Increased => v > prev,
            FindStep::Decreased => v < prev,
            FindStep::Changed => v != prev,
            FindStep::Unchanged => v == prev,
        }
    }
}

/// A candidate field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub key: MsgKey,
    pub start_bit: u32,
    pub bits: u32,
    /// The value at the last step.
    pub value: f64,
}

/// The candidate set of Find signal with its steps and undo.
#[derive(Clone, Debug, Default)]
pub struct FindSignal {
    pub settings: FindSettings,
    cands: Vec<Candidate>,
    undo: Vec<(Vec<Candidate>, String)>,
    steps: Vec<String>,
    /// The start set was larger than `MAX_CANDIDATES`.
    pub cut: bool,
    started: bool,
}

impl FindSignal {
    pub fn new(settings: FindSettings) -> FindSignal {
        FindSignal { settings, ..FindSignal::default() }
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.cands
    }

    /// The step texts so far.
    pub fn steps(&self) -> &[String] {
        &self.steps
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    fn value(&self, data: &[u8], start: u32, bits: u32) -> Option<f64> {
        let raw = raw_bits(data, start, bits, self.settings.big_endian)?;
        raw_number(raw, bits, if self.settings.signed && bits > 1 { RawKind::Signed } else { RawKind::Unsigned })
    }

    /// Start (or reset): every field of the configured sizes of every
    /// message in `latest` (newest data per COB-ID), with its current value.
    pub fn reset<'a>(&mut self, latest: impl IntoIterator<Item = (MsgKey, &'a [u8])>) {
        self.cands.clear();
        self.undo.clear();
        self.steps.clear();
        self.cut = false;
        self.started = true;
        let mut msgs: Vec<(MsgKey, &[u8])> = latest.into_iter().filter(|(k, _)| self.settings.ids.is_empty() || self.settings.ids.contains(k)).collect();
        msgs.sort_by_key(|(k, _)| *k);
        let mut sizes = self.settings.sizes.clone();
        sizes.sort_unstable();
        sizes.dedup();
        'outer: for (key, data) in msgs {
            let width = (data.len().min(8) * 8) as u32;
            for &bits in &sizes {
                if bits == 0 || bits > 64 {
                    continue;
                }
                for start in 0..width {
                    if bits > 1 && self.settings.byte_aligned {
                        // LE: LSB at a byte start; BE: MSB at bit 7 of a byte.
                        let aligned = if self.settings.big_endian { start % 8 == 7 } else { start.is_multiple_of(8) };
                        if !aligned {
                            continue;
                        }
                    }
                    let Some(pos) = field_bits(start, bits, self.settings.big_endian) else { continue };
                    if pos.iter().any(|p| *p >= width) {
                        continue;
                    }
                    let Some(value) = self.value(data, start, bits) else { continue };
                    if self.cands.len() >= MAX_CANDIDATES {
                        self.cut = true;
                        break 'outer;
                    }
                    self.cands.push(Candidate { key, start_bit: start, bits, value });
                }
            }
        }
    }

    /// Keep the candidates whose current value passes `step`. A candidate
    /// whose message has no data now (or too few bytes) is dropped.
    pub fn step(&mut self, step: FindStep, latest: impl Fn(MsgKey) -> Option<Vec<u8>>) {
        let mut cache: HashMap<MsgKey, Option<Vec<u8>>> = HashMap::new();
        let mut next = Vec::with_capacity(self.cands.len());
        for c in &self.cands {
            let d = cache.entry(c.key).or_insert_with(|| latest(c.key));
            let Some(d) = d else { continue };
            let Some(v) = self.value(d, c.start_bit, c.bits) else { continue };
            if step.keep(c.value, v) {
                next.push(Candidate { value: v, ..*c });
            }
        }
        let old = std::mem::replace(&mut self.cands, next);
        self.undo.push((old, step.text()));
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.steps.push(step.text());
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some((c, _)) => {
                self.cands = c;
                self.steps.pop();
                true
            }
            None => false,
        }
    }
}

// --------------------------------------------------------------------- history

/// Operators of the history filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Lt,
}

impl CmpOp {
    pub const ALL: [CmpOp; 4] = [CmpOp::Eq, CmpOp::Ne, CmpOp::Gt, CmpOp::Lt];

    pub fn label(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Gt => ">",
            CmpOp::Lt => "<",
        }
    }

    pub fn test(self, v: f64, x: f64) -> bool {
        match self {
            CmpOp::Eq => v == x,
            CmpOp::Ne => v != x,
            CmpOp::Gt => v > x,
            CmpOp::Lt => v < x,
        }
    }
}

/// Keep the rows where signal column `col` passes `op value`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistFilter {
    pub col: usize,
    pub op: CmpOp,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistRow {
    pub t_us: u64,
    /// Time since the previous frame of this COB-ID.
    pub dt_us: Option<u64>,
    pub len: u8,
    pub rtr: bool,
    pub data: [u8; 8],
    pub values: Vec<Option<f64>>,
}

/// The frames of `key`, newest first, with one value per signal; rows that
/// fail `filter` are left out. At most `limit` rows (the newest).
pub fn history<'a>(frames: impl DoubleEndedIterator<Item = &'a CanFrame>, key: MsgKey, sigs: &[UserSignal], filter: Option<&HistFilter>, limit: usize) -> Vec<HistRow> {
    let mut out = Vec::new();
    let mut newer: Option<HistRow> = None;
    let mut it = frames.rev().filter(|f| MsgKey::of(f) == key);
    // Walk newest to oldest; a row's delta is known when the older frame comes.
    loop {
        let f = it.next();
        if let Some(mut row) = newer.take() {
            row.dt_us = f.map(|f| row.t_us.saturating_sub(f.timestamp_us));
            let keep = match filter {
                Some(h) => row.values.get(h.col).copied().flatten().is_some_and(|v| h.op.test(v, h.value)),
                None => true,
            };
            if keep {
                out.push(row);
                if out.len() >= limit {
                    break;
                }
            }
        }
        let Some(f) = f else { break };
        let d = f.payload();
        let mut data = [0u8; 8];
        let n = d.len().min(8);
        data[..n].copy_from_slice(&d[..n]);
        let values = sigs.iter().map(|s| if f.rtr { None } else { s.value(d) }).collect();
        newer = Some(HistRow { t_us: f.timestamp_us, dt_us: None, len: n as u8, rtr: f.rtr, data, values });
    }
    out
}

/// The history rows as CSV: `time_s,delta_ms,data,<signal> [<unit>],...`
/// (data as hex bytes with spaces, "RTR" for a remote frame; an empty cell
/// for no value).
pub fn write_history_csv(w: &mut impl Write, rows: &[HistRow], sigs: &[UserSignal]) -> std::io::Result<()> {
    let mut head = String::from("time_s,delta_ms,data");
    for s in sigs {
        let n = if s.unit.is_empty() { s.name.clone() } else { format!("{} [{}]", s.name, s.unit) };
        head.push(',');
        head.push_str(&crate::trace::csv_field(&n));
    }
    writeln!(w, "{head}")?;
    for r in rows {
        let dt = r.dt_us.map(|d| format!("{:.3}", d as f64 / 1000.0)).unwrap_or_default();
        let data = if r.rtr { "RTR".to_string() } else { crate::canopen::types::hex_bytes(&r.data[..r.len as usize]) };
        let mut line = format!("{:.6},{dt},{data}", r.t_us as f64 / 1e6);
        for v in &r.values {
            line.push(',');
            if let Some(v) = v {
                line.push_str(&v.to_string());
            }
        }
        writeln!(w, "{line}")?;
    }
    Ok(())
}

// ------------------------------------------------------------------ sparklines

/// Points per sparkline.
pub const SPARK_POINTS: usize = 128;

/// A fixed ring of (time s, value): no allocation after it is made. One
/// point per time bucket (window / (points - 1)), so the ring spans the
/// window.
#[derive(Clone, Debug)]
pub struct Sparkline {
    pts: [(f64, f64); SPARK_POINTS],
    head: usize,
    len: usize,
    window_s: f64,
}

impl Sparkline {
    pub fn new(window_s: f64) -> Sparkline {
        Sparkline { pts: [(0.0, 0.0); SPARK_POINTS], head: 0, len: 0, window_s: window_s.max(0.1) }
    }

    pub fn window_s(&self) -> f64 {
        self.window_s
    }

    pub fn set_window(&mut self, s: f64) {
        self.window_s = s.max(0.1);
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.head = 0;
    }

    pub fn push(&mut self, t: f64, v: f64) {
        // One point per time bucket of window / (points - 1): the newest
        // value of a bucket replaces the one before.
        let step = self.window_s / (SPARK_POINTS - 1) as f64;
        if self.len > 0 {
            let last = (self.head + SPARK_POINTS - 1) % SPARK_POINTS;
            if t < self.pts[last].0 {
                // Time went back (a new connection or a seek).
                self.clear();
            } else if (t / step).floor() == (self.pts[last].0 / step).floor() {
                self.pts[last] = (t, v);
                return;
            }
        }
        self.pts[self.head] = (t, v);
        self.head = (self.head + 1) % SPARK_POINTS;
        self.len = (self.len + 1).min(SPARK_POINTS);
    }

    /// The points, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        let start = (self.head + SPARK_POINTS - self.len) % SPARK_POINTS;
        (0..self.len).map(move |i| self.pts[(start + i) % SPARK_POINTS])
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The sparklines of the signals on screen, fed from every frame.
#[derive(Debug, Default)]
pub struct SparkSet {
    items: Vec<(MsgKey, UserSignal, Sparkline, u64)>,
    frame: u64,
}

/// Most sparklines kept.
pub const MAX_SPARKS: usize = 64;

impl SparkSet {
    /// The sparkline of `sig` (made once; marked as shown).
    pub fn ensure(&mut self, key: MsgKey, sig: &UserSignal, window_s: f64) -> &Sparkline {
        let frame = self.frame;
        let i = match self.items.iter().position(|(k, s, _, _)| *k == key && s.start_bit == sig.start_bit && s.bits == sig.bits && s.big_endian == sig.big_endian && s.kind == sig.kind && s.factor == sig.factor && s.offset == sig.offset) {
            Some(i) => i,
            None => {
                if self.items.len() >= MAX_SPARKS {
                    // Drop the one shown least recently.
                    if let Some(old) = self.items.iter().enumerate().min_by_key(|(_, it)| it.3).map(|(i, _)| i) {
                        self.items.swap_remove(old);
                    }
                }
                self.items.push((key, sig.clone(), Sparkline::new(window_s), frame));
                self.items.len() - 1
            }
        };
        let it = &mut self.items[i];
        it.3 = frame;
        if it.2.window_s() != window_s.max(0.1) {
            it.2.set_window(window_s);
        }
        &self.items[i].2
    }

    /// Count a drawn frame (for least-recently-shown).
    pub fn tick(&mut self) {
        self.frame += 1;
    }

    pub fn on_frame(&mut self, f: &CanFrame) {
        if f.rtr {
            return;
        }
        let key = MsgKey::of(f);
        let t = f.timestamp_us as f64 / 1e6;
        for (k, s, line, _) in &mut self.items {
            if *k == key {
                if let Some(v) = s.value(f.payload()) {
                    line.push(t, v);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        for it in &mut self.items {
            it.2.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::Dir;

    fn fr(id: u32, t_ms: u64, data: &[u8]) -> CanFrame {
        let mut f = CanFrame::new(id, data);
        f.timestamp_us = t_ms * 1000;
        f.dir = Dir::Rx;
        f
    }

    fn key(id: u32) -> MsgKey {
        MsgKey { id, extended: false }
    }

    /// 0x181 bit 0 toggles; 0x281 bit 9 follows it, bit 3 is inverted,
    /// bit 5 is random-ish (half), bit 7 never toggles.
    #[test]
    fn similar_bits_known_answer() {
        let mut frames = Vec::new();
        for i in 0..40u64 {
            let r = (i % 2) as u8;
            let half = ((i / 2) % 2) as u8;
            frames.push(fr(0x181, i * 10, &[r]));
            let b0 = ((1 - r) << 3) | (half << 5) | 0x80;
            frames.push(fr(0x281, i * 10 + 5, &[b0, r << 1]));
        }
        let cancel = AtomicBool::new(false);
        let v = similar_bits(&frames, BitRef { key: key(0x181), bit: 0 }, &cancel, None).unwrap();
        let find = |id: u32, bit: u32| v.iter().find(|m| m.key == key(id) && m.bit == bit).copied();
        // Compared at each of the 40 reference frames, except the first
        // (0x281 not seen yet).
        let follow = find(0x281, 9).unwrap();
        assert_eq!(follow.compared, 39);
        assert_eq!(follow.match_pct(), 0.0, "0x281 is one frame behind: it equals the previous reference value");
        let inv = find(0x281, 3).unwrap();
        assert_eq!(inv.match_pct(), 100.0, "inverted and one frame behind");
        assert!(find(0x281, 7).is_none(), "never toggled");
        assert!(find(0x181, 0).is_none(), "the reference bit itself is left out");
        // Reference frames come after the other message here: build the
        // same with 0x281 first, so its current value matches.
        let mut frames = Vec::new();
        for i in 0..40u64 {
            let r = (i % 2) as u8;
            let half = ((i / 2) % 2) as u8;
            let b0 = ((1 - r) << 3) | (half << 5) | 0x80;
            frames.push(fr(0x281, i * 10, &[b0, r << 1]));
            frames.push(fr(0x181, i * 10 + 5, &[r]));
        }
        let v = similar_bits(&frames, BitRef { key: key(0x181), bit: 0 }, &cancel, None).unwrap();
        let find = |id: u32, bit: u32| v.iter().find(|m| m.key == key(id) && m.bit == bit).copied();
        let follow = find(0x281, 9).unwrap();
        assert_eq!((follow.equal, follow.compared, follow.match_pct()), (40, 40, 100.0));
        assert_eq!(follow.toggles, 39);
        let inv = find(0x281, 3).unwrap();
        assert_eq!((inv.match_pct(), inv.inverted_pct()), (0.0, 100.0));
        let half = find(0x281, 5).unwrap();
        assert_eq!(half.match_pct(), 50.0);
        // Best first: the two exact ones lead.
        assert!(v[0].best_pct() == 100.0 && v[1].best_pct() == 100.0 && v[2].best_pct() < 100.0);
        cancel.store(true, Ordering::Relaxed);
        assert!(similar_bits(&frames, BitRef { key: key(0x181), bit: 0 }, &cancel, None).is_none());
    }

    #[test]
    fn find_signal_known_answer() {
        // 0x301: byte 2..3 = a 16-bit LE counter that we narrow down.
        let mut cur: HashMap<MsgKey, Vec<u8>> = HashMap::new();
        let set = |cur: &mut HashMap<MsgKey, Vec<u8>>, v: u16, other: u8| {
            let b = v.to_le_bytes();
            cur.insert(key(0x301), vec![0x11, other, b[0], b[1], 0, 0, 0, 0]);
            cur.insert(key(0x401), vec![other, 0x22]);
        };
        set(&mut cur, 1000, 5);
        let mut fs = FindSignal::new(FindSettings::default());
        fs.reset(cur.iter().map(|(k, v)| (*k, v.as_slice())));
        // 0x301: 64 bits + 8 bytes + 7 words + 5 dwords; 0x401: 16 + 2 + 1.
        assert_eq!(fs.candidates().len(), 64 + 8 + 7 + 5 + 16 + 2 + 1);
        assert!(!fs.cut);
        set(&mut cur, 1100, 5);
        fs.step(FindStep::Increased, |k| cur.get(&k).cloned());
        set(&mut cur, 1100, 6);
        fs.step(FindStep::Unchanged, |k| cur.get(&k).cloned());
        fs.step(FindStep::Between(1000.0, 1200.0), |k| cur.get(&k).cloned());
        let c = fs.candidates();
        assert!(c.iter().any(|c| c.key == key(0x301) && c.start_bit == 16 && c.bits == 16 && c.value == 1100.0), "{c:?}");
        let n = c.len();
        fs.step(FindStep::Eq(1100.0), |k| cur.get(&k).cloned());
        assert!(fs.candidates().len() <= n);
        assert_eq!(fs.steps().len(), 4);
        assert!(fs.undo());
        assert_eq!(fs.candidates().len(), n);
        assert_eq!(fs.steps(), ["increased", "unchanged", "between 1000 and 1200"]);
        // A message that is gone drops its candidates.
        fs.step(FindStep::Changed, |_| None);
        assert!(fs.candidates().is_empty());
    }

    #[test]
    fn find_signal_signed_big_endian_and_cut() {
        let mut cur: HashMap<MsgKey, Vec<u8>> = HashMap::new();
        cur.insert(key(0x181), (-300i16).to_be_bytes().to_vec());
        let mut fs = FindSignal::new(FindSettings { sizes: vec![16], big_endian: true, signed: true, ..FindSettings::default() });
        fs.reset(cur.iter().map(|(k, v)| (*k, v.as_slice())));
        assert_eq!(fs.candidates().len(), 1);
        assert_eq!(fs.candidates()[0].start_bit, 7);
        fs.step(FindStep::Lt(0.0), |k| cur.get(&k).cloned());
        assert_eq!(fs.candidates()[0].value, -300.0);
        // Bounded: every start bit of 2048 messages exceeds the limit.
        let many: Vec<(MsgKey, Vec<u8>)> = (0..2048u32).map(|i| (key(i), vec![0u8; 8])).collect();
        let mut fs = FindSignal::new(FindSettings { sizes: (1..=32).collect(), byte_aligned: false, ..FindSettings::default() });
        fs.reset(many.iter().map(|(k, v)| (*k, v.as_slice())));
        assert!(fs.cut && fs.candidates().len() == MAX_CANDIDATES);
        for _ in 0..MAX_UNDO + 3 {
            fs.step(FindStep::Unchanged, |_| Some(vec![0u8; 8]));
        }
        let mut n = 0;
        while fs.undo() {
            n += 1;
        }
        assert_eq!(n, MAX_UNDO);
    }

    #[test]
    fn history_filter_and_csv_layout() {
        let frames = [fr(0x181, 0, &[1, 0]), fr(0x281, 5, &[9]), fr(0x181, 10, &[2, 0]), fr(0x181, 30, &[3, 1]), fr(0x181, 60, &[4, 0])];
        let mut s = UserSignal::new("v", 0x181, false, 0, 8);
        s.unit = "V".to_string();
        let b = UserSignal::new("b8", 0x181, false, 8, 1);
        let sigs = vec![s, b];
        let rows = history(frames.iter(), key(0x181), &sigs, None, 100);
        assert_eq!(rows.iter().map(|r| r.t_us / 1000).collect::<Vec<_>>(), [60, 30, 10, 0], "newest first");
        assert_eq!(rows.iter().map(|r| r.dt_us.map(|d| d / 1000)).collect::<Vec<_>>(), [Some(30), Some(20), Some(10), None]);
        assert_eq!(rows[0].values, [Some(4.0), Some(0.0)]);
        let f = HistFilter { col: 0, op: CmpOp::Gt, value: 2.0 };
        let rows = history(frames.iter(), key(0x181), &sigs, Some(&f), 100);
        assert_eq!(rows.iter().map(|r| r.values[0]).collect::<Vec<_>>(), [Some(4.0), Some(3.0)]);
        assert_eq!(rows[1].dt_us, Some(20_000), "the delta counts every frame, also the filtered ones");
        let f = HistFilter { col: 1, op: CmpOp::Eq, value: 1.0 };
        assert_eq!(history(frames.iter(), key(0x181), &sigs, Some(&f), 100).len(), 1);
        assert_eq!(history(frames.iter(), key(0x181), &sigs, None, 2).len(), 2, "limit");
        let mut out = Vec::new();
        let rows = history(frames.iter(), key(0x181), &sigs, Some(&HistFilter { col: 0, op: CmpOp::Gt, value: 2.0 }), 100);
        write_history_csv(&mut out, &rows, &sigs).unwrap();
        let t = String::from_utf8(out).unwrap();
        assert_eq!(t, "time_s,delta_ms,data,v [V],b8\n0.060000,30.000,04 00,4,0\n0.030000,20.000,03 01,3,1\n");
    }

    #[test]
    fn sparkline_ring_is_fixed_and_spans_the_window() {
        let mut s = Sparkline::new(10.0);
        for i in 0..10_000 {
            s.push(i as f64 * 0.01, i as f64);
        }
        assert_eq!(s.len(), SPARK_POINTS);
        let pts: Vec<(f64, f64)> = s.iter().collect();
        assert!(pts.windows(2).all(|w| w[0].0 <= w[1].0), "oldest first");
        let span = pts.last().unwrap().0 - pts[0].0;
        assert!(span >= 9.0, "span {span}");
        assert_eq!(pts.last().unwrap().1, 9999.0, "the newest value is kept");
        s.push(1.0, 1.0);
        assert_eq!(s.len(), 1, "time went back");
        let mut set = SparkSet::default();
        let sig = UserSignal::new("a", 0x181, false, 0, 8);
        set.ensure(key(0x181), &sig, 10.0);
        set.on_frame(&fr(0x181, 100, &[7]));
        set.on_frame(&fr(0x281, 100, &[8]));
        assert_eq!(set.ensure(key(0x181), &sig, 10.0).iter().collect::<Vec<_>>(), [(0.1, 7.0)]);
        assert_eq!(set.len(), 1);
    }
}
