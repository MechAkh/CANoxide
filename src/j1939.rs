//! SAE J1939 on 29-bit identifiers, read only: the ID split (priority, PGN,
//! source and destination address), short PGN names, the transport protocol
//! (BAM and RTS/CTS, followed on the bus in both directions), the address
//! claim NAME, DM1/DM2 faults, the decoded text of a frame and the state of
//! the J1939 view (`J1939Monitor`).
//!
//! Everything here is pure and deterministic: the time comes from the frame
//! time stamps. Nothing here builds a frame to send.

use crate::can::CanFrame;
use std::collections::BTreeMap;

pub const PGN_TSC1: u32 = 0;
pub const PGN_ACKM: u32 = 59392;
pub const PGN_RQST: u32 = 59904;
pub const PGN_TP_DT: u32 = 60160;
pub const PGN_TP_CM: u32 = 60416;
pub const PGN_AC: u32 = 60928;
pub const PGN_ETP_DT: u32 = 50944;
pub const PGN_ETP_CM: u32 = 51200;
pub const PGN_DM1: u32 = 65226;
pub const PGN_DM2: u32 = 65227;

/// The global destination address.
pub const GLOBAL: u8 = 255;
/// The null address: an address claim from it is "cannot claim".
pub const NULL_ADDR: u8 = 254;

/// Time between two BAM packets after which the session is dropped.
pub const BAM_TIMEOUT_US: u64 = 750_000;
/// Time between two frames of an RTS/CTS session after which it is dropped.
pub const RTS_TIMEOUT_US: u64 = 1_250_000;
/// The largest message of the transport protocol (255 packets of 7 bytes).
pub const MAX_TP_SIZE: usize = 1785;

/// The fields of a 29-bit J1939 identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct J1939Id {
    pub priority: u8,
    pub edp: bool,
    pub dp: bool,
    pub pf: u8,
    pub ps: u8,
    pub sa: u8,
}

impl J1939Id {
    /// Split a 29-bit identifier (no check).
    pub fn split(id: u32) -> J1939Id {
        J1939Id {
            priority: ((id >> 26) & 7) as u8,
            edp: id & (1 << 25) != 0,
            dp: id & (1 << 24) != 0,
            pf: (id >> 16) as u8,
            ps: (id >> 8) as u8,
            sa: id as u8,
        }
    }

    /// The J1939 fields of a frame ID: `None` for a standard frame and for
    /// EDP = 1 with DP = 1 (not J1939: a raw extended frame).
    pub fn parse(id: u32, extended: bool) -> Option<J1939Id> {
        if !extended {
            return None;
        }
        let j = J1939Id::split(id);
        (!(j.edp && j.dp)).then_some(j)
    }

    /// PF < 240: PS is the destination address.
    pub fn is_pdu1(&self) -> bool {
        self.pf < 240
    }

    pub fn pgn(&self) -> u32 {
        let base = ((self.edp as u32) << 17) | ((self.dp as u32) << 16) | ((self.pf as u32) << 8);
        if self.is_pdu1() {
            base
        } else {
            base | self.ps as u32
        }
    }

    /// The destination address (255 = all, also for every PDU2 frame).
    pub fn da(&self) -> u8 {
        if self.is_pdu1() {
            self.ps
        } else {
            GLOBAL
        }
    }
}

/// Short names of common PGNs (sorted by PGN).
const NAMES: &[(u32, &str)] = &[
    (0, "TSC1"),
    (50944, "ETP.DT"),
    (51200, "ETP.CM"),
    (59392, "ACKM"),
    (59904, "RQST"),
    (60160, "TP.DT"),
    (60416, "TP.CM"),
    (60928, "AC"),
    (61184, "PropA"),
    (61441, "EBC1"),
    (61442, "ETC1"),
    (61443, "EEC2"),
    (61444, "EEC1"),
    (61445, "ETC2"),
    (65226, "DM1"),
    (65227, "DM2"),
    (65228, "DM3"),
    (65235, "DM11"),
    (65242, "SOFT"),
    (65248, "VD"),
    (65253, "HOURS"),
    (65254, "TD"),
    (65259, "CI"),
    (65260, "VI"),
    (65262, "ET1"),
    (65263, "EFL/P1"),
    (65265, "CCVS"),
    (65266, "LFE"),
    (65269, "AMB"),
    (65270, "IC1"),
    (65271, "VEP1"),
    (65276, "DD"),
];

/// The short name of a PGN; 65280..=65535 is PropB.
pub fn pgn_name(pgn: u32) -> Option<&'static str> {
    if (65280..=65535).contains(&pgn) {
        return Some("PropB");
    }
    NAMES.binary_search_by_key(&pgn, |&(p, _)| p).ok().map(|i| NAMES[i].1)
}

/// "PGN 65226 DM1", or "PGN 12345" without a name.
pub fn pgn_text(pgn: u32) -> String {
    match pgn_name(pgn) {
        Some(n) => format!("PGN {pgn} {n}"),
        None => format!("PGN {pgn}"),
    }
}

/// "all" for 255, else the address in decimal.
pub fn dest_text(da: u8) -> String {
    if da == GLOBAL {
        "all".to_string()
    } else {
        da.to_string()
    }
}

fn pgn24(d: &[u8]) -> u32 {
    d[0] as u32 | (d[1] as u32) << 8 | (d[2] as u32) << 16
}

/// True when a frame with ID `frame_id` carries the PGN of `sig_id` (any
/// priority, any source address, PDU1 any destination), from `sa` when it
/// is given. Both IDs are 29-bit.
pub fn pgn_matches(sig_id: u32, sa: Option<u8>, frame_id: u32) -> bool {
    let (Some(s), Some(f)) = (J1939Id::parse(sig_id, true), J1939Id::parse(frame_id, true)) else { return false };
    s.pgn() == f.pgn() && sa.is_none_or(|a| a == f.sa)
}

// ---------------------------------------------------------------- NAME

/// The 64-bit NAME of an address claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Name {
    pub raw: u64,
    pub identity: u32,
    pub manufacturer: u16,
    pub ecu_instance: u8,
    pub function_instance: u8,
    pub function: u8,
    pub vehicle_system: u8,
    pub vehicle_system_instance: u8,
    pub industry_group: u8,
    pub arbitrary_address: bool,
}

impl Name {
    /// The NAME of 8 data bytes (little endian).
    pub fn decode(d: &[u8]) -> Option<Name> {
        let b: [u8; 8] = d.get(..8)?.try_into().ok()?;
        Some(Name::from_u64(u64::from_le_bytes(b)))
    }

    pub fn from_u64(raw: u64) -> Name {
        let bits = |lo: u32, n: u32| ((raw >> lo) & ((1u64 << n) - 1)) as u32;
        Name {
            raw,
            identity: bits(0, 21),
            manufacturer: bits(21, 11) as u16,
            ecu_instance: bits(32, 3) as u8,
            function_instance: bits(35, 5) as u8,
            function: bits(40, 8) as u8,
            vehicle_system: bits(49, 7) as u8,
            vehicle_system_instance: bits(56, 4) as u8,
            industry_group: bits(60, 3) as u8,
            arbitrary_address: bits(63, 1) != 0,
        }
    }

    /// "NAME mfr 123 fn 4 id 5678".
    pub fn short_text(&self) -> String {
        format!("NAME mfr {} fn {} id {}", self.manufacturer, self.function, self.identity)
    }
}

// ---------------------------------------------------------------- DM1/DM2

/// One diagnostic trouble code of a DM1/DM2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dtc {
    pub spn: u32,
    pub fmi: u8,
    /// The SPN conversion method bit.
    pub cm: bool,
    pub oc: u8,
}

/// A DM1 or DM2 message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmMessage {
    /// Byte 0: MIL (bits 7-6), red stop (5-4), amber (3-2), protect (1-0).
    pub lamps: u8,
    pub flash: u8,
    /// The faults; empty for "no active fault".
    pub faults: Vec<Dtc>,
}

/// The lamp names in the order of byte 0 (bit pair 7-6 first).
pub const LAMPS: [&str; 4] = ["MIL", "red stop", "amber", "protect"];

/// The lamps of byte 0 that are on (bit pair = 1).
pub fn lamps_on(lamps: u8) -> Vec<&'static str> {
    LAMPS.iter().enumerate().filter(|(i, _)| (lamps >> (6 - 2 * i)) & 3 == 1).map(|(_, n)| *n).collect()
}

/// Decode a DM1/DM2 payload (a single frame or a reassembled message).
pub fn decode_dm(d: &[u8]) -> Option<DmMessage> {
    if d.len() < 2 {
        return None;
    }
    let mut faults = Vec::new();
    for c in d[2..].as_chunks::<4>().0 {
        let spn = c[0] as u32 | (c[1] as u32) << 8 | ((c[2] as u32 >> 5) & 7) << 16;
        let fmi = c[2] & 0x1F;
        // SPN 0 FMI 0 = no active fault; all ones = padding.
        if (spn == 0 && fmi == 0) || *c == [0xFF; 4] {
            continue;
        }
        faults.push(Dtc { spn, fmi, cm: c[3] & 0x80 != 0, oc: c[3] & 0x7F });
    }
    Some(DmMessage { lamps: d[0], flash: d[1], faults })
}

/// "DM1: no active fault" / "DM1: SPN 100 FMI 1 OC 3, ..." + "; lamps: amber".
pub fn dm_text(label: &str, m: &DmMessage) -> String {
    let mut s = if m.faults.is_empty() {
        format!("{label}: no active fault")
    } else {
        let f: Vec<String> = m.faults.iter().map(|f| format!("SPN {} FMI {} OC {}", f.spn, f.fmi, f.oc)).collect();
        format!("{label}: {}", f.join(", "))
    };
    let on = lamps_on(m.lamps);
    if !on.is_empty() {
        s.push_str(&format!("; lamps: {}", on.join(", ")));
    }
    s
}

fn dm_label(pgn: u32) -> Option<&'static str> {
    match pgn {
        PGN_DM1 => Some("DM1"),
        PGN_DM2 => Some("DM2"),
        _ => None,
    }
}

// ---------------------------------------------------------------- transport

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TpKind {
    Bam,
    RtsCts,
}

/// A reassembled transport protocol message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TpMessage {
    pub pgn: u32,
    pub sa: u8,
    pub da: u8,
    pub priority: u8,
    pub data: Vec<u8>,
    pub first_us: u64,
    pub last_us: u64,
    pub kind: TpKind,
}

/// Counters of the transport protocol sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TpCounters {
    pub complete: u64,
    /// Replaced by a new BAM or RTS of the same pair, or closed by an EOM
    /// ACK before all data was seen.
    pub incomplete: u64,
    pub timeout: u64,
    pub seq_error: u64,
    pub abort: u64,
}

impl TpCounters {
    pub fn errors(&self) -> u64 {
        self.timeout + self.seq_error + self.abort
    }

    /// "TP: 12 complete, 1 incomplete, 0 errors".
    pub fn text(&self) -> String {
        format!("TP: {} complete, {} incomplete, {} errors", self.complete, self.incomplete, self.errors())
    }
}

/// What one frame did to the transport protocol state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TpStep {
    /// (sequence number, packets) of a TP.DT of a known session.
    pub packet: Option<(u8, u8)>,
    /// The message this frame completed.
    pub done: Option<TpMessage>,
}

#[derive(Clone, Debug)]
struct Session {
    kind: TpKind,
    pgn: u32,
    size: usize,
    packets: u8,
    next: u8,
    priority: u8,
    data: Vec<u8>,
    first_us: u64,
    last_us: u64,
}

impl Session {
    fn timeout_us(&self) -> u64 {
        match self.kind {
            TpKind::Bam => BAM_TIMEOUT_US,
            TpKind::RtsCts => RTS_TIMEOUT_US,
        }
    }
}

/// The transport protocol reassembler. A session is keyed by (originator
/// SA, DA): one BAM (DA 255) and one RTS/CTS session per pair. Feed it every
/// frame in time order (both directions).
#[derive(Clone, Debug, Default)]
pub struct TpReassembler {
    sessions: BTreeMap<(u8, u8), Session>,
    pub counters: TpCounters,
}

impl TpReassembler {
    pub fn new() -> Self {
        TpReassembler::default()
    }

    pub fn reset(&mut self) {
        *self = TpReassembler::default();
    }

    /// The open sessions.
    pub fn open_sessions(&self) -> usize {
        self.sessions.len()
    }

    /// Drop the sessions whose last frame is older than their timeout.
    pub fn expire(&mut self, now_us: u64) {
        let before = self.sessions.len();
        self.sessions.retain(|_, s| now_us.saturating_sub(s.last_us) <= s.timeout_us());
        self.counters.timeout += (before - self.sessions.len()) as u64;
    }

    pub fn on_frame(&mut self, f: &CanFrame) -> TpStep {
        let Some(j) = J1939Id::parse(f.id, f.extended) else { return TpStep::default() };
        if f.rtr {
            return TpStep::default();
        }
        let now = f.timestamp_us;
        self.expire(now);
        let d = f.payload();
        let (sa, da) = (j.sa, j.da());
        match j.pgn() {
            PGN_TP_CM if d.len() >= 8 => {
                let pgn = pgn24(&d[5..8]);
                match d[0] {
                    32 | 16 => {
                        let kind = if d[0] == 32 { TpKind::Bam } else { TpKind::RtsCts };
                        // A BAM goes to all, an RTS to one address.
                        if (kind == TpKind::Bam) != (da == GLOBAL) {
                            return TpStep::default();
                        }
                        let size = u16::from_le_bytes([d[1], d[2]]) as usize;
                        if size == 0 || size > MAX_TP_SIZE || d[3] == 0 {
                            return TpStep::default();
                        }
                        let s = Session { kind, pgn, size, packets: d[3], next: 1, priority: j.priority, data: Vec::with_capacity(size), first_us: now, last_us: now };
                        if self.sessions.insert((sa, da), s).is_some() {
                            self.counters.incomplete += 1;
                        }
                    }
                    // CTS and EOM ACK come from the receiver.
                    17 => {
                        if let Some(s) = self.sessions.get_mut(&(da, sa)).filter(|s| s.kind == TpKind::RtsCts) {
                            s.last_us = now;
                            if d[1] > 0 && d[2] >= 1 {
                                s.next = d[2];
                                s.data.truncate((d[2] as usize - 1) * 7);
                            }
                        }
                    }
                    19 => {
                        if self.sessions.get(&(da, sa)).is_some_and(|s| s.kind == TpKind::RtsCts) {
                            self.sessions.remove(&(da, sa));
                            self.counters.incomplete += 1;
                        }
                    }
                    255 => {
                        let key = if self.sessions.get(&(sa, da)).is_some_and(|s| s.kind == TpKind::RtsCts) { (sa, da) } else { (da, sa) };
                        if self.sessions.get(&key).is_some_and(|s| s.kind == TpKind::RtsCts) {
                            self.sessions.remove(&key);
                            self.counters.abort += 1;
                        }
                    }
                    _ => {}
                }
                TpStep::default()
            }
            PGN_TP_DT if !d.is_empty() => {
                let key = (sa, da);
                let Some(s) = self.sessions.get_mut(&key) else { return TpStep::default() };
                let seq = d[0];
                if seq != s.next {
                    self.sessions.remove(&key);
                    self.counters.seq_error += 1;
                    return TpStep::default();
                }
                s.data.extend_from_slice(&d[1..]);
                s.next = s.next.wrapping_add(1);
                s.last_us = now;
                let packet = Some((seq, s.packets));
                if s.data.len() < s.size {
                    return TpStep { packet, done: None };
                }
                let Some(mut s) = self.sessions.remove(&key) else { return TpStep { packet, done: None } };
                // The padding of the last packet is cut.
                s.data.truncate(s.size);
                self.counters.complete += 1;
                let msg = TpMessage { pgn: s.pgn, sa, da, priority: s.priority, data: s.data, first_us: s.first_us, last_us: now, kind: s.kind };
                TpStep { packet, done: Some(msg) }
            }
            _ => TpStep::default(),
        }
    }
}

// ---------------------------------------------------------------- text

/// The decoded text of a J1939 frame, for example
/// "PGN 59904 RQST  P6  -> 0  req PGN 65259 CI". `tp` is what the frame did
/// to the transport protocol state (a stateful caller), `None` for the
/// single-frame text.
pub fn frame_text(j: &J1939Id, data: &[u8], tp: Option<&TpStep>) -> String {
    let pgn = j.pgn();
    let mut s = format!("{}  P{}  -> {}", pgn_text(pgn), j.priority, dest_text(j.da()));
    let detail = detail_text(j, data, tp);
    if !detail.is_empty() {
        s.push_str("  ");
        s.push_str(&detail);
    }
    s
}

fn detail_text(j: &J1939Id, d: &[u8], tp: Option<&TpStep>) -> String {
    match j.pgn() {
        PGN_RQST if d.len() >= 3 => format!("req {}", pgn_text(pgn24(d))),
        PGN_ACKM if d.len() >= 8 => {
            let what = match d[0] {
                0 => "ACK".to_string(),
                1 => "NACK".to_string(),
                2 => "access denied".to_string(),
                3 => "busy".to_string(),
                c => format!("ACK control {c}"),
            };
            format!("{what} {}", pgn_text(pgn24(&d[5..8])))
        }
        PGN_AC => match Name::decode(d) {
            Some(n) if j.sa == NULL_ADDR => format!("cannot claim  {}", n.short_text()),
            Some(n) => n.short_text(),
            None => "address claim (short frame)".to_string(),
        },
        PGN_TP_CM if d.len() >= 8 => {
            let p = pgn_text(pgn24(&d[5..8]));
            let size = u16::from_le_bytes([d[1], d[2]]);
            match d[0] {
                32 => format!("BAM {p}, {size} bytes, {} packets", d[3]),
                16 => format!("RTS {p}, {size} bytes, {} packets", d[3]),
                17 if d[1] == 0 => format!("CTS hold {p}"),
                17 => format!("CTS {} packets from {} {p}", d[1], d[2]),
                19 => format!("EOM ACK {p}, {size} bytes, {} packets", d[3]),
                255 => format!("abort {p}, reason {}", d[1]),
                c => format!("TP.CM control {c}"),
            }
        }
        PGN_TP_CM => "TP.CM (short frame)".to_string(),
        PGN_TP_DT if !d.is_empty() => {
            let mut s = match tp.and_then(|t| t.packet) {
                Some((seq, n)) => format!("TP.DT {seq}/{n}"),
                None => format!("TP.DT {}", d[0]),
            };
            if let Some(m) = tp.and_then(|t| t.done.as_ref()) {
                s.push_str(&format!("  complete: {}, {} bytes", pgn_text(m.pgn), m.data.len()));
                if let (Some(l), Some(dm)) = (dm_label(m.pgn), decode_dm(&m.data)) {
                    s.push_str("  ");
                    s.push_str(&dm_text(l, &dm));
                }
            }
            s
        }
        PGN_ETP_CM | PGN_ETP_DT => "ETP, not reassembled".to_string(),
        p => match (dm_label(p), decode_dm(d)) {
            (Some(l), Some(dm)) => dm_text(l, &dm),
            _ => String::new(),
        },
    }
}

// ---------------------------------------------------------------- monitor

/// The most parameter group rows of the J1939 view.
pub const MAX_PG_ROWS: usize = 4096;
/// The most fault rows of the J1939 view.
pub const MAX_FAULT_ROWS: usize = 1024;

/// One parameter group of one source address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PgRow {
    pub pgn: u32,
    pub sa: u8,
    /// The destination of the last frame / message.
    pub da: u8,
    pub priority: u8,
    pub count: u64,
    /// Time between the last two frames / messages.
    pub period_us: Option<u64>,
    pub first_us: u64,
    pub last_us: u64,
    /// The data of the last frame / message.
    pub data: Vec<u8>,
    /// A reassembled multi-frame message.
    pub multi: bool,
}

/// One source address seen on the bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddrRow {
    pub sa: u8,
    /// The NAME of the last address claim from this address.
    pub name: Option<Name>,
    pub frames: u64,
    pub last_us: u64,
}

/// One DM1 fault of one source address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultRow {
    pub sa: u8,
    pub spn: u32,
    pub fmi: u8,
    pub oc: u8,
    /// Byte 0 of the last DM1 that listed the fault.
    pub lamps: u8,
    pub first_us: u64,
    pub last_us: u64,
    /// False once a later DM1 of the same address does not list it.
    pub active: bool,
}

/// The state of the J1939 view, fed with every received frame.
#[derive(Clone, Debug, Default)]
pub struct J1939Monitor {
    pgs: BTreeMap<(u32, u8), PgRow>,
    addrs: BTreeMap<u8, AddrRow>,
    faults: BTreeMap<(u8, u32, u8), FaultRow>,
    tp: TpReassembler,
    /// New (PGN, SA) pairs not added because the table was full.
    pub pg_dropped: u64,
    /// New faults not added because the table was full.
    pub fault_dropped: u64,
}

impl J1939Monitor {
    pub fn new() -> Self {
        J1939Monitor::default()
    }

    pub fn clear(&mut self) {
        *self = J1939Monitor::default();
    }

    pub fn is_empty(&self) -> bool {
        self.pgs.is_empty()
    }

    /// The rows by PGN, then SA.
    pub fn pgs(&self) -> impl Iterator<Item = &PgRow> {
        self.pgs.values()
    }

    pub fn pg(&self, pgn: u32, sa: u8) -> Option<&PgRow> {
        self.pgs.get(&(pgn, sa))
    }

    pub fn addrs(&self) -> impl Iterator<Item = &AddrRow> {
        self.addrs.values()
    }

    pub fn faults(&self) -> impl Iterator<Item = &FaultRow> {
        self.faults.values()
    }

    pub fn counters(&self) -> TpCounters {
        self.tp.counters
    }

    pub fn pg_rows(&self) -> usize {
        self.pgs.len()
    }

    pub fn fault_rows(&self) -> usize {
        self.faults.len()
    }

    pub fn on_frame(&mut self, f: &CanFrame) {
        let Some(j) = J1939Id::parse(f.id, f.extended) else { return };
        if f.rtr {
            return;
        }
        let now = f.timestamp_us;
        let d = f.payload();
        let a = self.addrs.entry(j.sa).or_insert(AddrRow { sa: j.sa, name: None, frames: 0, last_us: now });
        a.frames += 1;
        a.last_us = now;
        if j.pgn() == PGN_AC {
            if let Some(n) = Name::decode(d) {
                a.name = Some(n);
            }
        }
        self.put_pg(j.pgn(), j.sa, j.da(), j.priority, d, now, false);
        if j.pgn() == PGN_DM1 {
            self.on_dm1(j.sa, d, now);
        }
        if let Some(m) = self.tp.on_frame(f).done {
            self.put_pg(m.pgn, m.sa, m.da, m.priority, &m.data, now, true);
            if m.pgn == PGN_DM1 {
                self.on_dm1(m.sa, &m.data, now);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn put_pg(&mut self, pgn: u32, sa: u8, da: u8, priority: u8, data: &[u8], now: u64, multi: bool) {
        if !self.pgs.contains_key(&(pgn, sa)) && self.pgs.len() >= MAX_PG_ROWS {
            self.pg_dropped += 1;
            return;
        }
        let r = self.pgs.entry((pgn, sa)).or_insert(PgRow { pgn, sa, da, priority, count: 0, period_us: None, first_us: now, last_us: now, data: Vec::new(), multi });
        if r.count > 0 {
            r.period_us = Some(now.saturating_sub(r.last_us));
        }
        r.count += 1;
        r.da = da;
        r.priority = priority;
        r.last_us = now;
        r.data = data.to_vec();
        r.multi = multi;
    }

    fn on_dm1(&mut self, sa: u8, d: &[u8], now: u64) {
        let Some(m) = decode_dm(d) else { return };
        for r in self.faults.values_mut().filter(|r| r.sa == sa) {
            if !m.faults.iter().any(|f| f.spn == r.spn && f.fmi == r.fmi) {
                r.active = false;
            }
        }
        for f in &m.faults {
            let key = (sa, f.spn, f.fmi);
            if !self.faults.contains_key(&key) && self.faults.len() >= MAX_FAULT_ROWS {
                self.fault_dropped += 1;
                continue;
            }
            let r = self.faults.entry(key).or_insert(FaultRow { sa, spn: f.spn, fmi: f.fmi, oc: f.oc, lamps: m.lamps, first_us: now, last_us: now, active: true });
            r.oc = f.oc;
            r.lamps = m.lamps;
            r.last_us = now;
            r.active = true;
        }
    }

    /// The parameter group table as CSV (one header line).
    pub fn csv(&self) -> String {
        let mut s = String::from("pgn,name,sa,da,priority,length,count,cycle_ms,multi_frame,last_s,data\n");
        for r in self.pgs() {
            let cycle = r.period_us.map(|p| format!("{:.1}", p as f64 / 1000.0)).unwrap_or_default();
            s.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{:.6},{}\n",
                r.pgn,
                pgn_name(r.pgn).unwrap_or(""),
                r.sa,
                r.da,
                r.priority,
                r.data.len(),
                r.count,
                cycle,
                r.multi,
                r.last_us as f64 / 1e6,
                crate::canopen::types::hex_bytes(&r.data)
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::Dir;

    fn fr(id: u32, data: &[u8], t_ms: u64) -> CanFrame {
        let mut f = CanFrame::new(id, data);
        f.extended = true;
        f.dir = Dir::Rx;
        f.timestamp_us = t_ms * 1000;
        f
    }

    /// TP.CM from `sa` to `da`.
    fn cm(sa: u8, da: u8, d: [u8; 8], t: u64) -> CanFrame {
        fr(0x1CEC_0000 | (da as u32) << 8 | sa as u32, &d, t)
    }

    fn dt(sa: u8, da: u8, d: &[u8], t: u64) -> CanFrame {
        fr(0x1CEB_0000 | (da as u32) << 8 | sa as u32, d, t)
    }

    fn bam(pgn: u32, size: u16, packets: u8) -> [u8; 8] {
        let s = size.to_le_bytes();
        [32, s[0], s[1], packets, 0xFF, pgn as u8, (pgn >> 8) as u8, (pgn >> 16) as u8]
    }

    fn rts(pgn: u32, size: u16, packets: u8) -> [u8; 8] {
        let s = size.to_le_bytes();
        [16, s[0], s[1], packets, 0xFF, pgn as u8, (pgn >> 8) as u8, (pgn >> 16) as u8]
    }

    #[test]
    fn id_split_pdu1_pdu2_and_not_j1939() {
        // EEC1 (PDU2): priority 3, PGN 61444, SA 0, to all.
        let j = J1939Id::parse(0x0CF0_0400, true).unwrap();
        assert_eq!((j.priority, j.pgn(), j.sa, j.da(), j.is_pdu1()), (3, 61444, 0, 255, false));
        // A request (PDU1) from 0xF9 to 0x00: PGN without the PS.
        let j = J1939Id::parse(0x18EA_00F9, true).unwrap();
        assert_eq!((j.priority, j.pgn(), j.sa, j.da(), j.is_pdu1()), (6, 59904, 0xF9, 0, true));
        // PF 239 is still PDU1, 240 is PDU2.
        assert!(J1939Id::split(0x00EF_1234).is_pdu1());
        assert!(!J1939Id::split(0x00F0_1234).is_pdu1());
        // DP = 1 adds 65536; EDP = 1 with DP = 0 adds 131072.
        assert_eq!(J1939Id::parse(0x01FE_CA00, true).unwrap().pgn(), 65536 + 65226);
        assert_eq!(J1939Id::parse(0x02FE_CA00, true).unwrap().pgn(), 131072 + 65226);
        // EDP = 1 and DP = 1: not J1939. A standard frame: not J1939.
        assert!(J1939Id::parse(0x03FE_CA00, true).is_none());
        assert!(J1939Id::parse(0x181, false).is_none());
        // Priority 7 bits.
        assert_eq!(J1939Id::split(0x1CEC_FF00).priority, 7);
    }

    #[test]
    fn pgn_names_and_propb() {
        assert_eq!(pgn_name(61444), Some("EEC1"));
        assert_eq!(pgn_name(0), Some("TSC1"));
        assert_eq!(pgn_name(65263), Some("EFL/P1"));
        assert_eq!(pgn_name(61184), Some("PropA"));
        assert_eq!(pgn_name(65280), Some("PropB"));
        assert_eq!(pgn_name(65535), Some("PropB"));
        assert_eq!(pgn_name(65279), None);
        assert_eq!(pgn_name(12345), None);
        assert_eq!(pgn_text(65226), "PGN 65226 DM1");
        assert_eq!(pgn_text(1000), "PGN 1000");
        // The table is sorted (binary search).
        assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn pgn_match_of_signals() {
        // Any priority, any SA.
        assert!(pgn_matches(0x18FE_EE00, None, 0x18FE_EE21));
        assert!(pgn_matches(0x18FE_EE00, None, 0x0CFE_EE05));
        assert!(!pgn_matches(0x18FE_EE00, None, 0x18FE_EF00));
        // SA filter.
        assert!(pgn_matches(0x18FE_EE00, Some(0x21), 0x18FE_EE21));
        assert!(!pgn_matches(0x18FE_EE00, Some(0x21), 0x18FE_EE00));
        // PDU1: any DA.
        assert!(pgn_matches(0x18EA_00F9, None, 0x18EA_FF10));
        // Not J1939.
        assert!(!pgn_matches(0x03FE_CA00, None, 0x03FE_CA00));
    }

    #[test]
    fn bam_complete_with_padding_cut() {
        let mut r = TpReassembler::new();
        let data: Vec<u8> = (1..=20).collect();
        assert_eq!(r.on_frame(&cm(0, 255, bam(65226, 20, 3), 0)), TpStep::default());
        let s = r.on_frame(&dt(0, 255, &[1, 1, 2, 3, 4, 5, 6, 7], 50));
        assert_eq!((s.packet, s.done.is_none()), (Some((1, 3)), true));
        r.on_frame(&dt(0, 255, &[2, 8, 9, 10, 11, 12, 13, 14], 100));
        let s = r.on_frame(&dt(0, 255, &[3, 15, 16, 17, 18, 19, 20, 0xFF], 150));
        assert_eq!(s.packet, Some((3, 3)));
        let m = s.done.unwrap();
        assert_eq!((m.pgn, m.sa, m.da, m.kind, m.first_us, m.last_us), (65226, 0, 255, TpKind::Bam, 0, 150_000));
        assert_eq!(m.data, data);
        assert_eq!(r.counters, TpCounters { complete: 1, ..TpCounters::default() });
        assert_eq!(r.open_sessions(), 0);
        assert_eq!(r.counters.text(), "TP: 1 complete, 0 incomplete, 0 errors");
    }

    #[test]
    fn rts_cts_followed_in_both_directions() {
        let mut r = TpReassembler::new();
        // 0x10 sends 9 bytes of PGN 65259 to 0x20.
        r.on_frame(&cm(0x10, 0x20, rts(65259, 9, 2), 0));
        // CTS from the receiver: 2 packets from 1.
        r.on_frame(&cm(0x20, 0x10, [17, 2, 1, 0xFF, 0xFF, 0xEB, 0xFE, 0x00], 10));
        r.on_frame(&dt(0x10, 0x20, &[1, 1, 2, 3, 4, 5, 6, 7], 20));
        // The receiver asks for packet 2 again (CTS 1 from 2): no error.
        r.on_frame(&cm(0x20, 0x10, [17, 1, 2, 0xFF, 0xFF, 0xEB, 0xFE, 0x00], 25));
        let s = r.on_frame(&dt(0x10, 0x20, &[2, 8, 9, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], 30));
        let m = s.done.unwrap();
        assert_eq!((m.pgn, m.sa, m.da, m.kind, m.data.len()), (65259, 0x10, 0x20, TpKind::RtsCts, 9));
        // The EOM ACK of the receiver closes nothing more.
        r.on_frame(&cm(0x20, 0x10, [19, 9, 0, 2, 0xFF, 0xEB, 0xFE, 0x00], 40));
        assert_eq!(r.counters, TpCounters { complete: 1, ..TpCounters::default() });
        // A retransmission after a CTS from 1 again.
        r.on_frame(&cm(0x10, 0x20, rts(65259, 9, 2), 100));
        r.on_frame(&dt(0x10, 0x20, &[1, 1, 2, 3, 4, 5, 6, 7], 110));
        r.on_frame(&cm(0x20, 0x10, [17, 2, 1, 0xFF, 0xFF, 0xEB, 0xFE, 0x00], 115));
        r.on_frame(&dt(0x10, 0x20, &[1, 1, 2, 3, 4, 5, 6, 7], 120));
        assert!(r.on_frame(&dt(0x10, 0x20, &[2, 8, 9, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], 130)).done.is_some());
        assert_eq!(r.counters.complete, 2);
    }

    #[test]
    fn timeouts_bam_and_rts() {
        let mut r = TpReassembler::new();
        r.on_frame(&cm(0, 255, bam(65226, 20, 3), 0));
        r.on_frame(&dt(0, 255, &[1, 1, 2, 3, 4, 5, 6, 7], 700));
        // 751 ms after the last packet: dropped, the next packet is ignored.
        let s = r.on_frame(&dt(0, 255, &[2, 1, 2, 3, 4, 5, 6, 7], 1451));
        assert_eq!(s, TpStep::default());
        assert_eq!(r.counters.timeout, 1);
        // RTS/CTS: 1250 ms is still in time, 1251 not.
        r.on_frame(&cm(1, 2, rts(65259, 9, 2), 2000));
        r.on_frame(&dt(1, 2, &[1, 1, 2, 3, 4, 5, 6, 7], 3250));
        assert_eq!((r.counters.timeout, r.open_sessions()), (1, 1));
        r.expire(4_501_000);
        assert_eq!((r.counters.timeout, r.open_sessions()), (2, 0));
    }

    #[test]
    fn out_of_order_abort_and_replaced() {
        let mut r = TpReassembler::new();
        r.on_frame(&cm(0, 255, bam(65226, 20, 3), 0));
        r.on_frame(&dt(0, 255, &[1, 1, 2, 3, 4, 5, 6, 7], 50));
        // Packet 3 before 2: the session ends as an error.
        r.on_frame(&dt(0, 255, &[3, 1, 2, 3, 4, 5, 6, 7], 100));
        assert_eq!((r.counters.seq_error, r.open_sessions()), (1, 0));
        // Abort from the receiver of an RTS/CTS session (reason 3).
        r.on_frame(&cm(0x10, 0x20, rts(65259, 9, 2), 200));
        r.on_frame(&cm(0x20, 0x10, [255, 3, 0xFF, 0xFF, 0xFF, 0xEB, 0xFE, 0x00], 210));
        assert_eq!((r.counters.abort, r.open_sessions()), (1, 0));
        // Abort from the sender.
        r.on_frame(&cm(0x10, 0x20, rts(65259, 9, 2), 300));
        r.on_frame(&cm(0x10, 0x20, [255, 1, 0xFF, 0xFF, 0xFF, 0xEB, 0xFE, 0x00], 310));
        assert_eq!((r.counters.abort, r.open_sessions()), (2, 0));
        // A new BAM on the same pair replaces the old one (incomplete).
        r.on_frame(&cm(0, 255, bam(65226, 20, 3), 400));
        r.on_frame(&dt(0, 255, &[1, 1, 2, 3, 4, 5, 6, 7], 410));
        r.on_frame(&cm(0, 255, bam(65227, 10, 2), 420));
        assert_eq!(r.counters.incomplete, 1);
        r.on_frame(&dt(0, 255, &[1, 1, 2, 3, 4, 5, 6, 7], 430));
        let m = r.on_frame(&dt(0, 255, &[2, 8, 9, 10, 0xFF, 0xFF, 0xFF, 0xFF], 440)).done.unwrap();
        assert_eq!((m.pgn, m.data.len()), (65227, 10));
        assert_eq!(r.counters.errors(), 3);
    }

    #[test]
    fn interleaved_sources() {
        let mut r = TpReassembler::new();
        r.on_frame(&cm(0, 255, bam(65226, 10, 2), 0));
        r.on_frame(&cm(0x21, 255, bam(65227, 10, 2), 1));
        r.on_frame(&dt(0x21, 255, &[1, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7], 2));
        r.on_frame(&dt(0, 255, &[1, 1, 2, 3, 4, 5, 6, 7], 3));
        let a = r.on_frame(&dt(0, 255, &[2, 8, 9, 10, 0xFF, 0xFF, 0xFF, 0xFF], 4)).done.unwrap();
        let b = r.on_frame(&dt(0x21, 255, &[2, 0xA8, 0xA9, 0xAA, 0xFF, 0xFF, 0xFF, 0xFF], 5)).done.unwrap();
        assert_eq!((a.sa, a.pgn, a.data[9]), (0, 65226, 10));
        assert_eq!((b.sa, b.pgn, b.data[9]), (0x21, 65227, 0xAA));
        assert_eq!(r.counters.complete, 2);
    }

    #[test]
    fn name_decode() {
        // identity 5678, mfr 123, ECU inst 1, fn inst 2, fn 4, veh sys 3,
        // veh sys inst 5, industry group 2, arbitrary address capable.
        let raw: u64 = 5678 | 123 << 21 | 1 << 32 | 2 << 35 | 4 << 40 | 3 << 49 | 5 << 56 | 2 << 60 | 1 << 63;
        let n = Name::decode(&raw.to_le_bytes()).unwrap();
        assert_eq!((n.identity, n.manufacturer, n.ecu_instance, n.function_instance, n.function), (5678, 123, 1, 2, 4));
        assert_eq!((n.vehicle_system, n.vehicle_system_instance, n.industry_group, n.arbitrary_address), (3, 5, 2, true));
        assert_eq!(n.short_text(), "NAME mfr 123 fn 4 id 5678");
        assert!(Name::decode(&[1, 2, 3]).is_none());
        // All bits of the identity and the manufacturer.
        let m = Name::from_u64(0x1F_FFFF | 0x7FF << 21);
        assert_eq!((m.identity, m.manufacturer, m.function), (0x1F_FFFF, 0x7FF, 0));
    }

    fn fault(spn: u32, fmi: u8, oc: u8) -> [u8; 4] {
        [spn as u8, (spn >> 8) as u8, ((spn >> 16) as u8 & 7) << 5 | fmi, oc & 0x7F]
    }

    #[test]
    fn dm1_none_one_several() {
        // None: SPN 0 FMI 0.
        let m = decode_dm(&[0, 0xFF, 0, 0, 0, 0, 0xFF, 0xFF]).unwrap();
        assert!(m.faults.is_empty());
        assert_eq!(dm_text("DM1", &m), "DM1: no active fault");
        // One, amber lamp on; the padding after it is not a fault.
        let mut d = vec![0x04, 0xFF];
        d.extend(fault(100, 1, 3));
        d.extend([0xFF, 0xFF]);
        let m = decode_dm(&d).unwrap();
        assert_eq!(m.faults, vec![Dtc { spn: 100, fmi: 1, cm: false, oc: 3 }]);
        assert_eq!(dm_text("DM1", &m), "DM1: SPN 100 FMI 1 OC 3; lamps: amber");
        // A 19-bit SPN.
        let m = decode_dm(&[0x40, 0, 0xFF, 0xFF, 0xE5, 0x81]).unwrap();
        assert_eq!(m.faults, vec![Dtc { spn: 0x7FFFF, fmi: 5, cm: true, oc: 1 }]);
        assert_eq!(lamps_on(0x40), vec!["MIL"]);
        assert_eq!(lamps_on(0x55), vec!["MIL", "red stop", "amber", "protect"]);
        assert!(lamps_on(0xAA).is_empty());
        // Several through the transport protocol.
        let mut msg = vec![0x14, 0xFF];
        msg.extend(fault(100, 1, 3));
        msg.extend(fault(110, 0, 1));
        msg.extend(fault(523000, 31, 126));
        assert_eq!(msg.len(), 14);
        let mut r = TpReassembler::new();
        r.on_frame(&cm(0, 255, bam(PGN_DM1, 14, 2), 0));
        let mut p1 = vec![1];
        p1.extend(&msg[..7]);
        let mut p2 = vec![2];
        p2.extend(&msg[7..]);
        r.on_frame(&dt(0, 255, &p1, 50));
        let m = r.on_frame(&dt(0, 255, &p2, 100)).done.unwrap();
        let dm = decode_dm(&m.data).unwrap();
        assert_eq!(dm.faults.iter().map(|f| (f.spn, f.fmi, f.oc)).collect::<Vec<_>>(), vec![(100, 1, 3), (110, 0, 1), (523000, 31, 126)]);
        assert_eq!(dm_text("DM1", &dm), "DM1: SPN 100 FMI 1 OC 3, SPN 110 FMI 0 OC 1, SPN 523000 FMI 31 OC 126; lamps: red stop, amber");
    }

    #[test]
    fn frame_texts() {
        let t = |id: u32, d: &[u8]| frame_text(&J1939Id::parse(id, true).unwrap(), d, None);
        assert_eq!(t(0x0CF0_0400, &[0; 8]), "PGN 61444 EEC1  P3  -> all");
        assert_eq!(t(0x18EA_00F9, &[0xEB, 0xFE, 0x00]), "PGN 59904 RQST  P6  -> 0  req PGN 65259 CI");
        assert_eq!(t(0x18E8_F900, &[1, 0xFF, 0xFF, 0xFF, 0xFF, 0xEB, 0xFE, 0x00]), "PGN 59392 ACKM  P6  -> 249  NACK PGN 65259 CI");
        let name = (5678u64 | 123 << 21 | 4 << 40).to_le_bytes();
        assert_eq!(t(0x18EE_FF00, &name), "PGN 60928 AC  P6  -> all  NAME mfr 123 fn 4 id 5678");
        assert_eq!(t(0x18EE_FFFE, &name), "PGN 60928 AC  P6  -> all  cannot claim  NAME mfr 123 fn 4 id 5678");
        assert_eq!(t(0x1CEC_FF00, &bam(65226, 20, 3)), "PGN 60416 TP.CM  P7  -> all  BAM PGN 65226 DM1, 20 bytes, 3 packets");
        assert_eq!(t(0x1CEB_FF00, &[2, 0, 0, 0, 0, 0, 0, 0]), "PGN 60160 TP.DT  P7  -> all  TP.DT 2");
        assert_eq!(t(0x18FE_CA00, &[0, 0xFF, 0, 0, 0, 0, 0xFF, 0xFF]), "PGN 65226 DM1  P6  -> all  DM1: no active fault");
        assert_eq!(t(0x18FF_1000, &[1]), "PGN 65296 PropB  P6  -> all");
        assert_eq!(t(0x18C8_FF00, &[0; 8]), "PGN 51200 ETP.CM  P6  -> all  ETP, not reassembled");
        // With the transport state: n/N and the complete message.
        let mut r = TpReassembler::new();
        r.on_frame(&cm(0, 255, bam(65226, 10, 2), 0));
        let f1 = dt(0, 255, &[1, 0x04, 0xFF, 100, 0, 1, 3, 0x6E], 50);
        let s = r.on_frame(&f1);
        assert_eq!(frame_text(&J1939Id::parse(f1.id, true).unwrap(), f1.payload(), Some(&s)), "PGN 60160 TP.DT  P7  -> all  TP.DT 1/2");
        let f2 = dt(0, 255, &[2, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF], 100);
        let s = r.on_frame(&f2);
        assert_eq!(
            frame_text(&J1939Id::parse(f2.id, true).unwrap(), f2.payload(), Some(&s)),
            "PGN 60160 TP.DT  P7  -> all  TP.DT 2/2  complete: PGN 65226 DM1, 10 bytes  DM1: SPN 100 FMI 1 OC 3, SPN 110 FMI 0 OC 1; lamps: amber"
        );
    }

    #[test]
    fn monitor_tables_and_limits() {
        let mut m = J1939Monitor::new();
        m.on_frame(&fr(0x0CF0_0400, &[1; 8], 0));
        m.on_frame(&fr(0x0CF0_0400, &[2; 8], 50));
        m.on_frame(&fr(0x18FE_EE21, &[3; 8], 60));
        let name = (5678u64 | 123 << 21 | 4 << 40).to_le_bytes();
        m.on_frame(&fr(0x18EE_FF00, &name, 70));
        let r = m.pg(61444, 0).unwrap();
        assert_eq!((r.count, r.period_us, r.data[0], r.da, r.priority), (2, Some(50_000), 2, 255, 3));
        assert_eq!(m.addrs().map(|a| (a.sa, a.frames)).collect::<Vec<_>>(), vec![(0, 3), (0x21, 1)]);
        assert_eq!(m.addrs().next().unwrap().name.unwrap().identity, 5678);
        // DM1 through BAM: two faults, then a DM1 with one of them.
        let mut msg = vec![0x04, 0xFF];
        msg.extend(fault(100, 1, 3));
        msg.extend(fault(110, 0, 1));
        m.on_frame(&cm(0, 255, bam(PGN_DM1, 10, 2), 100));
        let mut p1 = vec![1];
        p1.extend(&msg[..7]);
        let mut p2 = vec![2];
        p2.extend(&msg[7..]);
        p2.resize(8, 0xFF);
        m.on_frame(&dt(0, 255, &p1, 150));
        m.on_frame(&dt(0, 255, &p2, 200));
        let dm = m.pg(PGN_DM1, 0).unwrap();
        assert!(dm.multi && dm.data.len() == 10);
        assert_eq!(m.faults().filter(|f| f.active).count(), 2);
        let mut one = vec![0x00, 0xFF];
        one.extend(fault(110, 0, 2));
        one.extend([0xFF, 0xFF]);
        m.on_frame(&fr(0x18FE_CA00, &one, 1200));
        let f: Vec<(u32, bool, u8)> = m.faults().map(|f| (f.spn, f.active, f.oc)).collect();
        assert_eq!(f, vec![(100, false, 3), (110, true, 2)]);
        assert_eq!(m.counters().complete, 1);
        let csv = m.csv();
        // Rows in PGN order: TP.DT, TP.CM, address claim, EEC1, ...
        let line = csv.lines().find(|l| l.starts_with("61444,")).unwrap();
        assert!(line.starts_with("61444,EEC1,0,255,3,8,2,50.0,false,"), "{line}");
        assert!(csv.lines().nth(1).unwrap().starts_with("60160,TP.DT,"), "{csv}");
        // Standard frames and EDP+DP frames are ignored.
        let n = m.pg_rows();
        let mut s = fr(0x181, &[1], 1300);
        s.extended = false;
        m.on_frame(&s);
        m.on_frame(&fr(0x03FE_CA00, &[1], 1300));
        assert_eq!(m.pg_rows(), n);
        // The PG limit: new pairs are dropped and counted.
        let mut m = J1939Monitor::new();
        for i in 0..(MAX_PG_ROWS as u32 + 10) {
            m.on_frame(&fr(0x18FF_0000 | (i / 256) << 8 | (i % 256), &[0], 0));
        }
        assert_eq!((m.pg_rows(), m.pg_dropped), (MAX_PG_ROWS, 10));
        m.clear();
        assert!(m.is_empty());
    }
}
