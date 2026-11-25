//! CiA 402 drive profile: the state of the drive from the statusword
//! 0x6041, the controlword 0x6040 commands, which command is allowed in
//! which state, and the names of the modes of operation 0x6060 / 0x6061.
//!
//! The masks follow the published state machine of CiA 402 (IEC
//! 61800-7-201): the state is in bits 0, 1, 2, 3, 5 and 6 of the
//! statusword; "Not ready", "Switch on disabled", "Fault reaction active"
//! and "Fault" need mask 0x4F, the other four mask 0x6F (bit 5 "quick
//! stop" tells "Quick stop active" from "Operation enabled").

use serde::{Deserialize, Serialize};

pub const CONTROLWORD: u16 = 0x6040;
pub const STATUSWORD: u16 = 0x6041;
pub const MODES_OF_OPERATION: u16 = 0x6060;
pub const MODES_DISPLAY: u16 = 0x6061;
pub const POSITION_ACTUAL: u16 = 0x6064;
pub const VELOCITY_ACTUAL: u16 = 0x606C;
pub const TORQUE_ACTUAL: u16 = 0x6077;
pub const ERROR_CODE: u16 = 0x603F;
pub const TARGET_TORQUE: u16 = 0x6071;
pub const TARGET_POSITION: u16 = 0x607A;
pub const TARGET_VELOCITY: u16 = 0x60FF;

/// Statusword bit 4: voltage enabled.
pub const SW_VOLTAGE_ENABLED: u16 = 1 << 4;
/// Statusword bit 5: quick stop (0 = a quick stop is active).
pub const SW_QUICK_STOP: u16 = 1 << 5;
/// Statusword bit 7: warning.
pub const SW_WARNING: u16 = 1 << 7;
/// Statusword bit 9: remote (the controlword is in charge).
pub const SW_REMOTE: u16 = 1 << 9;
/// Statusword bit 10: target reached.
pub const SW_TARGET_REACHED: u16 = 1 << 10;
/// Statusword bit 11: internal limit active.
pub const SW_INTERNAL_LIMIT: u16 = 1 << 11;

/// The eight states of the CiA 402 state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveState {
    NotReadyToSwitchOn,
    SwitchOnDisabled,
    ReadyToSwitchOn,
    SwitchedOn,
    OperationEnabled,
    QuickStopActive,
    FaultReactionActive,
    Fault,
}

impl DriveState {
    /// All states, in the order of the state machine.
    pub const ALL: [DriveState; 8] = [
        DriveState::NotReadyToSwitchOn,
        DriveState::SwitchOnDisabled,
        DriveState::ReadyToSwitchOn,
        DriveState::SwitchedOn,
        DriveState::OperationEnabled,
        DriveState::QuickStopActive,
        DriveState::FaultReactionActive,
        DriveState::Fault,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DriveState::NotReadyToSwitchOn => "Not ready to switch on",
            DriveState::SwitchOnDisabled => "Switch on disabled",
            DriveState::ReadyToSwitchOn => "Ready to switch on",
            DriveState::SwitchedOn => "Switched on",
            DriveState::OperationEnabled => "Operation enabled",
            DriveState::QuickStopActive => "Quick stop active",
            DriveState::FaultReactionActive => "Fault reaction active",
            DriveState::Fault => "Fault",
        }
    }

    /// (mask, value) of the statusword for this state.
    pub fn pattern(self) -> (u16, u16) {
        match self {
            DriveState::NotReadyToSwitchOn => (0x4F, 0x00),
            DriveState::SwitchOnDisabled => (0x4F, 0x40),
            DriveState::ReadyToSwitchOn => (0x6F, 0x21),
            DriveState::SwitchedOn => (0x6F, 0x23),
            DriveState::OperationEnabled => (0x6F, 0x27),
            DriveState::QuickStopActive => (0x6F, 0x07),
            DriveState::FaultReactionActive => (0x4F, 0x0F),
            DriveState::Fault => (0x4F, 0x08),
        }
    }

    /// The statusword bits of this state alone (no other bits set).
    pub fn bits(self) -> u16 {
        self.pattern().1
    }

    /// The commands that the state machine accepts in this state.
    pub fn allowed(self) -> &'static [Command] {
        use Command::*;
        match self {
            DriveState::NotReadyToSwitchOn | DriveState::FaultReactionActive => &[],
            DriveState::SwitchOnDisabled => &[Shutdown],
            DriveState::ReadyToSwitchOn => &[SwitchOn, DisableVoltage, QuickStop],
            DriveState::SwitchedOn => &[EnableOperation, Shutdown, DisableVoltage, QuickStop],
            DriveState::OperationEnabled => &[DisableOperation, Shutdown, DisableVoltage, QuickStop],
            DriveState::QuickStopActive => &[EnableOperation, DisableVoltage],
            DriveState::Fault => &[FaultReset],
        }
    }

    pub fn allows(self, c: Command) -> bool {
        self.allowed().contains(&c)
    }
}

/// The state of a statusword, or `None` for a bit pattern that is no state.
pub fn decode_state(sw: u16) -> Option<DriveState> {
    DriveState::ALL.into_iter().find(|s| {
        let (m, v) = s.pattern();
        sw & m == v
    })
}

/// The named bits of a statusword.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusBits {
    pub voltage_enabled: bool,
    pub warning: bool,
    pub remote: bool,
    pub target_reached: bool,
    pub internal_limit_active: bool,
}

impl StatusBits {
    pub fn of(sw: u16) -> StatusBits {
        StatusBits {
            voltage_enabled: sw & SW_VOLTAGE_ENABLED != 0,
            warning: sw & SW_WARNING != 0,
            remote: sw & SW_REMOTE != 0,
            target_reached: sw & SW_TARGET_REACHED != 0,
            internal_limit_active: sw & SW_INTERNAL_LIMIT != 0,
        }
    }

    /// (name, set) of each bit, in bit order.
    pub fn list(&self) -> [(&'static str, bool); 5] {
        [
            ("Voltage enabled", self.voltage_enabled),
            ("Warning", self.warning),
            ("Remote", self.remote),
            ("Target reached", self.target_reached),
            ("Internal limit active", self.internal_limit_active),
        ]
    }
}

/// A controlword command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    Shutdown,
    SwitchOn,
    EnableOperation,
    DisableVoltage,
    QuickStop,
    DisableOperation,
    FaultReset,
}

impl Command {
    pub const ALL: [Command; 7] = [
        Command::Shutdown,
        Command::SwitchOn,
        Command::EnableOperation,
        Command::DisableOperation,
        Command::DisableVoltage,
        Command::QuickStop,
        Command::FaultReset,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::Shutdown => "Shutdown",
            Command::SwitchOn => "Switch on",
            Command::EnableOperation => "Enable operation",
            Command::DisableVoltage => "Disable voltage",
            Command::QuickStop => "Quick stop",
            Command::DisableOperation => "Disable operation",
            Command::FaultReset => "Fault reset",
        }
    }

    /// The CLI word.
    pub fn word(self) -> &'static str {
        match self {
            Command::Shutdown => "shutdown",
            Command::SwitchOn => "switch-on",
            Command::EnableOperation => "enable",
            Command::DisableVoltage => "disable-voltage",
            Command::QuickStop => "quick-stop",
            Command::DisableOperation => "disable-operation",
            Command::FaultReset => "fault-reset",
        }
    }

    pub fn from_word(w: &str) -> Option<Command> {
        Command::ALL.into_iter().find(|c| c.word() == w)
    }

    /// The controlword values to write, in order. A fault reset is the
    /// rising edge of bit 7, so it writes 0x80 and then 0x00.
    pub fn controlwords(self) -> &'static [u16] {
        match self {
            Command::Shutdown => &[0x06],
            Command::SwitchOn => &[0x07],
            Command::EnableOperation => &[0x0F],
            Command::DisableVoltage => &[0x00],
            Command::QuickStop => &[0x02],
            Command::DisableOperation => &[0x07],
            Command::FaultReset => &[0x80, 0x00],
        }
    }

    /// True for a command that can make the drive move.
    pub fn moves(self) -> bool {
        self == Command::EnableOperation
    }
}

/// The command that a controlword gives (bits 0..3 and 7), by the
/// transition masks of the state machine. `prev` is the controlword before,
/// for the edge of the fault reset.
pub fn command_of(cw: u16, prev: u16) -> Option<Command> {
    if cw & 0x80 != 0 && prev & 0x80 == 0 {
        return Some(Command::FaultReset);
    }
    if cw & 0x02 == 0 {
        return Some(Command::DisableVoltage);
    }
    if cw & 0x06 == 0x02 {
        return Some(Command::QuickStop);
    }
    match cw & 0x0F {
        0x06 | 0x0E => Some(Command::Shutdown),
        0x07 => Some(Command::SwitchOn),
        0x0F => Some(Command::EnableOperation),
        _ => None,
    }
}

/// The state after a controlword (the drive side of the state machine,
/// for the simulator). `Disable operation` and `Switch on` share 0x07.
pub fn transition(state: DriveState, cw: u16, prev: u16) -> DriveState {
    use DriveState::*;
    let Some(c) = command_of(cw, prev) else { return state };
    match (state, c) {
        (Fault, Command::FaultReset) => SwitchOnDisabled,
        (Fault | FaultReactionActive | NotReadyToSwitchOn, _) => state,
        (_, Command::FaultReset) => state,
        (SwitchOnDisabled, Command::Shutdown) => ReadyToSwitchOn,
        (SwitchOnDisabled, _) => SwitchOnDisabled,
        (QuickStopActive, Command::DisableVoltage) => SwitchOnDisabled,
        (QuickStopActive, Command::EnableOperation) => OperationEnabled,
        (QuickStopActive, _) => QuickStopActive,
        (_, Command::DisableVoltage) => SwitchOnDisabled,
        (OperationEnabled, Command::QuickStop) => QuickStopActive,
        (_, Command::QuickStop) => SwitchOnDisabled,
        (ReadyToSwitchOn, Command::SwitchOn) => SwitchedOn,
        (ReadyToSwitchOn, Command::EnableOperation) => OperationEnabled,
        (SwitchedOn, Command::EnableOperation) => OperationEnabled,
        (SwitchedOn, Command::Shutdown) => ReadyToSwitchOn,
        (OperationEnabled, Command::Shutdown) => ReadyToSwitchOn,
        (OperationEnabled, Command::SwitchOn | Command::DisableOperation) => SwitchedOn,
        (s, _) => s,
    }
}

/// The standard name of a mode of operation (0x6060 / 0x6061).
pub fn mode_name(mode: i8) -> Option<&'static str> {
    Some(match mode {
        1 => "Profile position (pp)",
        3 => "Profile velocity (pv)",
        4 => "Profile torque (tq)",
        6 => "Homing (hm)",
        7 => "Interpolated position (ip)",
        8 => "Cyclic synchronous position (csp)",
        9 => "Cyclic synchronous velocity (csv)",
        10 => "Cyclic synchronous torque (cst)",
        _ => return None,
    })
}

/// The standard modes, in number order.
pub const MODES: [i8; 8] = [1, 3, 4, 6, 7, 8, 9, 10];

/// "3 Profile velocity (pv)" or "-1 (manufacturer)" / "0 (no mode)".
pub fn mode_text(mode: i8) -> String {
    match mode_name(mode) {
        Some(n) => format!("{mode} {n}"),
        None if mode < 0 => format!("{mode} (manufacturer)"),
        None if mode == 0 => "0 (no mode)".to_string(),
        None => format!("{mode} (reserved)"),
    }
}

/// The objects that a status read polls, in order.
pub const POLL_OBJECTS: [u16; 6] = [STATUSWORD, MODES_DISPLAY, POSITION_ACTUAL, VELOCITY_ACTUAL, TORQUE_ACTUAL, ERROR_CODE];

/// The status of a drive from the polled objects (each one optional).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DriveStatus {
    pub statusword: Option<u16>,
    pub state: Option<DriveState>,
    pub bits: Option<StatusBits>,
    pub mode: Option<i8>,
    pub mode_name: Option<String>,
    pub position: Option<i32>,
    pub velocity: Option<i32>,
    pub torque: Option<i16>,
    pub error_code: Option<u16>,
}

fn le_u16(d: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*d.first()?, *d.get(1)?]))
}

fn le_i32(d: &[u8]) -> Option<i32> {
    Some(i32::from_le_bytes([*d.first()?, *d.get(1)?, *d.get(2)?, *d.get(3)?]))
}

impl DriveStatus {
    /// Set one polled object from its raw bytes.
    pub fn set(&mut self, index: u16, data: &[u8]) {
        match index {
            STATUSWORD => {
                if let Some(sw) = le_u16(data) {
                    self.statusword = Some(sw);
                    self.state = decode_state(sw);
                    self.bits = Some(StatusBits::of(sw));
                }
            }
            MODES_DISPLAY => {
                if let Some(&b) = data.first() {
                    self.mode = Some(b as i8);
                    self.mode_name = Some(mode_text(b as i8));
                }
            }
            POSITION_ACTUAL => self.position = le_i32(data),
            VELOCITY_ACTUAL => self.velocity = le_i32(data),
            TORQUE_ACTUAL => self.torque = le_u16(data).map(|v| v as i16),
            ERROR_CODE => self.error_code = le_u16(data),
            _ => {}
        }
    }

    pub fn from_reads(r: &crate::canopen::task::ReadsReport) -> DriveStatus {
        let mut s = DriveStatus::default();
        for x in &r.reads {
            if let Some(d) = &x.data {
                s.set(x.index, d);
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_from_its_mask() {
        for s in DriveState::ALL {
            assert_eq!(decode_state(s.bits()), Some(s), "{s:?}");
            // Other bits do not change the state.
            assert_eq!(decode_state(s.bits() | 0xFF00 | 0x10 | 0x80), Some(s), "{s:?} with other bits");
        }
        // Typical words of real drives.
        assert_eq!(decode_state(0x0250), Some(DriveState::SwitchOnDisabled));
        assert_eq!(decode_state(0x0231), Some(DriveState::ReadyToSwitchOn));
        assert_eq!(decode_state(0x0233), Some(DriveState::SwitchedOn));
        assert_eq!(decode_state(0x0237), Some(DriveState::OperationEnabled));
        assert_eq!(decode_state(0x0217), Some(DriveState::QuickStopActive));
        assert_eq!(decode_state(0x0218), Some(DriveState::Fault));
        assert_eq!(decode_state(0x021F), Some(DriveState::FaultReactionActive));
        // No state: bit 6 and bit 0 together with bit 5 clear and bit 3 clear.
        assert_eq!(decode_state(0x0041), None);
    }

    #[test]
    fn named_bits() {
        let b = StatusBits::of(0x0E90);
        assert!(b.voltage_enabled && b.warning && b.remote && b.target_reached && b.internal_limit_active);
        assert_eq!(StatusBits::of(0x0040), StatusBits::default());
    }

    #[test]
    fn allowed_commands_per_state() {
        use Command::*;
        use DriveState::*;
        assert!(NotReadyToSwitchOn.allowed().is_empty());
        assert!(FaultReactionActive.allowed().is_empty());
        assert_eq!(SwitchOnDisabled.allowed(), &[Shutdown]);
        assert!(ReadyToSwitchOn.allows(SwitchOn) && !ReadyToSwitchOn.allows(DisableOperation));
        assert!(SwitchedOn.allows(EnableOperation) && !SwitchedOn.allows(SwitchOn));
        assert!(OperationEnabled.allows(DisableOperation) && OperationEnabled.allows(QuickStop) && !OperationEnabled.allows(EnableOperation));
        assert!(QuickStopActive.allows(DisableVoltage) && !QuickStopActive.allows(QuickStop));
        assert_eq!(Fault.allowed(), &[FaultReset]);
        // Each allowed command changes the state (no dead button).
        for s in DriveState::ALL {
            for c in s.allowed() {
                let mut st = s;
                let mut prev = 0u16;
                for &cw in c.controlwords() {
                    st = transition(st, cw, prev);
                    prev = cw;
                }
                assert_ne!(st, s, "{c:?} in {s:?}");
            }
        }
    }

    #[test]
    fn controlwords_and_transitions() {
        use DriveState::*;
        assert_eq!(Command::FaultReset.controlwords(), &[0x80, 0x00]);
        let mut s = SwitchOnDisabled;
        s = transition(s, 0x06, 0);
        assert_eq!(s, ReadyToSwitchOn);
        s = transition(s, 0x07, 0x06);
        assert_eq!(s, SwitchedOn);
        s = transition(s, 0x0F, 0x07);
        assert_eq!(s, OperationEnabled);
        s = transition(s, 0x07, 0x0F);
        assert_eq!(s, SwitchedOn, "disable operation");
        s = transition(s, 0x0F, 0x07);
        s = transition(s, 0x02, 0x0F);
        assert_eq!(s, QuickStopActive);
        s = transition(s, 0x00, 0x02);
        assert_eq!(s, SwitchOnDisabled);
        assert_eq!(transition(Fault, 0x80, 0x00), SwitchOnDisabled);
        assert_eq!(transition(Fault, 0x80, 0x80), Fault, "no edge, no reset");
        assert_eq!(transition(Fault, 0x06, 0x00), Fault);
    }

    #[test]
    fn mode_names() {
        assert_eq!(mode_name(3), Some("Profile velocity (pv)"));
        assert_eq!(mode_name(10), Some("Cyclic synchronous torque (cst)"));
        assert_eq!(mode_name(2), None);
        assert_eq!(mode_text(-3), "-3 (manufacturer)");
        for m in MODES {
            assert!(mode_name(m).is_some());
        }
        for c in Command::ALL {
            assert_eq!(Command::from_word(c.word()), Some(c));
        }
    }

    // The simulated axis through the state machine, over SDO.
    use crate::can::sim::SimNode;
    use crate::canopen::task::{ReadsTask, SdoStep, StepsTask, TaskResult};
    use crate::session::{Ctl, Session};

    fn cmd(s: &mut Session, c: Command) {
        let steps = c.controlwords().iter().map(|cw| SdoStep::write(1, CONTROLWORD, 0, cw.to_le_bytes().to_vec(), c.name())).collect();
        let r = s.run_task(Box::new(StepsTask::new(c.name(), steps, true)), &mut Ctl::default()).unwrap();
        let TaskResult::Steps(r) = r else { panic!("wrong result") };
        assert!(r.ok, "{c:?}");
    }

    fn status(s: &mut Session) -> DriveStatus {
        let items = POLL_OBJECTS.iter().map(|i| (*i, 0)).collect();
        let r = s.run_task(Box::new(ReadsTask::new("status", 1, items)), &mut Ctl::default()).unwrap();
        let TaskResult::Reads(r) = r else { panic!("wrong result") };
        DriveStatus::from_reads(&r)
    }

    #[test]
    fn sim_axis_state_machine() {
        use DriveState::*;
        let mut s = Session::sim(SimNode::builtin(1), false);
        assert_eq!(status(&mut s).state, Some(SwitchOnDisabled));
        for (c, want) in [(Command::Shutdown, ReadyToSwitchOn), (Command::SwitchOn, SwitchedOn), (Command::EnableOperation, OperationEnabled), (Command::QuickStop, QuickStopActive)] {
            cmd(&mut s, c);
            let st = status(&mut s);
            assert_eq!(st.state, Some(want), "after {c:?}");
            assert_eq!(s.sim_node().unwrap().drive_state(), want);
            assert!(want.allowed().contains(&Command::DisableVoltage) || want == SwitchOnDisabled);
        }
        cmd(&mut s, Command::DisableVoltage);
        assert_eq!(status(&mut s).state, Some(SwitchOnDisabled));
        // A fault: only Fault reset leaves it, and it clears 0x603F.
        s.sim_node().unwrap().drive_fault(0x2310);
        let st = status(&mut s);
        assert_eq!(st.state, Some(Fault));
        assert_eq!(st.error_code, Some(0x2310));
        cmd(&mut s, Command::Shutdown);
        assert_eq!(status(&mut s).state, Some(Fault), "Shutdown does not leave Fault");
        cmd(&mut s, Command::FaultReset);
        let st = status(&mut s);
        assert_eq!(st.state, Some(SwitchOnDisabled));
        assert_eq!(st.error_code, Some(0));
    }
}