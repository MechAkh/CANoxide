//! Network overview data: per node the NMT state, last seen time, device
//! type, error register, identity, device name and the last EMCY. The
//! passive part comes from the frames; `NodeInfoTask` reads the objects by
//! SDO (only on a user command). `heartbeat_steps` plans "set heartbeat of
//! all nodes".

use crate::can::CanFrame;
use crate::canopen::decode::emcy_code_text;
use crate::canopen::nmt::NodeState;
use crate::canopen::sdo::{SdoError, SdoOutcome, SdoRequest, TransferMode};
use crate::canopen::task::{SdoCall, SdoStep, SdoTask, TaskResult};
use crate::canopen::types::{ascii, le_u64};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmcyRecord {
    pub code: u16,
    pub register: u8,
    pub text: String,
    /// Manufacturer bytes 3..7.
    pub data: Vec<u8>,
    pub time_us: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node: u8,
    /// NMT state label from the last heartbeat ("Operational").
    pub state: Option<String>,
    /// Bus time (us) of the last frame from the node.
    pub last_seen_us: Option<u64>,
    /// 0x1000.
    pub device_type: Option<u32>,
    /// 0x1001.
    pub error_register: Option<u8>,
    /// 0x1018:01..04.
    pub vendor_id: Option<u32>,
    pub product_code: Option<u32>,
    pub revision: Option<u32>,
    pub serial: Option<u32>,
    /// 0x1008 when readable.
    pub device_name: Option<String>,
    pub last_emcy: Option<EmcyRecord>,
    /// Errors of the last "refresh node info" (object, text).
    pub read_errors: Vec<String>,
}

impl NodeInfo {
    pub fn new(node: u8) -> NodeInfo {
        NodeInfo { node, ..Default::default() }
    }
}

/// The node table of the network overview.
#[derive(Clone, Debug, Default)]
pub struct NetworkOverview {
    nodes: BTreeMap<u8, NodeInfo>,
}

impl NetworkOverview {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Add a node (found by a scan) when it is not known yet.
    pub fn add(&mut self, node: u8) -> &mut NodeInfo {
        self.nodes.entry(node).or_insert_with(|| NodeInfo::new(node))
    }

    pub fn get(&self, node: u8) -> Option<&NodeInfo> {
        self.nodes.get(&node)
    }

    pub fn nodes(&self) -> Vec<&NodeInfo> {
        self.nodes.values().collect()
    }

    pub fn known_nodes(&self) -> Vec<u8> {
        self.nodes.keys().copied().collect()
    }

    /// Update from a received frame (heartbeat, EMCY, SDO response, PDO).
    pub fn on_frame(&mut self, f: &CanFrame) {
        if f.extended || f.dir != crate::can::Dir::Rx {
            return;
        }
        let node = (f.id & 0x7F) as u8;
        if node == 0 {
            return;
        }
        let d = f.payload();
        match f.id & !0x7F {
            0x700 if !f.rtr && !d.is_empty() => {
                let n = self.add(node);
                n.state = Some(NodeState::from_byte(d[0]).label().to_string());
                n.last_seen_us = Some(f.timestamp_us);
            }
            0x080 if d.len() >= 3 => {
                let code = u16::from_le_bytes([d[0], d[1]]);
                let n = self.add(node);
                n.last_emcy = Some(EmcyRecord {
                    code,
                    register: d[2],
                    text: emcy_code_text(code).to_string(),
                    data: d[3..].to_vec(),
                    time_us: f.timestamp_us,
                });
                n.error_register = Some(d[2]);
                n.last_seen_us = Some(f.timestamp_us);
            }
            0x580 | 0x180 | 0x280 | 0x380 | 0x480 => {
                if let Some(n) = self.nodes.get_mut(&node) {
                    n.last_seen_us = Some(f.timestamp_us);
                } else if f.id & !0x7F == 0x580 {
                    self.add(node).last_seen_us = Some(f.timestamp_us);
                }
            }
            _ => {}
        }
    }

    /// Take the SDO-read fields of a `NodeInfoTask` result.
    pub fn merge(&mut self, info: &NodeInfo) {
        let n = self.add(info.node);
        n.device_type = info.device_type.or(n.device_type);
        n.error_register = info.error_register.or(n.error_register);
        n.vendor_id = info.vendor_id.or(n.vendor_id);
        n.product_code = info.product_code.or(n.product_code);
        n.revision = info.revision.or(n.revision);
        n.serial = info.serial.or(n.serial);
        n.device_name = info.device_name.clone().or(n.device_name.take());
        n.read_errors = info.read_errors.clone();
    }
}

const INFO_OBJECTS: [(u16, u8); 7] = [(0x1000, 0), (0x1001, 0), (0x1018, 1), (0x1018, 2), (0x1018, 3), (0x1018, 4), (0x1008, 0)];

/// "Refresh node info": reads 0x1000, 0x1001, 0x1018:01..04 and 0x1008
/// of each node, one request at a time. A node that does not answer
/// 0x1000 in time is skipped.
pub struct NodeInfoTask {
    queue: VecDeque<(u8, u16, u8)>,
    total: usize,
    done: usize,
    current: Option<(u8, u16, u8)>,
    infos: BTreeMap<u8, NodeInfo>,
    stopped: bool,
}

impl NodeInfoTask {
    pub fn new(nodes: &[u8]) -> NodeInfoTask {
        let mut queue = VecDeque::new();
        let mut infos = BTreeMap::new();
        for &n in nodes {
            infos.insert(n, NodeInfo::new(n));
            for (i, s) in INFO_OBJECTS {
                queue.push_back((n, i, s));
            }
        }
        let total = queue.len();
        NodeInfoTask { queue, total, done: 0, current: None, infos, stopped: false }
    }
}

impl SdoTask for NodeInfoTask {
    fn label(&self) -> String {
        "refresh node info".to_string()
    }

    fn next(&mut self) -> Option<SdoCall> {
        if self.stopped {
            return None;
        }
        let (node, index, sub) = self.queue.pop_front()?;
        self.current = Some((node, index, sub));
        Some(SdoCall { node, req: SdoRequest::Upload { index, sub }, mode: TransferMode::Auto })
    }

    fn on_outcome(&mut self, o: &SdoOutcome) {
        let Some((node, index, sub)) = self.current.take() else { return };
        self.done += 1;
        let Some(info) = self.infos.get_mut(&node) else { return };
        match &o.result {
            Ok(d) => {
                let v = le_u64(d) as u32;
                match (index, sub) {
                    (0x1000, _) => info.device_type = Some(v),
                    (0x1001, _) => info.error_register = Some(v as u8),
                    (0x1018, 1) => info.vendor_id = Some(v),
                    (0x1018, 2) => info.product_code = Some(v),
                    (0x1018, 3) => info.revision = Some(v),
                    (0x1018, 4) => info.serial = Some(v),
                    (0x1008, _) => info.device_name = Some(ascii(d).trim_end_matches('\0').trim_end_matches('.').to_string()),
                    _ => {}
                }
            }
            Err(SdoError::Cancelled) => self.stopped = true,
            Err(e) => {
                info.read_errors.push(format!("0x{index:04X}:{sub:02X}: {e}"));
                if index == 0x1000 && *e == SdoError::Timeout {
                    // The node does not answer: skip its other objects.
                    let before = self.queue.len();
                    self.queue.retain(|q| q.0 != node);
                    self.done += before - self.queue.len();
                }
            }
        }
    }

    fn progress(&self) -> (usize, Option<usize>) {
        (self.done, Some(self.total))
    }

    fn cancel(&mut self) {
        self.stopped = true;
    }

    fn into_result(self: Box<Self>) -> TaskResult {
        TaskResult::NodeInfo(self.infos.into_values().collect())
    }
}

/// Write the producer heartbeat time 0x1017 on each node.
pub fn heartbeat_steps(nodes: &[u8], ms: u16) -> Vec<SdoStep> {
    nodes.iter().map(|&n| SdoStep::write(n, 0x1017, 0, ms.to_le_bytes().to_vec(), format!("node {n}: heartbeat {ms} ms"))).collect()
}
