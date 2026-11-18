//! CAN frame model and the adapter trait. Classic CAN only (no CAN FD).

// The PCAN FFI module is the only place with `unsafe` code.
#[allow(unsafe_code)]
pub mod pcan;
pub mod sim;

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
    /// Adapter specific handle (PCAN: the TPCANHandle).
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
}
