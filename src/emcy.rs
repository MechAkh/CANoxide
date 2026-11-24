//! Emergency (EMCY) messages: the history of the received EMCY frames (a
//! bounded buffer, fed with the network overview, live and in a replay),
//! and the error history object 0x1003 of a node (read and clear).

use crate::can::{CanFrame, Dir};
use crate::canopen::decode::{emcy_code_text, error_register_words};
use crate::canopen::task::{ArrayReadTask, ReadsReport, SdoStep};
use crate::canopen::COB_EMCY;
use crate::canopen::types::{hex_bytes, le_u64};
use std::collections::VecDeque;

/// Entries the history keeps; the oldest are dropped.
pub const EMCY_HISTORY_CAP: usize = 10_000;

/// One received EMCY frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmcyEntry {
    /// Bus time of the frame (us).
    pub time_us: u64,
    pub node: u8,
    pub code: u16,
    pub register: u8,
    /// Manufacturer-specific bytes 3..7 (zero when the frame was shorter).
    pub mfr: [u8; 5],
}

impl EmcyEntry {
    /// The entry of a received frame, `None` for any other frame.
    pub fn from_frame(f: &CanFrame) -> Option<EmcyEntry> {
        if f.extended || f.rtr || f.dir != Dir::Rx || f.id & !0x7F != COB_EMCY {
            return None;
        }
        let node = (f.id & 0x7F) as u8;
        let d = f.payload();
        if node == 0 || d.len() < 3 {
            return None;
        }
        let mut mfr = [0u8; 5];
        for (i, b) in d[3..].iter().take(5).enumerate() {
            mfr[i] = *b;
        }
        Some(EmcyEntry { time_us: f.timestamp_us, node, code: u16::from_le_bytes([d[0], d[1]]), register: d[2], mfr })
    }

    pub fn text(&self) -> &'static str {
        emcy_code_text(self.code)
    }

    /// The set bits of the error register as words ("generic, communication").
    pub fn register_words(&self) -> String {
        error_register_words(self.register).join(", ")
    }

    pub fn mfr_hex(&self) -> String {
        hex_bytes(&self.mfr)
    }

    /// Code 0x0000: error reset / no error.
    pub fn is_reset(&self) -> bool {
        self.code == 0
    }
}

/// The received EMCY frames, oldest first, at most `cap` entries.
#[derive(Clone, Debug)]
pub struct EmcyHistory {
    entries: VecDeque<EmcyEntry>,
    cap: usize,
    /// Entries dropped because the buffer was full.
    pub dropped: u64,
}

impl Default for EmcyHistory {
    fn default() -> Self {
        EmcyHistory::with_capacity(EMCY_HISTORY_CAP)
    }
}

impl EmcyHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        EmcyHistory { entries: VecDeque::new(), cap: cap.max(1), dropped: 0 }
    }

    pub fn push(&mut self, e: EmcyEntry) {
        while self.entries.len() >= self.cap {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(e);
    }

    /// Take a frame when it is an EMCY.
    pub fn on_frame(&mut self, f: &CanFrame) {
        if let Some(e) = EmcyEntry::from_frame(f) {
            self.push(e);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.dropped = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &EmcyEntry> {
        self.entries.iter()
    }

    /// Newest first, only `node` when given.
    pub fn newest_first(&self, node: Option<u8>) -> Vec<EmcyEntry> {
        self.entries.iter().rev().filter(|e| node.is_none_or(|n| e.node == n)).copied().collect()
    }

    /// The nodes that sent an EMCY, sorted.
    pub fn nodes(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self.entries.iter().map(|e| e.node).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// CSV of EMCY entries (as given, for example newest first). `name` gives
/// the device name of a node.
pub fn history_csv(entries: &[EmcyEntry], name: impl Fn(u8) -> Option<String>) -> String {
    let mut s = String::from("time_s,node,device,code,text,error_register,register_bits,manufacturer_bytes\n");
    for e in entries {
        let row = [
            format!("{:.6}", e.time_us as f64 / 1e6),
            e.node.to_string(),
            name(e.node).unwrap_or_default(),
            format!("0x{:04X}", e.code),
            e.text().to_string(),
            format!("0x{:02X}", e.register),
            e.register_words(),
            e.mfr_hex(),
        ];
        let cells: Vec<String> = row.iter().map(|c| csv_cell(c)).collect();
        s.push_str(&cells.join(","));
        s.push('\n');
    }
    s
}

/// A CSV cell, quoted when it holds a comma, a quote or a line break.
pub fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// One entry of the error history 0x1003 of a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorField {
    /// Sub-index (1 = newest).
    pub number: u8,
    /// The 32-bit value.
    pub raw: u32,
}

impl ErrorField {
    /// Error code: the low 16 bits.
    pub fn code(&self) -> u16 {
        self.raw as u16
    }

    /// Additional information: the high 16 bits.
    pub fn info(&self) -> u16 {
        (self.raw >> 16) as u16
    }

    pub fn text(&self) -> &'static str {
        emcy_code_text(self.code())
    }
}

/// The task that reads 0x1003:00 and then 0x1003:01..n.
pub fn error_history_task(node: u8) -> ArrayReadTask {
    ArrayReadTask::new("read error history", node, vec![0x1003])
}

/// The error history of a finished `error_history_task`, or the error text
/// (with the abort text) when 0x1003:00 could not be read. A failed entry
/// read is reported as an error too.
pub fn error_history(r: &ReadsReport) -> Result<Vec<ErrorField>, String> {
    let Some(first) = r.reads.iter().find(|x| x.index == 0x1003 && x.sub == 0) else {
        return Err(if r.cancelled { "cancelled".to_string() } else { "0x1003:00 not read".to_string() });
    };
    if first.data.is_none() {
        return Err(format!("0x1003:00: {}", first.error.clone().unwrap_or_default()));
    }
    let mut v = Vec::new();
    for x in r.reads.iter().filter(|x| x.index == 0x1003 && x.sub >= 1) {
        match &x.data {
            Some(d) => v.push(ErrorField { number: x.sub, raw: le_u64(d) as u32 }),
            None => return Err(format!("0x1003:{:02}: {}", x.sub, x.error.clone().unwrap_or_default())),
        }
    }
    Ok(v)
}

/// The write that clears the error history: 0 to 0x1003:00.
pub fn clear_history_step(node: u8) -> SdoStep {
    SdoStep::write(node, 0x1003, 0, vec![0], "clear error history (0x1003:00 = 0)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emcy(node: u8, code: u16, reg: u8, t: u64) -> CanFrame {
        let c = code.to_le_bytes();
        let mut f = CanFrame::new(COB_EMCY + node as u32, &[c[0], c[1], reg, 1, 2, 3, 4, 5]);
        f.dir = Dir::Rx;
        f.timestamp_us = t;
        f
    }

    #[test]
    fn entry_from_frame() {
        let e = EmcyEntry::from_frame(&emcy(5, 0x8130, 0x11, 42)).unwrap();
        assert_eq!((e.node, e.code, e.register, e.time_us), (5, 0x8130, 0x11, 42));
        assert_eq!(e.mfr, [1, 2, 3, 4, 5]);
        assert_eq!(e.text(), "Life guard or heartbeat error");
        assert_eq!(e.register_words(), "generic, communication");
        assert_eq!(e.mfr_hex(), "01 02 03 04 05");
        // SYNC (0x080) and a TX frame are not EMCY entries.
        let mut sync = CanFrame::new(0x080, &[]);
        sync.dir = Dir::Rx;
        assert!(EmcyEntry::from_frame(&sync).is_none());
        let mut tx = emcy(5, 0x8130, 0x11, 1);
        tx.dir = Dir::Tx;
        assert!(EmcyEntry::from_frame(&tx).is_none());
    }

    #[test]
    fn history_is_bounded_and_drops_the_oldest() {
        let mut h = EmcyHistory::new();
        for i in 0..(EMCY_HISTORY_CAP as u64 + 25) {
            h.on_frame(&emcy(3, 0x1000, 1, i));
        }
        assert_eq!(h.len(), EMCY_HISTORY_CAP);
        assert_eq!(h.dropped, 25);
        assert_eq!(h.iter().next().unwrap().time_us, 25);
        assert_eq!(h.newest_first(None)[0].time_us, EMCY_HISTORY_CAP as u64 + 24);
        let mut small = EmcyHistory::with_capacity(2);
        small.on_frame(&emcy(1, 1, 0, 1));
        small.on_frame(&emcy(2, 2, 0, 2));
        small.on_frame(&emcy(1, 3, 0, 3));
        assert_eq!(small.newest_first(None).iter().map(|e| e.code).collect::<Vec<_>>(), vec![3, 2]);
        assert_eq!(small.newest_first(Some(1)).len(), 1);
        assert_eq!(small.nodes(), vec![1, 2]);
        small.clear();
        assert!(small.is_empty());
    }

    #[test]
    fn csv_quotes_cells() {
        let e = EmcyEntry::from_frame(&emcy(2, 0x2100, 0x03, 1_500_000)).unwrap();
        let csv = history_csv(&[e], |_| Some("Drive, left".to_string()));
        let line = csv.lines().nth(1).unwrap();
        assert_eq!(line, "1.500000,2,\"Drive, left\",0x2100,\"Current, device input side\",0x03,\"generic, current\",01 02 03 04 05");
    }

    #[test]
    fn simulator_error_history() {
        use crate::can::sim::SimNode;
        use crate::canopen::task::TaskResult;
        use crate::session::{Ctl, Session};
        let mut n = SimNode::builtin(1);
        assert_eq!(n.error_history(), vec![0x0011_8130, 0x5000]);
        let mut out = Vec::new();
        n.emcy(0x2310, 0x03, &mut out);
        assert_eq!(n.error_history()[0], 0x2310);
        // An error reset is sent but not stored.
        n.emcy(0x0000, 0x00, &mut out);
        assert_eq!(n.error_history().len(), 3);
        assert_eq!(n.write(0x1003, 0, vec![1]), Err(0x0609_0030));
        assert_eq!(n.write(0x1003, 0, vec![0]), Ok(()));
        assert!(n.error_history().is_empty());
        let mut s = Session::sim(SimNode::builtin(1), false);
        let r = match s.run_task(Box::new(error_history_task(1)), &mut Ctl::default()).unwrap() {
            TaskResult::Reads(r) => r,
            _ => panic!(),
        };
        let h = error_history(&r).unwrap();
        assert_eq!(h.iter().map(|e| e.code()).collect::<Vec<_>>(), vec![0x8130, 0x5000]);
    }

    #[test]
    fn error_history_of_a_report() {
        use crate::canopen::task::ReadResult;
        let ok = |sub: u8, d: Vec<u8>| ReadResult { index: 0x1003, sub, data: Some(d), error: None, abort_code: None };
        let r = ReadsReport { node: 1, reads: vec![ok(0, vec![2]), ok(1, 0x0011_8130u32.to_le_bytes().to_vec()), ok(2, 0x5000u32.to_le_bytes().to_vec())], ..Default::default() };
        let h = error_history(&r).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].number, h[0].code(), h[0].info()), (1, 0x8130, 0x0011));
        assert_eq!(h[1].text(), "Device hardware");
        let bad = ReadsReport {
            node: 1,
            reads: vec![ReadResult { index: 0x1003, sub: 0, data: None, error: Some("abort 0x06020000".into()), abort_code: Some(0x0602_0000) }],
            ..Default::default()
        };
        assert!(error_history(&bad).unwrap_err().contains("0x06020000"));
        assert_eq!(clear_history_step(4).data, vec![0]);
    }
}
