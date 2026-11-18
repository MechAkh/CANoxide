//! Virtual CAN bus with one simulated CANopen node.
//!
//! The node is built from an object dictionary (a loaded EDS or the
//! built-in one). It answers SDO expedited, segmented and block transfers
//! (with CRC), sends a heartbeat, reacts to NMT, sends its TPDOs when
//! operational, takes RPDOs, can send an EMCY and has an LSS slave. The PDO
//! communication and mapping objects are writable with the CiA 301 rules,
//! and the node sends what they say. `SimNode` is driven by frames and an
//! explicit time, so tests are deterministic; `SimAdapter` wraps it as a
//! `CanAdapter`.

use super::{Bitrate, CanAdapter, CanError, CanFrame, ChannelInfo, Dir};
use crate::canopen::lss::{LssAction, LssIdentity, LssSlave, NODE_ID_UNCONFIGURED};
use crate::canopen::nmt::{NmtCommand, NodeState};
use crate::canopen::pdo::{configs_with, extract_bits, insert_bits, MapEntry, PdoConfig, PdoKind};
use crate::canopen::sdo::crc16_ccitt;
use crate::canopen::types::{self, le_u64, DataType};
use crate::canopen::{COB_EMCY, COB_HEARTBEAT, COB_LSS_MASTER, COB_LSS_SLAVE, COB_NMT, COB_SDO_RX, COB_SDO_TX, COB_SYNC};
use crate::eds::{self, AccessType, ObjectDictionary};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const BUILTIN_EDS: &str = include_str!("sim_builtin.eds");

/// Largest string / domain the simulator stores.
pub const MAX_DOMAIN: usize = 65536;
/// Size of the large DOMAIN object 0x2100 of the built-in dictionary.
pub const LARGE_OBJECT_SIZE: usize = 4096;
pub const LARGE_OBJECT: u16 = 0x2100;
/// TPDO period when the event timer is 0 and the PDO is not SYNC driven.
pub const DEFAULT_TPDO_PERIOD_MS: u64 = 100;

pub fn builtin_dictionary() -> ObjectDictionary {
    eds::parse_str(BUILTIN_EDS)
}

/// The default content of 0x2100: a byte pattern that is easy to check.
pub fn large_object_pattern() -> Vec<u8> {
    (0..LARGE_OBJECT_SIZE).map(|i| (i % 251) as u8 ^ (i / 251) as u8).collect()
}

#[derive(Clone, Debug)]
enum Server {
    Idle,
    Upload { data: Vec<u8>, pos: usize, toggle: bool },
    Download { index: u16, sub: u8, buf: Vec<u8>, toggle: bool },
    /// Block download: receiving sub-blocks.
    BlkDown { index: u16, sub: u8, size: Option<usize>, crc: bool, buf: Vec<u8>, expected: u8, complete: bool },
    /// Block download: all data in; waiting for the end frame.
    BlkDownEnd { index: u16, sub: u8, size: Option<usize>, crc: bool, buf: Vec<u8> },
    /// Block upload initiated; waiting for "start".
    BlkUpReady { data: Vec<u8>, crc: bool, blksize: u8 },
    /// A sub-block is sent from segment `start`; waiting for the ack.
    BlkUpSent { data: Vec<u8>, crc: bool, start: usize, sent: u8 },
    /// End frame sent; waiting for the client's end response.
    BlkUpEnd,
}

pub struct SimNode {
    /// 1..127, or 255 while LSS-unconfigured.
    pub node_id: u8,
    od: Arc<ObjectDictionary>,
    values: BTreeMap<(u16, u8), Vec<u8>>,
    state: NodeState,
    booted: bool,
    server: Server,
    next_hb_ms: u64,
    pdo_due: HashMap<u16, u64>,
    sync_count: HashMap<u16, u32>,
    now_ms: u64,
    fill_large: bool,
    /// Block size the server asks for on a block download (1..127).
    pub block_size: u8,
    /// The server supports CRC on block transfers.
    pub block_crc: bool,
    /// The server supports block transfer at all (else it aborts).
    pub block_enabled: bool,
    pub lss: LssSlave,
    /// "Activate bit timing" requests: (table index, delay ms). Only recorded.
    pub lss_bit_timing_requests: Vec<(u8, u16)>,
    /// Number of "store configuration" requests.
    pub lss_store_count: u32,
    /// Successful SDO writes (to check that nothing was written).
    pub sdo_writes: u32,
    /// Successful "store parameters" writes (0x1010:01 = "save").
    pub store_count: u32,
}

impl SimNode {
    pub fn new(node_id: u8, od: Arc<ObjectDictionary>) -> SimNode {
        let mut n = SimNode {
            node_id: node_id.clamp(1, 127),
            od,
            values: BTreeMap::new(),
            state: NodeState::PreOperational,
            booted: false,
            server: Server::Idle,
            next_hb_ms: 0,
            pdo_due: HashMap::new(),
            sync_count: HashMap::new(),
            now_ms: 0,
            fill_large: false,
            block_size: 127,
            block_crc: true,
            block_enabled: true,
            lss: LssSlave::new(LssIdentity::default(), node_id.clamp(1, 127)),
            lss_bit_timing_requests: Vec::new(),
            lss_store_count: 0,
            sdo_writes: 0,
            store_count: 0,
        };
        n.load_defaults(0x0000, 0xFFFF);
        n.lss.identity = n.identity();
        n
    }

    pub fn builtin(node_id: u8) -> SimNode {
        let mut n = SimNode::new(node_id, Arc::new(builtin_dictionary()));
        n.fill_large = true;
        n.load_defaults(LARGE_OBJECT, LARGE_OBJECT);
        n
    }

    /// A node that starts LSS-unconfigured (node-ID 255): it sends no
    /// boot-up and no heartbeat and answers no SDO until LSS gives it a
    /// node-ID and it is switched to waiting.
    pub fn unconfigured(mut self) -> SimNode {
        self.node_id = NODE_ID_UNCONFIGURED;
        self.lss.active_node_id = NODE_ID_UNCONFIGURED;
        self.lss.pending_node_id = NODE_ID_UNCONFIGURED;
        self.booted = false;
        self
    }

    /// The identity of 0x1018:01..04.
    pub fn identity(&self) -> LssIdentity {
        let g = |s| self.values.get(&(0x1018, s)).map(|v| le_u64(v) as u32).unwrap_or(0);
        LssIdentity { vendor: g(1), product: g(2), revision: g(3), serial: g(4) }
    }

    pub fn is_configured(&self) -> bool {
        self.node_id != NODE_ID_UNCONFIGURED
    }

    pub fn dictionary(&self) -> &ObjectDictionary {
        &self.od
    }

    pub fn state(&self) -> NodeState {
        self.state
    }

    pub fn value(&self, index: u16, sub: u8) -> Option<&[u8]> {
        self.values.get(&(index, sub)).map(|v| v.as_slice())
    }

    pub fn set_value(&mut self, index: u16, sub: u8, data: Vec<u8>) {
        self.values.insert((index, sub), data);
    }

    fn load_defaults(&mut self, from: u16, to: u16) {
        let node = if self.is_configured() { self.node_id } else { 1 };
        for (ix, o) in self.od.objects.range(from..=to) {
            for (sub, e) in &o.subs {
                let v = e.effective_bytes(node).unwrap_or_else(|| e.dtype().and_then(|t| t.size()).map(|n| vec![0; n]).unwrap_or_default());
                self.values.insert((*ix, *sub), v);
            }
        }
        if self.fill_large && (from..=to).contains(&LARGE_OBJECT) && self.od.objects.contains_key(&LARGE_OBJECT) {
            self.values.insert((LARGE_OBJECT, 0), large_object_pattern());
        }
        // The simulator always sends a heartbeat unless the user writes 0.
        if let Some(v) = self.values.get_mut(&(0x1017, 0)) {
            if le_u64(v) == 0 && v.len() >= 2 {
                v[..2].copy_from_slice(&1000u16.to_le_bytes());
            }
        }
    }

    fn heartbeat_ms(&self) -> u64 {
        match self.values.get(&(0x1017, 0)) {
            Some(v) => le_u64(v) & 0xFFFF,
            None => 1000,
        }
    }

    fn frame(&self, id: u32, data: &[u8]) -> CanFrame {
        let mut f = CanFrame::new(id, data);
        f.dir = Dir::Rx;
        f.timestamp_us = self.now_ms * 1000;
        f
    }

    fn boot(&mut self, out: &mut Vec<CanFrame>) {
        self.state = NodeState::PreOperational;
        self.server = Server::Idle;
        self.pdo_due.clear();
        self.sync_count.clear();
        if !self.is_configured() {
            self.booted = true;
            return;
        }
        out.push(self.frame(COB_HEARTBEAT + self.node_id as u32, &[0x00]));
        self.booted = true;
        self.next_hb_ms = self.now_ms + self.heartbeat_ms().max(1);
    }

    /// Advance the time; sends boot-up, heartbeat and timed TPDOs.
    pub fn tick(&mut self, now_ms: u64, out: &mut Vec<CanFrame>) {
        self.now_ms = now_ms;
        if !self.booted {
            self.boot(out);
            return;
        }
        if !self.is_configured() {
            return;
        }
        let hb = self.heartbeat_ms();
        if hb > 0 && now_ms >= self.next_hb_ms {
            out.push(self.frame(COB_HEARTBEAT + self.node_id as u32, &[self.state.byte()]));
            self.next_hb_ms = now_ms + hb;
        } else if hb == 0 {
            self.next_hb_ms = now_ms;
        }
        if self.state != NodeState::Operational {
            return;
        }
        for cfg in self.pdo_configs() {
            if cfg.kind != PdoKind::Tpdo {
                continue;
            }
            if !cfg.valid || cfg.extended || cfg.mapping.is_empty() {
                // A disabled TPDO starts a new period when it is enabled again.
                self.pdo_due.remove(&cfg.number);
                continue;
            }
            if (1..=240).contains(&cfg.trans_type) {
                continue;
            }
            let period = if cfg.event_timer_ms > 0 { cfg.event_timer_ms as u64 } else { DEFAULT_TPDO_PERIOD_MS };
            let due = self.pdo_due.entry(cfg.number).or_insert(now_ms + period);
            // A shorter event timer takes effect at once.
            if *due > now_ms + period {
                *due = now_ms + period;
            }
            if now_ms >= *due {
                self.pdo_due.insert(cfg.number, now_ms + period);
                if let Some(f) = self.build_tpdo(&cfg) {
                    out.push(f);
                }
            }
        }
    }

    pub fn pdo_configs(&self) -> Vec<PdoConfig> {
        configs_with(|i, s| self.values.get(&(i, s)).map(|b| le_u64(b) as u32), |i| self.od.objects.contains_key(&i))
    }

    fn build_tpdo(&mut self, cfg: &PdoConfig) -> Option<CanFrame> {
        let bits = cfg.total_bits();
        if bits > 64 {
            return None;
        }
        // Inputs change: count up the mapped read-only application objects.
        for m in &cfg.mapping {
            if !(0x2000..=0x9FFF).contains(&m.index) {
                continue;
            }
            let Some(e) = self.od.entry(m.index, m.sub) else { continue };
            if e.access != AccessType::Ro || !e.dtype().is_some_and(|t| t.is_unsigned() && t != DataType::Boolean) {
                continue;
            }
            if let Some(v) = self.values.get_mut(&(m.index, m.sub)) {
                let n = v.len().min(8);
                let x = le_u64(&v[..n]).wrapping_add(1);
                v[..n].copy_from_slice(&x.to_le_bytes()[..n]);
            }
        }
        let mut data = [0u8; 8];
        let mut off = 0u32;
        for m in &cfg.mapping {
            let b = m.bits as u32;
            if !m.is_dummy() {
                let v = self.values.get(&(m.index, m.sub)).map(|v| le_u64(v)).unwrap_or(0);
                insert_bits(&mut data, off, b, v);
            }
            off += b;
        }
        let len = bits.div_ceil(8) as usize;
        Some(self.frame(cfg.cob_id, &data[..len]))
    }

    /// Send an EMCY with an error code and error register.
    pub fn emcy(&mut self, code: u16, reg: u8, out: &mut Vec<CanFrame>) {
        if self.values.contains_key(&(0x1001, 0)) {
            self.values.insert((0x1001, 0), vec![reg]);
        }
        let c = code.to_le_bytes();
        out.push(self.frame(COB_EMCY + self.node_id as u32, &[c[0], c[1], reg, 0, 0, 0, 0, 0]));
    }

    /// Handle a frame from the bus; answers go to `out`.
    pub fn handle_frame(&mut self, f: &CanFrame, out: &mut Vec<CanFrame>) {
        if f.extended {
            return;
        }
        if f.id == COB_LSS_MASTER && !f.rtr {
            self.on_lss(&f.data, out);
            return;
        }
        if !self.is_configured() {
            return;
        }
        let me = self.node_id as u32;
        if f.id == COB_NMT && f.dlc >= 2 && !f.rtr {
            if f.data[1] == 0 || f.data[1] as u32 == me {
                self.nmt(f.data[0], out);
            }
            return;
        }
        if f.id == COB_SYNC {
            self.on_sync(out);
            return;
        }
        if f.id == COB_SDO_RX + me && !f.rtr {
            if self.state == NodeState::Stopped {
                return;
            }
            for r in self.sdo(&f.data) {
                out.push(self.frame(COB_SDO_TX + me, &r));
            }
            return;
        }
        if self.state == NodeState::Operational && !f.rtr {
            for cfg in self.pdo_configs() {
                if cfg.kind == PdoKind::Rpdo && cfg.valid && !cfg.extended && cfg.cob_id == f.id {
                    self.apply_rpdo(&cfg, f.payload());
                }
            }
        }
    }

    fn on_lss(&mut self, d: &[u8; 8], out: &mut Vec<CanFrame>) {
        let (reply, action) = self.lss.handle(d);
        if let Some(r) = reply {
            out.push(self.frame(COB_LSS_SLAVE, &r));
        }
        match action {
            LssAction::None => {}
            LssAction::ApplyNodeId(id) => {
                // The new node-ID: reset communication and boot with it.
                self.node_id = id;
                if self.is_configured() {
                    self.load_defaults(0x1000, 0x1FFF);
                }
                self.boot(out);
            }
            LssAction::ActivateBitTiming { index, delay_ms } => self.lss_bit_timing_requests.push((index, delay_ms)),
            LssAction::Store => self.lss_store_count += 1,
        }
    }

    fn nmt(&mut self, cs: u8, out: &mut Vec<CanFrame>) {
        match NmtCommand::from_code(cs) {
            Some(NmtCommand::Start) => self.state = NodeState::Operational,
            Some(NmtCommand::Stop) => self.state = NodeState::Stopped,
            Some(NmtCommand::PreOperational) => self.state = NodeState::PreOperational,
            Some(NmtCommand::ResetNode) => {
                self.load_defaults(0x0000, 0xFFFF);
                self.boot(out);
            }
            Some(NmtCommand::ResetCommunication) => {
                self.load_defaults(0x1000, 0x1FFF);
                self.boot(out);
            }
            None => {}
        }
    }

    fn on_sync(&mut self, out: &mut Vec<CanFrame>) {
        if self.state != NodeState::Operational {
            return;
        }
        for cfg in self.pdo_configs() {
            if cfg.kind != PdoKind::Tpdo || !cfg.valid || cfg.extended || !(1..=240).contains(&cfg.trans_type) {
                continue;
            }
            let c = self.sync_count.entry(cfg.number).or_insert(0);
            *c += 1;
            if *c >= cfg.trans_type as u32 {
                *c = 0;
                if let Some(f) = self.build_tpdo(&cfg) {
                    out.push(f);
                }
            }
        }
    }

    fn apply_rpdo(&mut self, cfg: &PdoConfig, data: &[u8]) {
        let mut off = 0u32;
        for m in &cfg.mapping {
            let b = m.bits as u32;
            if !m.is_dummy() {
                if let Some(raw) = extract_bits(data, off, b) {
                    if let Some(v) = self.values.get_mut(&(m.index, m.sub)) {
                        let n = (b as usize).div_ceil(8).min(8);
                        if v.len() < n {
                            v.resize(n, 0);
                        }
                        v[..n].copy_from_slice(&raw.to_le_bytes()[..n]);
                    }
                }
            }
            off += b;
        }
    }

    fn limit(&self, s: &Option<String>) -> Option<String> {
        let s = s.as_ref()?;
        if s.contains('$') {
            eds::eval_int(s, self.node_id).map(|v| v.to_string())
        } else {
            Some(s.clone())
        }
    }

    /// SDO read access; Err = abort code.
    pub fn read(&self, index: u16, sub: u8) -> Result<Vec<u8>, u32> {
        let o = self.od.objects.get(&index).ok_or(0x0602_0000u32)?;
        let e = o.subs.get(&sub).ok_or(0x0609_0011u32)?;
        if !e.access.readable() {
            return Err(0x0601_0001);
        }
        Ok(self.values.get(&(index, sub)).cloned().unwrap_or_default())
    }

    fn val32(&self, index: u16, sub: u8) -> u32 {
        self.values.get(&(index, sub)).map(|v| le_u64(v) as u32).unwrap_or(0)
    }

    /// The CiA 301 rules for the PDO communication and mapping objects.
    fn check_pdo_write(&self, index: u16, sub: u8, data: &[u8]) -> Result<(), u32> {
        let comm = (0x1400..=0x15FF).contains(&index) || (0x1800..=0x19FF).contains(&index);
        let map = (0x1600..=0x17FF).contains(&index) || (0x1A00..=0x1BFF).contains(&index);
        if comm && sub == 1 {
            let old = self.val32(index, 1);
            let new = le_u64(data) as u32;
            // The COB-ID may change only while the PDO is invalid.
            if old & 0x8000_0000 == 0 && new & 0x8000_0000 == 0 && (old & 0x1FFF_FFFF) != (new & 0x1FFF_FFFF) {
                return Err(0x0609_0030);
            }
        }
        if map {
            let comm_index = index - 0x200;
            if self.val32(comm_index, 1) & 0x8000_0000 == 0 {
                // Mapping changes need an invalid PDO.
                return Err(0x0800_0022);
            }
            if sub >= 1 && self.val32(index, 0) != 0 {
                return Err(0x0609_0030);
            }
            if sub == 0 {
                let n = data.first().copied().unwrap_or(0);
                let Some(obj) = self.od.objects.get(&index) else { return Ok(()) };
                let mut bits = 0u32;
                for s in 1..=n {
                    if !obj.subs.contains_key(&s) {
                        return Err(0x0604_0042);
                    }
                    let m = MapEntry::from_u32(self.val32(index, s));
                    if !m.is_dummy() {
                        let mappable = self.od.entry(m.index, m.sub).is_some_and(|e| e.pdo_mapping);
                        if !mappable {
                            return Err(0x0604_0041);
                        }
                    }
                    bits += m.bits as u32;
                }
                if bits > 64 {
                    return Err(0x0604_0042);
                }
            }
        }
        Ok(())
    }

    /// SDO write access; Err = abort code.
    pub fn write(&mut self, index: u16, sub: u8, data: Vec<u8>) -> Result<(), u32> {
        let o = self.od.objects.get(&index).ok_or(0x0602_0000u32)?;
        let e = o.subs.get(&sub).ok_or(0x0609_0011u32)?;
        if !e.access.writable() {
            return Err(0x0601_0002);
        }
        let mut data = data;
        match e.dtype().and_then(|t| t.size()) {
            Some(n) => {
                // Expedited transfers without size may carry 4 bytes for a smaller type.
                if data.len() > n && data[n..].iter().all(|&b| b == 0) && data.len() == 4 {
                    data.truncate(n);
                }
                if data.len() > n {
                    return Err(0x0607_0012);
                }
                if data.len() < n {
                    return Err(0x0607_0013);
                }
            }
            None => {
                if data.len() > MAX_DOMAIN {
                    return Err(0x0607_0012);
                }
            }
        }
        if let Some(dt) = e.dtype() {
            if let Err(err) = types::check_limits(dt, &data, self.limit(&e.low).as_deref(), self.limit(&e.high).as_deref()) {
                return Err(if err.0.contains("below") { 0x0609_0032 } else { 0x0609_0031 });
            }
        }
        self.check_pdo_write(index, sub, &data)?;
        if index == 0x1010 {
            // Store parameters: only the "save" signature; the value read back stays.
            if le_u64(&data) != 0x6576_6173 {
                return Err(0x0800_0020);
            }
            self.sdo_writes += 1;
            self.store_count += 1;
            return Ok(());
        }
        self.sdo_writes += 1;
        self.values.insert((index, sub), data);
        if index == 0x1017 {
            // A new heartbeat time takes effect at once.
            self.next_hb_ms = self.now_ms + self.heartbeat_ms().max(1);
        }
        Ok(())
    }

    /// Check that an object can be written before the data comes.
    fn check_writable(&self, index: u16, sub: u8) -> Option<u32> {
        let o = self.od.objects.get(&index);
        match o.and_then(|o| o.subs.get(&sub)) {
            None if o.is_none() => Some(0x0602_0000),
            None => Some(0x0609_0011),
            Some(e) if !e.access.writable() => Some(0x0601_0002),
            _ => None,
        }
    }

    fn sdo(&mut self, d: &[u8; 8]) -> Vec<[u8; 8]> {
        let index = u16::from_le_bytes([d[1], d[2]]);
        let sub = d[3];
        let abort = |index: u16, sub: u8, code: u32| {
            let i = index.to_le_bytes();
            let c = code.to_le_bytes();
            [0x80, i[0], i[1], sub, c[0], c[1], c[2], c[3]]
        };
        let ok_head = |cmd: u8| [cmd, d[1], d[2], d[3], 0, 0, 0, 0];
        // Block download sub-block: byte 0 is "c + sequence number".
        if let Server::BlkDown { .. } = self.server {
            if d[0] == 0x80 {
                self.server = Server::Idle;
                return Vec::new();
            }
            let Server::BlkDown { index, sub, size, crc, mut buf, expected, complete } = std::mem::replace(&mut self.server, Server::Idle) else {
                unreachable!()
            };
            let c = d[0] & 0x80 != 0;
            let seq = d[0] & 0x7F;
            let blksize = self.block_size.clamp(1, 127);
            if seq == 0 || seq > blksize {
                return vec![abort(index, sub, 0x0504_0003)];
            }
            let mut last_good = expected - 1;
            let mut complete = complete;
            if seq == expected {
                buf.extend_from_slice(&d[1..8]);
                last_good = seq;
                complete |= c;
            }
            if buf.len() > MAX_DOMAIN + 7 {
                return vec![abort(index, sub, 0x0607_0012)];
            }
            if seq == blksize || c {
                let ack = [0xA2, last_good, blksize, 0, 0, 0, 0, 0];
                self.server = if complete {
                    Server::BlkDownEnd { index, sub, size, crc, buf }
                } else {
                    Server::BlkDown { index, sub, size, crc, buf, expected: 1, complete: false }
                };
                return vec![ack];
            }
            self.server = Server::BlkDown { index, sub, size, crc, buf, expected: last_good + 1, complete };
            return Vec::new();
        }
        match d[0] >> 5 {
            1 => {
                self.server = Server::Idle;
                let e = d[0] & 0x02 != 0;
                let s = d[0] & 0x01 != 0;
                if e {
                    let len = if s { 4 - ((d[0] >> 2) & 3) as usize } else { 4 };
                    match self.write(index, sub, d[4..4 + len].to_vec()) {
                        Ok(()) => vec![ok_head(0x60)],
                        Err(c) => vec![abort(index, sub, c)],
                    }
                } else {
                    // Check access before the segments come.
                    if let Some(c) = self.check_writable(index, sub) {
                        return vec![abort(index, sub, c)];
                    }
                    self.server = Server::Download { index, sub, buf: Vec::new(), toggle: false };
                    vec![ok_head(0x60)]
                }
            }
            0 => {
                let Server::Download { index, sub, mut buf, toggle } = std::mem::replace(&mut self.server, Server::Idle) else {
                    return vec![abort(0, 0, 0x0504_0001)];
                };
                let t = d[0] & 0x10 != 0;
                if t != toggle {
                    return vec![abort(index, sub, 0x0503_0000)];
                }
                let n = ((d[0] >> 1) & 7) as usize;
                buf.extend_from_slice(&d[1..8 - n]);
                if buf.len() > MAX_DOMAIN {
                    return vec![abort(index, sub, 0x0607_0012)];
                }
                let resp = [0x20 | ((t as u8) << 4), 0, 0, 0, 0, 0, 0, 0];
                if d[0] & 0x01 != 0 {
                    return match self.write(index, sub, buf) {
                        Ok(()) => vec![resp],
                        Err(c) => vec![abort(index, sub, c)],
                    };
                }
                self.server = Server::Download { index, sub, buf, toggle: !toggle };
                vec![resp]
            }
            2 => {
                self.server = Server::Idle;
                match self.read(index, sub) {
                    Err(c) => vec![abort(index, sub, c)],
                    Ok(data) => self.upload_response(d, data),
                }
            }
            3 => {
                let Server::Upload { data, pos, toggle } = std::mem::replace(&mut self.server, Server::Idle) else {
                    return vec![abort(0, 0, 0x0504_0001)];
                };
                let t = d[0] & 0x10 != 0;
                if t != toggle {
                    return vec![abort(0, 0, 0x0503_0000)];
                }
                let end = (pos + 7).min(data.len());
                let chunk = &data[pos..end];
                let n = 7 - chunk.len() as u8;
                let last = end >= data.len();
                let mut r = [0u8; 8];
                r[0] = ((t as u8) << 4) | (n << 1) | last as u8;
                r[1..1 + chunk.len()].copy_from_slice(chunk);
                if !last {
                    self.server = Server::Upload { data, pos: end, toggle: !toggle };
                }
                vec![r]
            }
            4 => {
                self.server = Server::Idle;
                Vec::new()
            }
            5 => self.block_upload(d),
            6 => self.block_download(d),
            _ => vec![abort(index, sub, 0x0504_0001)],
        }
    }

    /// Expedited or segmented answer to an upload initiate.
    fn upload_response(&mut self, d: &[u8; 8], data: Vec<u8>) -> Vec<[u8; 8]> {
        let ok_head = |cmd: u8| [cmd, d[1], d[2], d[3], 0, 0, 0, 0];
        if (1..=4).contains(&data.len()) {
            let n = 4 - data.len() as u8;
            let mut r = ok_head(0x43 | (n << 2));
            r[4..4 + data.len()].copy_from_slice(&data);
            return vec![r];
        }
        let mut r = ok_head(0x41);
        r[4..8].copy_from_slice(&(data.len() as u32).to_le_bytes());
        self.server = Server::Upload { data, pos: 0, toggle: false };
        vec![r]
    }

    /// Client command specifier 6: block download initiate / end.
    fn block_download(&mut self, d: &[u8; 8]) -> Vec<[u8; 8]> {
        let index = u16::from_le_bytes([d[1], d[2]]);
        let sub = d[3];
        let abort = |index: u16, sub: u8, code: u32| {
            let i = index.to_le_bytes();
            let c = code.to_le_bytes();
            [0x80, i[0], i[1], sub, c[0], c[1], c[2], c[3]]
        };
        if d[0] & 0x01 == 0 {
            // Initiate.
            self.server = Server::Idle;
            if !self.block_enabled {
                return vec![abort(index, sub, 0x0504_0001)];
            }
            if let Some(c) = self.check_writable(index, sub) {
                return vec![abort(index, sub, c)];
            }
            let size = if d[0] & 0x02 != 0 { Some(u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize) } else { None };
            if size.is_some_and(|s| s > MAX_DOMAIN) {
                return vec![abort(index, sub, 0x0607_0012)];
            }
            let crc = d[0] & 0x04 != 0 && self.block_crc;
            let blksize = self.block_size.clamp(1, 127);
            self.server = Server::BlkDown { index, sub, size, crc, buf: Vec::new(), expected: 1, complete: false };
            return vec![[0xA0 | ((crc as u8) << 2), d[1], d[2], d[3], blksize, 0, 0, 0]];
        }
        // End.
        let Server::BlkDownEnd { index, sub, size, crc, mut buf } = std::mem::replace(&mut self.server, Server::Idle) else {
            return vec![abort(0, 0, 0x0504_0001)];
        };
        let n = ((d[0] >> 2) & 7) as usize;
        if n > buf.len() {
            return vec![abort(index, sub, 0x0607_0010)];
        }
        buf.truncate(buf.len() - n);
        if size.is_some_and(|s| s != buf.len()) {
            return vec![abort(index, sub, 0x0607_0010)];
        }
        if crc && crc16_ccitt(&buf) != u16::from_le_bytes([d[1], d[2]]) {
            return vec![abort(index, sub, 0x0504_0004)];
        }
        match self.write(index, sub, buf) {
            Ok(()) => vec![[0xA1, 0, 0, 0, 0, 0, 0, 0]],
            Err(c) => vec![abort(index, sub, c)],
        }
    }

    /// Client command specifier 5: block upload initiate / start / ack / end.
    fn block_upload(&mut self, d: &[u8; 8]) -> Vec<[u8; 8]> {
        let index = u16::from_le_bytes([d[1], d[2]]);
        let sub = d[3];
        let abort = |index: u16, sub: u8, code: u32| {
            let i = index.to_le_bytes();
            let c = code.to_le_bytes();
            [0x80, i[0], i[1], sub, c[0], c[1], c[2], c[3]]
        };
        match d[0] & 0x03 {
            0 => {
                self.server = Server::Idle;
                if !self.block_enabled {
                    return vec![abort(index, sub, 0x0504_0001)];
                }
                let blksize = d[4];
                if blksize == 0 || blksize > 127 {
                    return vec![abort(index, sub, 0x0504_0002)];
                }
                let pst = d[5] as usize;
                let data = match self.read(index, sub) {
                    Ok(v) => v,
                    Err(c) => return vec![abort(index, sub, c)],
                };
                if pst > 0 && data.len() <= pst {
                    // Protocol switch to an expedited / segmented upload.
                    return self.upload_response(d, data);
                }
                let crc = d[0] & 0x04 != 0 && self.block_crc;
                let s = (data.len() as u32).to_le_bytes();
                self.server = Server::BlkUpReady { data, crc, blksize };
                vec![[0xC2 | ((crc as u8) << 2), d[1], d[2], d[3], s[0], s[1], s[2], s[3]]]
            }
            3 => {
                let Server::BlkUpReady { data, crc, blksize } = std::mem::replace(&mut self.server, Server::Idle) else {
                    return vec![abort(0, 0, 0x0504_0001)];
                };
                self.send_up_block(data, crc, 0, blksize)
            }
            2 => {
                let Server::BlkUpSent { data, crc, start, sent } = std::mem::replace(&mut self.server, Server::Idle) else {
                    return vec![abort(0, 0, 0x0504_0001)];
                };
                let ack = d[1];
                let blksize = d[2];
                if ack > sent {
                    return vec![abort(0, 0, 0x0504_0003)];
                }
                let next = start + ack as usize;
                let total = data.len().div_ceil(7).max(1);
                if next >= total {
                    let n = (total * 7 - data.len()) as u8;
                    let c = if crc { crc16_ccitt(&data) } else { 0 }.to_le_bytes();
                    self.server = Server::BlkUpEnd;
                    return vec![[0xC1 | (n << 2), c[0], c[1], 0, 0, 0, 0, 0]];
                }
                if blksize == 0 || blksize > 127 {
                    return vec![abort(0, 0, 0x0504_0002)];
                }
                self.send_up_block(data, crc, next, blksize)
            }
            _ => {
                // End response of the client.
                self.server = Server::Idle;
                Vec::new()
            }
        }
    }

    fn send_up_block(&mut self, data: Vec<u8>, crc: bool, start: usize, blksize: u8) -> Vec<[u8; 8]> {
        let total = data.len().div_ceil(7).max(1);
        let count = (total - start).min(blksize as usize);
        let mut v = Vec::with_capacity(count);
        for k in 0..count {
            let i = start + k;
            let from = (i * 7).min(data.len());
            let to = (from + 7).min(data.len());
            let mut b = [0u8; 8];
            b[0] = (k as u8 + 1) | if i == total - 1 { 0x80 } else { 0 };
            b[1..1 + (to - from)].copy_from_slice(&data[from..to]);
            v.push(b);
        }
        self.server = Server::BlkUpSent { data, crc, start, sent: count as u8 };
        v
    }
}

/// The simulator as an adapter: a virtual bus with one node.
pub struct SimAdapter {
    node: SimNode,
    queue: VecDeque<CanFrame>,
    start: Instant,
    open: bool,
    listen_only: bool,
}

impl SimAdapter {
    pub fn new(node: SimNode) -> SimAdapter {
        SimAdapter { node, queue: VecDeque::new(), start: Instant::now(), open: false, listen_only: false }
    }

    pub fn node(&self) -> &SimNode {
        &self.node
    }

    pub fn node_mut(&mut self) -> &mut SimNode {
        &mut self.node
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Let the simulated node send an EMCY.
    pub fn send_emcy(&mut self, code: u16, reg: u8) {
        let mut out = Vec::new();
        self.node.emcy(code, reg, &mut out);
        self.queue.extend(out);
    }

    fn pump(&mut self) {
        let mut out = Vec::new();
        let now = self.now_ms();
        self.node.tick(now, &mut out);
        self.queue.extend(out);
    }
}

impl CanAdapter for SimAdapter {
    fn name(&self) -> String {
        "Simulator".to_string()
    }

    fn list_channels(&mut self) -> Result<Vec<ChannelInfo>, CanError> {
        Ok(vec![ChannelInfo { handle: 0, name: "Virtual bus".to_string(), available: true }])
    }

    fn open(&mut self, _channel: u32, _bitrate: Bitrate, listen_only: bool) -> Result<(), CanError> {
        self.open = true;
        self.listen_only = listen_only;
        self.start = Instant::now();
        // The node boots with the bus, before the first frame it handles.
        self.pump();
        Ok(())
    }

    fn close(&mut self) {
        self.open = false;
        self.queue.clear();
    }

    fn send(&mut self, frame: &CanFrame) -> Result<(), CanError> {
        if !self.open {
            return Err(CanError::new("channel not open"));
        }
        if self.listen_only {
            return Err(CanError::new("listen-only mode: sending is disabled"));
        }
        frame.validate()?;
        let mut out = Vec::new();
        self.node.now_ms = self.now_ms();
        self.node.handle_frame(frame, &mut out);
        self.queue.extend(out);
        Ok(())
    }

    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, CanError> {
        if !self.open {
            return Err(CanError::new("channel not open"));
        }
        self.pump();
        if let Some(f) = self.queue.pop_front() {
            return Ok(Some(f));
        }
        if timeout > Duration::ZERO {
            std::thread::sleep(timeout.min(Duration::from_millis(1)));
            self.pump();
        }
        Ok(self.queue.pop_front())
    }

    fn status(&mut self) -> String {
        if self.open {
            "OK (simulated)".to_string()
        } else {
            "closed".to_string()
        }
    }

    fn as_sim(&mut self) -> Option<&mut SimAdapter> {
        Some(self)
    }
}
