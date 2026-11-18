//! Virtual CAN bus with one simulated CANopen node.
//!
//! The node is built from an object dictionary (a loaded EDS or the
//! built-in one). It answers SDO expedited and segmented transfers, sends a
//! heartbeat, reacts to NMT, sends its TPDOs when operational, takes RPDOs
//! and can send an EMCY. `SimNode` is driven by frames and an explicit time,
//! so tests are deterministic; `SimAdapter` wraps it as a `CanAdapter`.

use super::{Bitrate, CanAdapter, CanError, CanFrame, ChannelInfo, Dir};
use crate::canopen::nmt::{NmtCommand, NodeState};
use crate::canopen::pdo::{configs_with, extract_bits, insert_bits, PdoConfig, PdoKind};
use crate::canopen::types::{self, le_u64, DataType};
use crate::canopen::{COB_EMCY, COB_HEARTBEAT, COB_NMT, COB_SDO_RX, COB_SDO_TX, COB_SYNC};
use crate::eds::{self, AccessType, ObjectDictionary};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const BUILTIN_EDS: &str = include_str!("sim_builtin.eds");

/// Largest string / domain the simulator stores.
pub const MAX_DOMAIN: usize = 4096;
/// TPDO period when the event timer is 0 and the PDO is not SYNC driven.
pub const DEFAULT_TPDO_PERIOD_MS: u64 = 100;

pub fn builtin_dictionary() -> ObjectDictionary {
    eds::parse_str(BUILTIN_EDS)
}

#[derive(Clone, Debug)]
enum Server {
    Idle,
    Upload { data: Vec<u8>, pos: usize, toggle: bool },
    Download { index: u16, sub: u8, buf: Vec<u8>, toggle: bool },
}

pub struct SimNode {
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
        };
        n.load_defaults(0x0000, 0xFFFF);
        n
    }

    pub fn builtin(node_id: u8) -> SimNode {
        SimNode::new(node_id, Arc::new(builtin_dictionary()))
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
        let node = self.node_id;
        for (ix, o) in self.od.objects.range(from..=to) {
            for (sub, e) in &o.subs {
                let v = e.effective_bytes(node).unwrap_or_else(|| e.dtype().and_then(|t| t.size()).map(|n| vec![0; n]).unwrap_or_default());
                self.values.insert((*ix, *sub), v);
            }
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
        out.push(self.frame(COB_HEARTBEAT + self.node_id as u32, &[0x00]));
        self.state = NodeState::PreOperational;
        self.booted = true;
        self.server = Server::Idle;
        self.next_hb_ms = self.now_ms + self.heartbeat_ms().max(1);
        self.pdo_due.clear();
        self.sync_count.clear();
    }

    /// Advance the time; sends boot-up, heartbeat and timed TPDOs.
    pub fn tick(&mut self, now_ms: u64, out: &mut Vec<CanFrame>) {
        self.now_ms = now_ms;
        if !self.booted {
            self.boot(out);
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
            if cfg.kind != PdoKind::Tpdo || !cfg.valid || cfg.extended || cfg.mapping.is_empty() {
                continue;
            }
            if (1..=240).contains(&cfg.trans_type) {
                continue;
            }
            let period = if cfg.event_timer_ms > 0 { cfg.event_timer_ms as u64 } else { DEFAULT_TPDO_PERIOD_MS };
            let due = *self.pdo_due.entry(cfg.number).or_insert(now_ms + period);
            if now_ms >= due {
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
            if let Some(r) = self.sdo(&f.data) {
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
        self.values.insert((index, sub), data);
        Ok(())
    }

    fn sdo(&mut self, d: &[u8; 8]) -> Option<[u8; 8]> {
        let index = u16::from_le_bytes([d[1], d[2]]);
        let sub = d[3];
        let abort = |index: u16, sub: u8, code: u32| {
            let i = index.to_le_bytes();
            let c = code.to_le_bytes();
            [0x80, i[0], i[1], sub, c[0], c[1], c[2], c[3]]
        };
        let ok_head = |cmd: u8| [cmd, d[1], d[2], d[3], 0, 0, 0, 0];
        match d[0] >> 5 {
            1 => {
                self.server = Server::Idle;
                let e = d[0] & 0x02 != 0;
                let s = d[0] & 0x01 != 0;
                if e {
                    let len = if s { 4 - ((d[0] >> 2) & 3) as usize } else { 4 };
                    match self.write(index, sub, d[4..4 + len].to_vec()) {
                        Ok(()) => Some(ok_head(0x60)),
                        Err(c) => Some(abort(index, sub, c)),
                    }
                } else {
                    // Check access before the segments come.
                    let o = self.od.objects.get(&index);
                    let code = match o.and_then(|o| o.subs.get(&sub)) {
                        None if o.is_none() => Some(0x0602_0000),
                        None => Some(0x0609_0011),
                        Some(e) if !e.access.writable() => Some(0x0601_0002),
                        _ => None,
                    };
                    if let Some(c) = code {
                        return Some(abort(index, sub, c));
                    }
                    self.server = Server::Download { index, sub, buf: Vec::new(), toggle: false };
                    Some(ok_head(0x60))
                }
            }
            0 => {
                let Server::Download { index, sub, mut buf, toggle } = std::mem::replace(&mut self.server, Server::Idle) else {
                    return Some(abort(0, 0, 0x0504_0001));
                };
                let t = d[0] & 0x10 != 0;
                if t != toggle {
                    return Some(abort(index, sub, 0x0503_0000));
                }
                let n = ((d[0] >> 1) & 7) as usize;
                buf.extend_from_slice(&d[1..8 - n]);
                if buf.len() > MAX_DOMAIN {
                    return Some(abort(index, sub, 0x0607_0012));
                }
                let resp = [0x20 | ((t as u8) << 4), 0, 0, 0, 0, 0, 0, 0];
                if d[0] & 0x01 != 0 {
                    return match self.write(index, sub, buf) {
                        Ok(()) => Some(resp),
                        Err(c) => Some(abort(index, sub, c)),
                    };
                }
                self.server = Server::Download { index, sub, buf, toggle: !toggle };
                Some(resp)
            }
            2 => {
                self.server = Server::Idle;
                match self.read(index, sub) {
                    Err(c) => Some(abort(index, sub, c)),
                    Ok(data) if (1..=4).contains(&data.len()) => {
                        let n = 4 - data.len() as u8;
                        let mut r = ok_head(0x43 | (n << 2));
                        r[4..4 + data.len()].copy_from_slice(&data);
                        Some(r)
                    }
                    Ok(data) => {
                        let mut r = ok_head(0x41);
                        r[4..8].copy_from_slice(&(data.len() as u32).to_le_bytes());
                        self.server = Server::Upload { data, pos: 0, toggle: false };
                        Some(r)
                    }
                }
            }
            3 => {
                let Server::Upload { data, pos, toggle } = std::mem::replace(&mut self.server, Server::Idle) else {
                    return Some(abort(0, 0, 0x0504_0001));
                };
                let t = d[0] & 0x10 != 0;
                if t != toggle {
                    return Some(abort(0, 0, 0x0503_0000));
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
                Some(r)
            }
            4 => {
                self.server = Server::Idle;
                None
            }
            _ => Some(abort(index, sub, 0x0504_0001)),
        }
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
