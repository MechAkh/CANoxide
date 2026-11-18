//! LSS (CiA 305): a master state machine and a slave state machine (the
//! simulator uses the slave). No I/O in here: frames and time go in, frames
//! to send and results come out.
//!
//! COB-IDs: 0x7E5 master to slave, 0x7E4 slave to master.
//!
//! WARNING: changing the node-ID or the bit timing of a device on a live
//! network can take that device off the bus (it no longer answers on its
//! old node-ID, or it no longer talks at the bitrate of the network).

use super::{COB_LSS_MASTER, COB_LSS_SLAVE};
use crate::can::{Bitrate, CanFrame};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CS_SWITCH_GLOBAL: u8 = 0x04;
pub const CS_CONFIGURE_NODE_ID: u8 = 0x11;
pub const CS_CONFIGURE_BIT_TIMING: u8 = 0x13;
pub const CS_ACTIVATE_BIT_TIMING: u8 = 0x15;
pub const CS_STORE: u8 = 0x17;
pub const CS_SELECT_VENDOR: u8 = 0x40;
pub const CS_SELECT_PRODUCT: u8 = 0x41;
pub const CS_SELECT_REVISION: u8 = 0x42;
pub const CS_SELECT_SERIAL: u8 = 0x43;
pub const CS_SELECT_RESPONSE: u8 = 0x44;
pub const CS_IDENTIFY_NON_CONFIGURED: u8 = 0x4C;
pub const CS_IDENTIFY_SLAVE: u8 = 0x4F;
pub const CS_NON_CONFIGURED_SLAVE: u8 = 0x50;
pub const CS_FASTSCAN: u8 = 0x51;
pub const CS_INQUIRE_VENDOR: u8 = 0x5A;
pub const CS_INQUIRE_PRODUCT: u8 = 0x5B;
pub const CS_INQUIRE_REVISION: u8 = 0x5C;
pub const CS_INQUIRE_SERIAL: u8 = 0x5D;
pub const CS_INQUIRE_NODE_ID: u8 = 0x5E;

/// Node-ID of an unconfigured LSS slave.
pub const NODE_ID_UNCONFIGURED: u8 = 255;
pub const DEFAULT_LSS_TIMEOUT_MS: u64 = 100;

/// The four values of the identity object (0x1018:01..04).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct LssIdentity {
    pub vendor: u32,
    pub product: u32,
    pub revision: u32,
    pub serial: u32,
}

impl LssIdentity {
    pub fn get(&self, i: u8) -> u32 {
        match i {
            0 => self.vendor,
            1 => self.product,
            2 => self.revision,
            _ => self.serial,
        }
    }
    fn set(&mut self, i: u8, v: u32) {
        match i {
            0 => self.vendor = v,
            1 => self.product = v,
            2 => self.revision = v,
            _ => self.serial = v,
        }
    }
}

/// Bit timing table 0 of CiA 305 (index -> kbit/s; 9 = automatic).
pub fn bit_timing_index(kbit: u32) -> Option<u8> {
    match kbit {
        1000 => Some(0),
        800 => Some(1),
        500 => Some(2),
        250 => Some(3),
        125 => Some(4),
        100 => Some(5),
        50 => Some(6),
        20 => Some(7),
        10 => Some(8),
        _ => None,
    }
}

/// kbit/s of an index of table 0 (`None` for 9 = automatic and unknown).
pub fn bit_timing_kbit(index: u8) -> Option<u32> {
    [1000, 800, 500, 250, 125, 100, 50, 20, 10].get(index as usize).copied()
}

pub fn bitrate_index(b: Bitrate) -> u8 {
    bit_timing_index(b.kbit()).unwrap_or(9)
}

/// Text of the error code of "configure node-ID".
pub fn node_id_error_text(code: u8) -> &'static str {
    match code {
        0 => "OK",
        1 => "Node-ID out of range",
        0xFF => "Implementation specific error",
        _ => "Reserved error code",
    }
}

/// Text of the error code of "configure bit timing".
pub fn bit_timing_error_text(code: u8) -> &'static str {
    match code {
        0 => "OK",
        1 => "Bit timing not supported",
        0xFF => "Implementation specific error",
        _ => "Reserved error code",
    }
}

/// Text of the error code of "store configuration".
pub fn store_error_text(code: u8) -> &'static str {
    match code {
        0 => "OK",
        1 => "Store configuration not supported",
        2 => "Storage media access error",
        0xFF => "Implementation specific error",
        _ => "Reserved error code",
    }
}

/// A service of the LSS master. Every command transmits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LssCommand {
    /// Switch state global: all slaves to configuration (true) or waiting.
    SwitchGlobal { configuration: bool },
    /// Switch state selective by identity; the slave confirms.
    SwitchSelective { identity: LssIdentity },
    /// Inquire the four identity values (one slave in configuration state).
    InquireIdentity,
    InquireNodeId,
    /// 1..127, or 255 = unconfigured.
    ConfigureNodeId { node_id: u8 },
    /// Table selector 0 = the standard table; `index` from `bit_timing_index`.
    ConfigureBitTiming { table: u8, index: u8 },
    /// Activate the bit timing after `delay_ms` (twice: before and after the switch).
    ActivateBitTiming { delay_ms: u16 },
    StoreConfiguration,
    /// Find one unconfigured slave by bit scan; it ends in configuration state.
    Fastscan,
    /// Ask all non-configured slaves to answer.
    IdentifyNonConfigured,
}

impl LssCommand {
    pub fn label(&self) -> &'static str {
        match self {
            LssCommand::SwitchGlobal { .. } => "switch state global",
            LssCommand::SwitchSelective { .. } => "switch state selective",
            LssCommand::InquireIdentity => "inquire identity",
            LssCommand::InquireNodeId => "inquire node-ID",
            LssCommand::ConfigureNodeId { .. } => "configure node-ID",
            LssCommand::ConfigureBitTiming { .. } => "configure bit timing",
            LssCommand::ActivateBitTiming { .. } => "activate bit timing",
            LssCommand::StoreConfiguration => "store configuration",
            LssCommand::Fastscan => "fastscan",
            LssCommand::IdentifyNonConfigured => "identify non-configured slaves",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LssResult {
    /// The service needs no answer, or the slave confirmed.
    Done,
    Identity(LssIdentity),
    NodeId(u8),
    /// Fastscan: the identity of the slave found (now in configuration
    /// state), or `None` when no unconfigured slave answered.
    Found(Option<LssIdentity>),
    /// Identify non-configured: number of answers.
    NonConfigured(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LssError {
    Timeout,
    /// The slave answered with an error code (and the specific error byte).
    Refused { code: u8, spec: u8, text: String },
    /// An answer that does not fit.
    Protocol(String),
    Cancelled,
    Busy,
    InvalidArgument(String),
}

impl fmt::Display for LssError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LssError::Timeout => write!(f, "LSS timeout, no answer"),
            LssError::Refused { code, spec, text } => write!(f, "LSS error {code} ({text}), specific error {spec}"),
            LssError::Protocol(s) => write!(f, "LSS protocol error: {s}"),
            LssError::Cancelled => write!(f, "cancelled"),
            LssError::Busy => write!(f, "LSS master busy"),
            LssError::InvalidArgument(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LssOutput {
    pub tx: Vec<CanFrame>,
    pub done: Option<Result<LssResult, LssError>>,
}

pub fn lss_frame(d: [u8; 8]) -> CanFrame {
    CanFrame::new(COB_LSS_MASTER, &d)
}

fn u32_frame(cs: u8, v: u32) -> CanFrame {
    let b = v.to_le_bytes();
    lss_frame([cs, b[0], b[1], b[2], b[3], 0, 0, 0])
}

fn fastscan_frame(id: u32, bit: u8, sub: u8, next: u8) -> CanFrame {
    let b = id.to_le_bytes();
    lss_frame([CS_FASTSCAN, b[0], b[1], b[2], b[3], bit, sub, next])
}

#[derive(Clone, Debug)]
enum Step {
    /// Waiting for one answer with this command specifier.
    Answer(u8),
    /// Inquire identity: value `i` asked.
    Inquire(u8, LssIdentity),
    /// Fastscan: the reset (bit 0x80) is out.
    FsStart,
    /// Fastscan: bit `bit` of value `sub` is asked with `id`.
    FsBit { sub: u8, bit: u8, id: u32, found: LssIdentity },
    /// Fastscan: bit 0 of value `sub` asked together with the switch to
    /// the next value (`second`: the try with bit 0 = 1).
    FsConfirm { sub: u8, id: u32, found: LssIdentity, second: bool },
    /// Collect 0x50 answers until the timeout.
    Collect(usize),
}

#[derive(Clone, Debug)]
struct Run {
    cmd: LssCommand,
    step: Step,
    deadline_ms: u64,
    steps_done: usize,
}

/// LSS master. One service at a time.
#[derive(Clone, Debug)]
pub struct LssMaster {
    pub timeout_ms: u64,
    run: Option<Run>,
}

impl Default for LssMaster {
    fn default() -> Self {
        LssMaster::new(DEFAULT_LSS_TIMEOUT_MS)
    }
}

/// Requests of a full fastscan: start, 4 x (32 bits + confirmation).
pub const FASTSCAN_STEPS: usize = 1 + 4 * 33;

impl LssMaster {
    pub fn new(timeout_ms: u64) -> LssMaster {
        LssMaster { timeout_ms: timeout_ms.max(1), run: None }
    }

    pub fn is_busy(&self) -> bool {
        self.run.is_some()
    }

    /// (steps done, total steps) of the running service.
    pub fn progress(&self) -> Option<(usize, usize)> {
        let r = self.run.as_ref()?;
        let total = match r.cmd {
            LssCommand::Fastscan => FASTSCAN_STEPS,
            LssCommand::InquireIdentity => 4,
            _ => 1,
        };
        Some((r.steps_done.min(total), total))
    }

    /// Start a service. Services without an answer end at once (`done`).
    pub fn start(&mut self, cmd: LssCommand, now_ms: u64) -> LssOutput {
        if self.run.is_some() {
            return LssOutput { tx: Vec::new(), done: Some(Err(LssError::Busy)) };
        }
        let mut tx = Vec::new();
        let step = match &cmd {
            LssCommand::SwitchGlobal { configuration } => {
                tx.push(lss_frame([CS_SWITCH_GLOBAL, *configuration as u8, 0, 0, 0, 0, 0, 0]));
                return LssOutput { tx, done: Some(Ok(LssResult::Done)) };
            }
            LssCommand::ActivateBitTiming { delay_ms } => {
                let d = delay_ms.to_le_bytes();
                tx.push(lss_frame([CS_ACTIVATE_BIT_TIMING, d[0], d[1], 0, 0, 0, 0, 0]));
                return LssOutput { tx, done: Some(Ok(LssResult::Done)) };
            }
            LssCommand::SwitchSelective { identity } => {
                for (i, cs) in [CS_SELECT_VENDOR, CS_SELECT_PRODUCT, CS_SELECT_REVISION, CS_SELECT_SERIAL].into_iter().enumerate() {
                    tx.push(u32_frame(cs, identity.get(i as u8)));
                }
                Step::Answer(CS_SELECT_RESPONSE)
            }
            LssCommand::InquireIdentity => {
                tx.push(lss_frame([CS_INQUIRE_VENDOR, 0, 0, 0, 0, 0, 0, 0]));
                Step::Inquire(0, LssIdentity::default())
            }
            LssCommand::InquireNodeId => {
                tx.push(lss_frame([CS_INQUIRE_NODE_ID, 0, 0, 0, 0, 0, 0, 0]));
                Step::Answer(CS_INQUIRE_NODE_ID)
            }
            LssCommand::ConfigureNodeId { node_id } => {
                if !(1..=127).contains(node_id) && *node_id != NODE_ID_UNCONFIGURED {
                    return LssOutput { tx, done: Some(Err(LssError::InvalidArgument("node-ID must be 1..127 or 255".into()))) };
                }
                tx.push(lss_frame([CS_CONFIGURE_NODE_ID, *node_id, 0, 0, 0, 0, 0, 0]));
                Step::Answer(CS_CONFIGURE_NODE_ID)
            }
            LssCommand::ConfigureBitTiming { table, index } => {
                tx.push(lss_frame([CS_CONFIGURE_BIT_TIMING, *table, *index, 0, 0, 0, 0, 0]));
                Step::Answer(CS_CONFIGURE_BIT_TIMING)
            }
            LssCommand::StoreConfiguration => {
                tx.push(lss_frame([CS_STORE, 0, 0, 0, 0, 0, 0, 0]));
                Step::Answer(CS_STORE)
            }
            LssCommand::Fastscan => {
                tx.push(fastscan_frame(0, 0x80, 0, 0));
                Step::FsStart
            }
            LssCommand::IdentifyNonConfigured => {
                tx.push(lss_frame([CS_IDENTIFY_NON_CONFIGURED, 0, 0, 0, 0, 0, 0, 0]));
                Step::Collect(0)
            }
        };
        self.run = Some(Run { cmd, step, deadline_ms: now_ms + self.timeout_ms, steps_done: 0 });
        LssOutput { tx, done: None }
    }

    /// Cancel the running service (nothing is sent).
    pub fn cancel(&mut self) -> LssOutput {
        if self.run.take().is_some() {
            LssOutput { tx: Vec::new(), done: Some(Err(LssError::Cancelled)) }
        } else {
            LssOutput::default()
        }
    }

    fn end(&mut self, r: Result<LssResult, LssError>) -> LssOutput {
        self.run = None;
        LssOutput { tx: Vec::new(), done: Some(r) }
    }

    fn next(&mut self, step: Step, frame: CanFrame, now_ms: u64) -> LssOutput {
        let r = self.run.as_mut().expect("run");
        r.step = step;
        r.steps_done += 1;
        r.deadline_ms = now_ms + self.timeout_ms;
        LssOutput { tx: vec![frame], done: None }
    }

    /// Fastscan: a bit was answered (`hit`) or timed out.
    fn fastscan_bit(&mut self, sub: u8, bit: u8, id: u32, found: LssIdentity, hit: bool, now_ms: u64) -> LssOutput {
        // A slave answers when the bits from `bit` up match: then the bit is 0.
        let id = if hit { id } else { id | (1u32 << bit) };
        if bit == 1 {
            // Bit 0 is asked together with the switch to the next value.
            let f = fastscan_frame(id, 0, sub, (sub + 1) % 4);
            return self.next(Step::FsConfirm { sub, id, found, second: false }, f, now_ms);
        }
        let b = bit - 1;
        self.next(Step::FsBit { sub, bit: b, id, found }, fastscan_frame(id, b, sub, sub), now_ms)
    }

    /// Check the timeout.
    pub fn poll(&mut self, now_ms: u64) -> LssOutput {
        let Some(r) = self.run.as_ref() else { return LssOutput::default() };
        if now_ms < r.deadline_ms {
            return LssOutput::default();
        }
        match r.step.clone() {
            Step::FsStart => self.end(Ok(LssResult::Found(None))),
            Step::FsBit { sub, bit, id, found } => self.fastscan_bit(sub, bit, id, found, false, now_ms),
            Step::FsConfirm { sub, id, found, second: false } => {
                let id = id | 1;
                self.next(Step::FsConfirm { sub, id, found, second: true }, fastscan_frame(id, 0, sub, (sub + 1) % 4), now_ms)
            }
            Step::FsConfirm { sub, .. } => self.end(Err(LssError::Protocol(format!("fastscan: no confirmation of identity value {sub}")))),
            Step::Collect(n) => self.end(Ok(LssResult::NonConfigured(n))),
            _ => self.end(Err(LssError::Timeout)),
        }
    }

    /// Feed a received frame; only 0x7E4 frames are used.
    pub fn on_frame(&mut self, f: &CanFrame, now_ms: u64) -> LssOutput {
        let Some(r) = self.run.as_ref() else { return LssOutput::default() };
        if f.extended || f.rtr || f.id != COB_LSS_SLAVE || f.dlc < 1 {
            return LssOutput::default();
        }
        let d = f.data;
        let cs = d[0];
        let v = u32::from_le_bytes([d[1], d[2], d[3], d[4]]);
        match r.step.clone() {
            Step::Answer(want) if cs == want => {
                let res = match cs {
                    CS_SELECT_RESPONSE => Ok(LssResult::Done),
                    CS_INQUIRE_NODE_ID => Ok(LssResult::NodeId(d[1])),
                    CS_CONFIGURE_NODE_ID | CS_CONFIGURE_BIT_TIMING | CS_STORE => {
                        if d[1] == 0 {
                            Ok(LssResult::Done)
                        } else {
                            let text = match cs {
                                CS_CONFIGURE_NODE_ID => node_id_error_text(d[1]),
                                CS_CONFIGURE_BIT_TIMING => bit_timing_error_text(d[1]),
                                _ => store_error_text(d[1]),
                            };
                            Err(LssError::Refused { code: d[1], spec: d[2], text: text.to_string() })
                        }
                    }
                    _ => Ok(LssResult::Done),
                };
                self.end(res)
            }
            Step::Inquire(i, mut id) if cs == CS_INQUIRE_VENDOR + i => {
                id.set(i, v);
                if i == 3 {
                    return self.end(Ok(LssResult::Identity(id)));
                }
                self.next(Step::Inquire(i + 1, id), lss_frame([CS_INQUIRE_VENDOR + i + 1, 0, 0, 0, 0, 0, 0, 0]), now_ms)
            }
            Step::FsStart if cs == CS_IDENTIFY_SLAVE => {
                self.next(Step::FsBit { sub: 0, bit: 31, id: 0, found: LssIdentity::default() }, fastscan_frame(0, 31, 0, 0), now_ms)
            }
            Step::FsBit { sub, bit, id, found } if cs == CS_IDENTIFY_SLAVE => self.fastscan_bit(sub, bit, id, found, true, now_ms),
            Step::FsConfirm { sub, id, mut found, .. } if cs == CS_IDENTIFY_SLAVE => {
                found.set(sub, id);
                if sub == 3 {
                    return self.end(Ok(LssResult::Found(Some(found))));
                }
                let n = sub + 1;
                self.next(Step::FsBit { sub: n, bit: 31, id: 0, found }, fastscan_frame(0, 31, n, n), now_ms)
            }
            Step::Collect(n) if cs == CS_NON_CONFIGURED_SLAVE => {
                if let Some(r) = self.run.as_mut() {
                    r.step = Step::Collect(n + 1);
                }
                LssOutput::default()
            }
            _ => LssOutput::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LssSlaveState {
    Waiting,
    Configuration,
}

/// What the slave asks its device to do after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LssAction {
    None,
    /// Switch to waiting with a changed node-ID: the device resets its
    /// communication and boots with this ID.
    ApplyNodeId(u8),
    /// Activate bit timing: table index and switch delay (recorded only).
    ActivateBitTiming { index: u8, delay_ms: u16 },
    /// Store configuration was asked.
    Store,
}

/// LSS slave (CiA 305) of one device.
#[derive(Clone, Debug)]
pub struct LssSlave {
    pub identity: LssIdentity,
    /// The active node-ID (255 = unconfigured).
    pub active_node_id: u8,
    /// The node-ID set by "configure node-ID", active after the switch to waiting.
    pub pending_node_id: u8,
    pub state: LssSlaveState,
    /// Bit timing index set by "configure bit timing".
    pub pending_bit_timing: Option<u8>,
    select_step: u8,
    fastscan_pos: u8,
}

impl LssSlave {
    pub fn new(identity: LssIdentity, node_id: u8) -> LssSlave {
        LssSlave {
            identity,
            active_node_id: node_id,
            pending_node_id: node_id,
            state: LssSlaveState::Waiting,
            pending_bit_timing: None,
            select_step: 0,
            fastscan_pos: 0,
        }
    }

    pub fn is_unconfigured(&self) -> bool {
        self.active_node_id == NODE_ID_UNCONFIGURED
    }

    /// Handle a master frame (0x7E5). Returns the answer and an action.
    pub fn handle(&mut self, d: &[u8; 8]) -> (Option<[u8; 8]>, LssAction) {
        let cs = d[0];
        let v = u32::from_le_bytes([d[1], d[2], d[3], d[4]]);
        let conf = self.state == LssSlaveState::Configuration;
        let reply = |cs: u8, b1: u8, b2: u8| Some([cs, b1, b2, 0, 0, 0, 0, 0]);
        let u32_reply = |cs: u8, v: u32| {
            let b = v.to_le_bytes();
            Some([cs, b[0], b[1], b[2], b[3], 0, 0, 0])
        };
        match cs {
            CS_SWITCH_GLOBAL => {
                if d[1] == 1 {
                    self.state = LssSlaveState::Configuration;
                } else if d[1] == 0 {
                    let was = self.state;
                    self.state = LssSlaveState::Waiting;
                    self.select_step = 0;
                    if was == LssSlaveState::Configuration && self.pending_node_id != self.active_node_id {
                        self.active_node_id = self.pending_node_id;
                        return (None, LssAction::ApplyNodeId(self.active_node_id));
                    }
                }
                (None, LssAction::None)
            }
            CS_SELECT_VENDOR..=CS_SELECT_SERIAL => {
                let i = cs - CS_SELECT_VENDOR;
                if i == 0 {
                    self.select_step = 0;
                }
                if i == self.select_step && v == self.identity.get(i) {
                    self.select_step += 1;
                    if self.select_step == 4 {
                        self.select_step = 0;
                        self.state = LssSlaveState::Configuration;
                        return (reply(CS_SELECT_RESPONSE, 0, 0), LssAction::None);
                    }
                } else {
                    self.select_step = 0;
                }
                (None, LssAction::None)
            }
            CS_CONFIGURE_NODE_ID if conf => {
                let n = d[1];
                if (1..=127).contains(&n) || n == NODE_ID_UNCONFIGURED {
                    self.pending_node_id = n;
                    (reply(CS_CONFIGURE_NODE_ID, 0, 0), LssAction::None)
                } else {
                    (reply(CS_CONFIGURE_NODE_ID, 1, 0), LssAction::None)
                }
            }
            CS_CONFIGURE_BIT_TIMING if conf => {
                if d[1] == 0 && bit_timing_kbit(d[2]).is_some() {
                    self.pending_bit_timing = Some(d[2]);
                    (reply(CS_CONFIGURE_BIT_TIMING, 0, 0), LssAction::None)
                } else {
                    (reply(CS_CONFIGURE_BIT_TIMING, 1, 0), LssAction::None)
                }
            }
            CS_ACTIVATE_BIT_TIMING if conf => {
                let delay_ms = u16::from_le_bytes([d[1], d[2]]);
                match self.pending_bit_timing {
                    Some(index) => (None, LssAction::ActivateBitTiming { index, delay_ms }),
                    None => (None, LssAction::None),
                }
            }
            CS_STORE if conf => (reply(CS_STORE, 0, 0), LssAction::Store),
            CS_INQUIRE_VENDOR..=CS_INQUIRE_SERIAL if conf => (u32_reply(cs, self.identity.get(cs - CS_INQUIRE_VENDOR)), LssAction::None),
            CS_INQUIRE_NODE_ID if conf => (reply(CS_INQUIRE_NODE_ID, self.active_node_id, 0), LssAction::None),
            CS_IDENTIFY_NON_CONFIGURED if self.is_unconfigured() => (reply(CS_NON_CONFIGURED_SLAVE, 0, 0), LssAction::None),
            CS_FASTSCAN if self.is_unconfigured() && self.state == LssSlaveState::Waiting => {
                let (bit, sub, next) = (d[5], d[6], d[7]);
                if bit == 0x80 {
                    self.fastscan_pos = 0;
                    return (reply(CS_IDENTIFY_SLAVE, 0, 0), LssAction::None);
                }
                if bit > 31 || sub > 3 || sub != self.fastscan_pos {
                    return (None, LssAction::None);
                }
                let mask = if bit == 0 { u32::MAX } else { u32::MAX << bit };
                if (v ^ self.identity.get(sub)) & mask != 0 {
                    return (None, LssAction::None);
                }
                if bit == 0 {
                    self.fastscan_pos = next;
                    if sub == 3 {
                        self.state = LssSlaveState::Configuration;
                    }
                }
                (reply(CS_IDENTIFY_SLAVE, 0, 0), LssAction::None)
            }
            _ => (None, LssAction::None),
        }
    }
}

/// Decoded text of an LSS frame (`master` = 0x7E5).
pub fn lss_text(master: bool, d: &[u8]) -> String {
    let Some(&cs) = d.first() else {
        return if master { "LSS master request (empty)" } else { "LSS slave response (empty)" }.to_string();
    };
    let b = |i: usize| d.get(i).copied().unwrap_or(0);
    let v = u32::from_le_bytes([b(1), b(2), b(3), b(4)]);
    let names = ["vendor", "product", "revision", "serial"];
    if master {
        match cs {
            CS_SWITCH_GLOBAL => format!("LSS switch state global {}", if b(1) == 1 { "configuration" } else { "waiting" }),
            CS_SELECT_VENDOR..=CS_SELECT_SERIAL => format!("LSS switch state selective {} 0x{v:08X}", names[(cs - CS_SELECT_VENDOR) as usize]),
            CS_CONFIGURE_NODE_ID => format!("LSS configure node-ID {}", b(1)),
            CS_CONFIGURE_BIT_TIMING => match bit_timing_kbit(b(2)) {
                Some(k) if b(1) == 0 => format!("LSS configure bit timing table {} index {} ({k} kbit/s)", b(1), b(2)),
                _ => format!("LSS configure bit timing table {} index {}", b(1), b(2)),
            },
            CS_ACTIVATE_BIT_TIMING => format!("LSS activate bit timing, delay {} ms", u16::from_le_bytes([b(1), b(2)])),
            CS_STORE => "LSS store configuration".to_string(),
            CS_INQUIRE_VENDOR..=CS_INQUIRE_SERIAL => format!("LSS inquire {}", names[(cs - CS_INQUIRE_VENDOR) as usize]),
            CS_INQUIRE_NODE_ID => "LSS inquire node-ID".to_string(),
            CS_IDENTIFY_NON_CONFIGURED => "LSS identify non-configured remote slave".to_string(),
            CS_FASTSCAN => {
                if b(5) == 0x80 {
                    "LSS fastscan start".to_string()
                } else {
                    format!("LSS fastscan {} 0x{v:08X} bit {} next {}", names[(b(6) & 3) as usize], b(5), b(7))
                }
            }
            _ => format!("LSS master request cs 0x{cs:02X}"),
        }
    } else {
        match cs {
            CS_SELECT_RESPONSE => "LSS switch state selective confirmed".to_string(),
            CS_CONFIGURE_NODE_ID => format!("LSS configure node-ID response: {}", node_id_error_text(b(1))),
            CS_CONFIGURE_BIT_TIMING => format!("LSS configure bit timing response: {}", bit_timing_error_text(b(1))),
            CS_STORE => format!("LSS store configuration response: {}", store_error_text(b(1))),
            CS_INQUIRE_VENDOR..=CS_INQUIRE_SERIAL => format!("LSS {} = 0x{v:08X}", names[(cs - CS_INQUIRE_VENDOR) as usize]),
            CS_INQUIRE_NODE_ID => format!("LSS node-ID = {}", b(1)),
            CS_IDENTIFY_SLAVE => "LSS identify slave (fastscan answer)".to_string(),
            CS_NON_CONFIGURED_SLAVE => "LSS non-configured remote slave".to_string(),
            _ => format!("LSS slave response cs 0x{cs:02X}"),
        }
    }
}
