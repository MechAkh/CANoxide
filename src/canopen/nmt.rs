//! NMT commands and node-state tracking from heartbeat / boot-up.

use super::{COB_HEARTBEAT, COB_NMT, COB_SYNC};
use crate::can::CanFrame;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NmtCommand {
    Start,
    Stop,
    PreOperational,
    ResetNode,
    ResetCommunication,
}

impl NmtCommand {
    pub const ALL: [NmtCommand; 5] = [
        NmtCommand::Start,
        NmtCommand::Stop,
        NmtCommand::PreOperational,
        NmtCommand::ResetNode,
        NmtCommand::ResetCommunication,
    ];

    pub fn code(self) -> u8 {
        match self {
            NmtCommand::Start => 0x01,
            NmtCommand::Stop => 0x02,
            NmtCommand::PreOperational => 0x80,
            NmtCommand::ResetNode => 0x81,
            NmtCommand::ResetCommunication => 0x82,
        }
    }

    pub fn from_code(c: u8) -> Option<NmtCommand> {
        NmtCommand::ALL.iter().copied().find(|x| x.code() == c)
    }

    pub fn label(self) -> &'static str {
        match self {
            NmtCommand::Start => "Start",
            NmtCommand::Stop => "Stop",
            NmtCommand::PreOperational => "Enter pre-operational",
            NmtCommand::ResetNode => "Reset node",
            NmtCommand::ResetCommunication => "Reset communication",
        }
    }
}

/// NMT frame for one node (1..127) or all nodes (0).
pub fn nmt_frame(cmd: NmtCommand, node: u8) -> CanFrame {
    CanFrame::new(COB_NMT, &[cmd.code(), node])
}

pub fn sync_frame() -> CanFrame {
    CanFrame::new(COB_SYNC, &[])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    BootUp,
    Stopped,
    Operational,
    PreOperational,
    Unknown(u8),
}

impl NodeState {
    pub fn from_byte(b: u8) -> NodeState {
        match b & 0x7F {
            0x00 => NodeState::BootUp,
            0x04 => NodeState::Stopped,
            0x05 => NodeState::Operational,
            0x7F => NodeState::PreOperational,
            x => NodeState::Unknown(x),
        }
    }
    pub fn byte(self) -> u8 {
        match self {
            NodeState::BootUp => 0x00,
            NodeState::Stopped => 0x04,
            NodeState::Operational => 0x05,
            NodeState::PreOperational => 0x7F,
            NodeState::Unknown(x) => x,
        }
    }
    pub fn label(self) -> String {
        match self {
            NodeState::BootUp => "Boot-up".to_string(),
            NodeState::Stopped => "Stopped".to_string(),
            NodeState::Operational => "Operational".to_string(),
            NodeState::PreOperational => "Pre-operational".to_string(),
            NodeState::Unknown(x) => format!("State 0x{x:02X}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeEntry {
    pub state: NodeState,
    pub last_seen_us: u64,
    pub count: u64,
}

#[derive(Clone, Debug, Default)]
pub struct NodeTracker {
    pub nodes: BTreeMap<u8, NodeEntry>,
}

impl NodeTracker {
    /// Feed a frame; true when it was a heartbeat / boot-up.
    pub fn on_frame(&mut self, f: &CanFrame) -> bool {
        if f.extended || f.rtr || f.dlc < 1 {
            return false;
        }
        if f.id <= COB_HEARTBEAT || f.id > COB_HEARTBEAT + 127 {
            return false;
        }
        let node = (f.id - COB_HEARTBEAT) as u8;
        let state = NodeState::from_byte(f.data[0]);
        let e = self.nodes.entry(node).or_insert(NodeEntry { state, last_seen_us: 0, count: 0 });
        e.state = state;
        e.last_seen_us = f.timestamp_us;
        e.count += 1;
        true
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
    }
}
