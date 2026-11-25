//! CAN frame model and the adapter trait. Classic CAN only (no CAN FD).

// `unsafe` code is only in the PCAN, Sontheim and Kvaser FFI modules, the
// COM port of the SLCAN module and the Win32 queries of ui::diag.
#[allow(unsafe_code)]
pub mod kvaser;
#[allow(unsafe_code)]
pub mod pcan;
pub mod sim;
#[allow(unsafe_code)]
pub mod slcan;
#[allow(unsafe_code)]
pub mod sontheim;

use std::fmt;
use std::time::Duration;

/// Direction of a frame, seen from this tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Rx,
    Tx,
}

impl Dir {
    pub fn label(self) -> &'static str {
        match self {
            Dir::Rx => "Rx",
            Dir::Tx => "Tx",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u32,
    pub extended: bool,
    pub rtr: bool,
    pub dlc: u8,
    pub data: [u8; 8],
    /// Microseconds since the channel was opened (set by the bus worker).
    pub timestamp_us: u64,
    pub dir: Dir,
}

impl CanFrame {
    /// A standard (11 bit) data frame. Bytes beyond 8 are ignored.
    pub fn new(id: u32, data: &[u8]) -> Self {
        let mut d = [0u8; 8];
        let n = data.len().min(8);
        d[..n].copy_from_slice(&data[..n]);
        CanFrame {
            id,
            extended: false,
            rtr: false,
            dlc: n as u8,
            data: d,
            timestamp_us: 0,
            dir: Dir::Tx,
        }
    }

    /// The data bytes that the DLC covers (none for an RTR frame).
    pub fn payload(&self) -> &[u8] {
        if self.rtr {
            &[]
        } else {
            &self.data[..(self.dlc.min(8) as usize)]
        }
    }

    /// Check id range, DLC; used before a frame goes to an adapter.
    pub fn validate(&self) -> Result<(), CanError> {
        let max = if self.extended { 0x1FFF_FFFF } else { 0x7FF };
        if self.id > max {
            return Err(CanError::new(format!("CAN ID 0x{:X} out of range", self.id)));
        }
        if self.dlc > 8 {
            return Err(CanError::new("DLC above 8 (CAN FD is not supported)"));
        }
        Ok(())
    }

    /// The identity of the frame: the number and the frame format.
    pub fn can_id(&self) -> CanId {
        CanId::of(self)
    }
}

/// The identity of a raw CAN message: the number and the frame format.
/// A standard (11 bit) and an extended (29 bit) frame with the same number
/// are two different messages. The order puts every standard ID before
/// every extended ID, then the number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CanId {
    pub id: u32,
    pub extended: bool,
}

impl CanId {
    /// The largest extended (29 bit) ID.
    pub const MAX_EXT: u32 = 0x1FFF_FFFF;
    /// The largest standard (11 bit) ID.
    pub const MAX_STD: u32 = 0x7FF;

    pub fn new(id: u32, extended: bool) -> CanId {
        CanId { id, extended }
    }

    pub fn std(id: u32) -> CanId {
        CanId { id, extended: false }
    }

    pub fn ext(id: u32) -> CanId {
        CanId { id, extended: true }
    }

    pub fn of(f: &CanFrame) -> CanId {
        CanId { id: f.id, extended: f.extended }
    }

    /// Parse the text form of a text field or the CLI: hex digits with an
    /// optional `0x` prefix. A value above 7FF is extended; a trailing `x`
    /// or `X` forces extended (`123x`); 8 digits (the extended text form,
    /// `00000123`) are extended too. More than 29 bits is an error.
    pub fn parse(s: &str) -> Result<CanId, String> {
        let t = s.trim();
        let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
        let (digits, forced) = match t.strip_suffix(['x', 'X']) {
            Some(d) => (d, true),
            None => (t, false),
        };
        if digits.is_empty() {
            return Err("Enter a CAN ID in hex (for example 123, 18FEF100 or 123x)".to_string());
        }
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("'{}' is not a hex CAN ID (for example 123, 18FEF100 or 123x)", s.trim()));
        }
        let v = u64::from_str_radix(digits, 16).map_err(|_| format!("CAN ID '{}' has more than 29 bits (largest 1FFFFFFF)", s.trim()))?;
        if v > CanId::MAX_EXT as u64 {
            return Err(format!("CAN ID '{}' has more than 29 bits (largest 1FFFFFFF)", s.trim()));
        }
        let v = v as u32;
        Ok(CanId { id: v, extended: forced || v > CanId::MAX_STD || digits.len() == 8 })
    }

    /// Parse a text field that has its own "Ext" check box: the ID is
    /// extended when the text says so or the box is ticked.
    pub fn parse_with_flag(s: &str, extended: bool) -> Result<CanId, String> {
        let mut c = CanId::parse(s)?;
        c.extended |= extended;
        Ok(c)
    }
}

impl Ord for CanId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.extended, self.id).cmp(&(other.extended, other.id))
    }
}

impl PartialOrd for CanId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for CanId {
    /// Standard: 3 hex digits (`123`); extended: 8 hex digits (`18FEF100`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.extended {
            write!(f, "{:08X}", self.id)
        } else {
            write!(f, "{:03X}", self.id)
        }
    }
}

#[cfg(test)]
mod can_id_tests {
    use super::*;

    #[test]
    fn display_by_format() {
        assert_eq!(CanId::std(0x123).to_string(), "123");
        assert_eq!(CanId::std(0x7).to_string(), "007");
        assert_eq!(CanId::ext(0x18FE_F100).to_string(), "18FEF100");
        assert_eq!(CanId::ext(0x123).to_string(), "00000123");
    }

    #[test]
    fn parse_forms() {
        assert_eq!(CanId::parse("123"), Ok(CanId::std(0x123)));
        assert_eq!(CanId::parse(" 0x7ff "), Ok(CanId::std(0x7FF)));
        assert_eq!(CanId::parse("800"), Ok(CanId::ext(0x800)));
        assert_eq!(CanId::parse("18fef100"), Ok(CanId::ext(0x18FE_F100)));
        assert_eq!(CanId::parse("123x"), Ok(CanId::ext(0x123)));
        assert_eq!(CanId::parse("123X"), Ok(CanId::ext(0x123)));
        assert_eq!(CanId::parse("0x123x"), Ok(CanId::ext(0x123)));
        assert_eq!(CanId::parse("00000123"), Ok(CanId::ext(0x123)));
        assert_eq!(CanId::parse("1FFFFFFF"), Ok(CanId::ext(0x1FFF_FFFF)));
        // The text form reads back as the same ID.
        for c in [CanId::std(0), CanId::std(0x123), CanId::ext(0x123), CanId::ext(0x18FE_F100)] {
            assert_eq!(CanId::parse(&c.to_string()), Ok(c));
        }
    }

    #[test]
    fn parse_errors() {
        for bad in ["", "x", "0x", "12g", "-1", "1 2"] {
            assert!(CanId::parse(bad).is_err(), "{bad}");
        }
        let e = CanId::parse("20000000").unwrap_err();
        assert!(e.contains("29 bits"), "{e}");
        assert!(CanId::parse("123456789ABCDEF0123").unwrap_err().contains("29 bits"));
    }

    #[test]
    fn parse_with_box() {
        assert_eq!(CanId::parse_with_flag("123", true), Ok(CanId::ext(0x123)));
        assert_eq!(CanId::parse_with_flag("123", false), Ok(CanId::std(0x123)));
        assert_eq!(CanId::parse_with_flag("800", false), Ok(CanId::ext(0x800)));
    }

    #[test]
    fn standard_sorts_before_extended() {
        let mut v = vec![CanId::ext(0x1), CanId::std(0x7FF), CanId::ext(0x123), CanId::std(0x123), CanId::std(0x1)];
        v.sort();
        assert_eq!(v, vec![CanId::std(0x1), CanId::std(0x123), CanId::std(0x7FF), CanId::ext(0x1), CanId::ext(0x123)]);
        assert_ne!(CanId::std(0x123), CanId::ext(0x123));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanError(pub String);

impl CanError {
    pub fn new(s: impl Into<String>) -> Self {
        CanError(s.into())
    }
}

impl fmt::Display for CanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CanError {}

/// The bitrates offered in the UI (CiA 301 standard set except 100k).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bitrate {
    K10,
    K20,
    K50,
    K125,
    K250,
    K500,
    K800,
    M1,
}

impl Bitrate {
    pub const ALL: [Bitrate; 8] = [
        Bitrate::K10,
        Bitrate::K20,
        Bitrate::K50,
        Bitrate::K125,
        Bitrate::K250,
        Bitrate::K500,
        Bitrate::K800,
        Bitrate::M1,
    ];

    pub fn kbit(self) -> u32 {
        match self {
            Bitrate::K10 => 10,
            Bitrate::K20 => 20,
            Bitrate::K50 => 50,
            Bitrate::K125 => 125,
            Bitrate::K250 => 250,
            Bitrate::K500 => 500,
            Bitrate::K800 => 800,
            Bitrate::M1 => 1000,
        }
    }

    pub fn from_kbit(k: u32) -> Option<Bitrate> {
        Bitrate::ALL.iter().copied().find(|b| b.kbit() == k)
    }

    pub fn label(self) -> String {
        if self.kbit() >= 1000 {
            "1 Mbit/s".to_string()
        } else {
            format!("{} kbit/s", self.kbit())
        }
    }
}

/// A channel that an adapter offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelInfo {
    /// Adapter specific handle (PCAN: the TPCANHandle; CANfox: the net number).
    pub handle: u32,
    pub name: String,
    pub available: bool,
}

/// A CAN interface. Implementations must not send anything on their own:
/// `open` only opens the channel.
pub trait CanAdapter: Send {
    fn name(&self) -> String;
    fn list_channels(&mut self) -> Result<Vec<ChannelInfo>, CanError>;
    fn open(&mut self, channel: u32, bitrate: Bitrate, listen_only: bool) -> Result<(), CanError>;
    fn close(&mut self);
    fn send(&mut self, frame: &CanFrame) -> Result<(), CanError>;
    /// Wait at most `timeout` for one frame. `Ok(None)` when none came.
    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, CanError>;
    /// Bus status as short text ("OK", "Bus off", ...).
    fn status(&mut self) -> String;
    /// Access to simulator controls; `None` for real hardware.
    fn as_sim(&mut self) -> Option<&mut sim::SimAdapter> {
        None
    }
    /// The adapter sets the frame time stamps itself (a log replay); the
    /// driver keeps them instead of its own clock.
    fn keeps_timestamps(&self) -> bool {
        false
    }
    /// No frame will come any more (the end of a log).
    fn finished(&self) -> bool {
        false
    }
}
