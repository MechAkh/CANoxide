//! PEAK PCAN-Basic backend. PCANBasic.dll is loaded at run time with
//! `libloading`; without the DLL the rest of the tool works.
//!
//! The constants and structures follow the PCAN-Basic API (PCANBasic.h).
//! No header was found on the build PC; the values are from knowledge of the
//! published API and are checked by the layout tests below.
//!
//! This is the only module with `unsafe` code.

use super::{Bitrate, CanAdapter, CanError, CanFrame, ChannelInfo, Dir};
use std::ffi::c_void;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub type TPCANHandle = u16;
pub type TPCANStatus = u32;
pub type TPCANBaudrate = u16;

// Channel handles: PCAN-USB interfaces 1..8.
pub const PCAN_USBBUS1: TPCANHandle = 0x51;
pub const PCAN_USB_CHANNELS: [TPCANHandle; 8] = [0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58];

// Status / error codes.
pub const PCAN_ERROR_OK: TPCANStatus = 0x00000;
pub const PCAN_ERROR_XMTFULL: TPCANStatus = 0x00001;
pub const PCAN_ERROR_OVERRUN: TPCANStatus = 0x00002;
pub const PCAN_ERROR_BUSLIGHT: TPCANStatus = 0x00004;
pub const PCAN_ERROR_BUSHEAVY: TPCANStatus = 0x00008;
pub const PCAN_ERROR_BUSOFF: TPCANStatus = 0x00010;
pub const PCAN_ERROR_QRCVEMPTY: TPCANStatus = 0x00020;
pub const PCAN_ERROR_QOVERRUN: TPCANStatus = 0x00040;
pub const PCAN_ERROR_QXMTFULL: TPCANStatus = 0x00080;
pub const PCAN_ERROR_REGTEST: TPCANStatus = 0x00100;
pub const PCAN_ERROR_NODRIVER: TPCANStatus = 0x00200;
pub const PCAN_ERROR_HWINUSE: TPCANStatus = 0x00400;
pub const PCAN_ERROR_NETINUSE: TPCANStatus = 0x00800;
pub const PCAN_ERROR_ILLHW: TPCANStatus = 0x01400;
pub const PCAN_ERROR_ILLNET: TPCANStatus = 0x01800;
pub const PCAN_ERROR_ILLCLIENT: TPCANStatus = 0x01C00;
pub const PCAN_ERROR_RESOURCE: TPCANStatus = 0x02000;
pub const PCAN_ERROR_ILLPARAMTYPE: TPCANStatus = 0x04000;
pub const PCAN_ERROR_ILLPARAMVAL: TPCANStatus = 0x08000;
pub const PCAN_ERROR_UNKNOWN: TPCANStatus = 0x10000;
pub const PCAN_ERROR_ILLDATA: TPCANStatus = 0x20000;
pub const PCAN_ERROR_BUSPASSIVE: TPCANStatus = 0x40000;
pub const PCAN_ERROR_ILLMODE: TPCANStatus = 0x80000;
pub const PCAN_ERROR_CAUTION: TPCANStatus = 0x2000000;
pub const PCAN_ERROR_INITIALIZE: TPCANStatus = 0x4000000;
pub const PCAN_ERROR_ILLOPERATION: TPCANStatus = 0x8000000;

// Parameters.
pub const PCAN_LISTEN_ONLY: u8 = 0x08;
pub const PCAN_CHANNEL_CONDITION: u8 = 0x0D;
pub const PCAN_PARAMETER_OFF: u32 = 0x00;
pub const PCAN_PARAMETER_ON: u32 = 0x01;
pub const PCAN_CHANNEL_UNAVAILABLE: u32 = 0x00;
pub const PCAN_CHANNEL_AVAILABLE: u32 = 0x01;
pub const PCAN_CHANNEL_OCCUPIED: u32 = 0x02;

// Message types.
pub const PCAN_MESSAGE_STANDARD: u8 = 0x00;
pub const PCAN_MESSAGE_RTR: u8 = 0x01;
pub const PCAN_MESSAGE_EXTENDED: u8 = 0x02;
pub const PCAN_MESSAGE_FD: u8 = 0x04;
pub const PCAN_MESSAGE_ERRFRAME: u8 = 0x40;
pub const PCAN_MESSAGE_STATUS: u8 = 0x80;

/// BTR0/BTR1 codes of the PCAN-Basic API for the supported bitrates.
pub fn baudrate_code(b: Bitrate) -> TPCANBaudrate {
    match b {
        Bitrate::M1 => 0x0014,
        Bitrate::K800 => 0x0016,
        Bitrate::K500 => 0x001C,
        Bitrate::K250 => 0x011C,
        Bitrate::K125 => 0x031C,
        Bitrate::K50 => 0x472F,
        Bitrate::K20 => 0x532F,
        Bitrate::K10 => 0x672F,
    }
}

/// TPCANMsg of PCANBasic.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TPCANMsg {
    pub id: u32,
    pub msgtype: u8,
    pub len: u8,
    pub data: [u8; 8],
}

/// TPCANTimestamp of PCANBasic.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TPCANTimestamp {
    pub millis: u32,
    pub millis_overflow: u16,
    pub micros: u16,
}

pub fn channel_name(h: TPCANHandle) -> String {
    if (PCAN_USBBUS1..=0x58).contains(&h) {
        format!("PCAN_USBBUS{}", h - PCAN_USBBUS1 + 1)
    } else {
        format!("PCAN 0x{h:X}")
    }
}

/// Text for a status value (bit field for the bus states).
pub fn status_text(s: TPCANStatus) -> String {
    if s == PCAN_ERROR_OK {
        return "OK".to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    let flags: [(TPCANStatus, &str); 9] = [
        (PCAN_ERROR_BUSOFF, "Bus off"),
        (PCAN_ERROR_BUSPASSIVE, "Bus passive"),
        (PCAN_ERROR_BUSHEAVY, "Bus heavy (warning)"),
        (PCAN_ERROR_BUSLIGHT, "Bus light"),
        (PCAN_ERROR_OVERRUN, "Controller overrun"),
        (PCAN_ERROR_QOVERRUN, "Receive queue overrun"),
        (PCAN_ERROR_XMTFULL, "Transmit buffer full"),
        (PCAN_ERROR_QXMTFULL, "Transmit queue full"),
        (PCAN_ERROR_QRCVEMPTY, "Receive queue empty"),
    ];
    let mut rest = s;
    for (bit, text) in flags {
        if s & bit != 0 {
            parts.push(text);
            rest &= !bit;
        }
    }
    let other = match rest {
        0 => None,
        PCAN_ERROR_NODRIVER => Some("Driver not loaded".to_string()),
        PCAN_ERROR_HWINUSE => Some("Hardware in use by another program".to_string()),
        PCAN_ERROR_NETINUSE => Some("Net in use".to_string()),
        PCAN_ERROR_ILLHW => Some("Invalid hardware handle".to_string()),
        PCAN_ERROR_ILLNET => Some("Invalid net handle".to_string()),
        PCAN_ERROR_ILLCLIENT => Some("Invalid client handle".to_string()),
        PCAN_ERROR_RESOURCE => Some("Resource could not be created".to_string()),
        PCAN_ERROR_ILLPARAMTYPE => Some("Invalid parameter".to_string()),
        PCAN_ERROR_ILLPARAMVAL => Some("Invalid parameter value".to_string()),
        PCAN_ERROR_ILLDATA => Some("Invalid data".to_string()),
        PCAN_ERROR_ILLMODE => Some("Invalid mode".to_string()),
        PCAN_ERROR_INITIALIZE => Some("Channel not initialized".to_string()),
        PCAN_ERROR_ILLOPERATION => Some("Invalid operation".to_string()),
        PCAN_ERROR_CAUTION => Some("Caution".to_string()),
        PCAN_ERROR_REGTEST => Some("Register test failed".to_string()),
        PCAN_ERROR_UNKNOWN => Some("Unknown error".to_string()),
        x => Some(format!("PCAN error 0x{x:X}")),
    };
    let mut out = parts.join(", ");
    if let Some(o) = other {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&o);
    }
    out
}

type FnInitialize = unsafe extern "system" fn(TPCANHandle, TPCANBaudrate, u8, u32, u16) -> TPCANStatus;
type FnUninitialize = unsafe extern "system" fn(TPCANHandle) -> TPCANStatus;
type FnGetStatus = unsafe extern "system" fn(TPCANHandle) -> TPCANStatus;
type FnRead = unsafe extern "system" fn(TPCANHandle, *mut TPCANMsg, *mut TPCANTimestamp) -> TPCANStatus;
type FnWrite = unsafe extern "system" fn(TPCANHandle, *mut TPCANMsg) -> TPCANStatus;
type FnGetValue = unsafe extern "system" fn(TPCANHandle, u8, *mut c_void, u32) -> TPCANStatus;
type FnSetValue = unsafe extern "system" fn(TPCANHandle, u8, *mut c_void, u32) -> TPCANStatus;
type FnGetErrorText = unsafe extern "system" fn(TPCANStatus, u16, *mut u8) -> TPCANStatus;

struct Api {
    // The function pointers stay valid while `_lib` is alive; both live in
    // this struct and are dropped together.
    initialize: FnInitialize,
    uninitialize: FnUninitialize,
    get_status: FnGetStatus,
    read: FnRead,
    write: FnWrite,
    get_value: FnGetValue,
    set_value: FnSetValue,
    get_error_text: FnGetErrorText,
    _lib: libloading::Library,
}

/// Paths tried for the DLL: the Windows system folder first, then the
/// folder of the exe (a portable copy next to the tool).
fn dll_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(root) = std::env::var_os("SystemRoot") {
        v.push(PathBuf::from(root).join("System32").join("PCANBasic.dll"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("PCANBasic.dll"));
        }
    }
    v
}

impl Api {
    fn load() -> Result<Api, CanError> {
        let mut last = String::from("PCANBasic.dll not found");
        for path in dll_candidates() {
            if !path.exists() {
                continue;
            }
            // SAFETY: loading a DLL runs its DllMain; PCANBasic.dll is the
            // PEAK driver library from a fixed path, not a search path.
            let lib = match unsafe { libloading::Library::new(&path) } {
                Ok(l) => l,
                Err(e) => {
                    last = format!("PCANBasic.dll could not be loaded: {e}");
                    continue;
                }
            };
            // SAFETY: the symbol types match the PCAN-Basic prototypes
            // (WINAPI = "system" calling convention).
            let api = unsafe {
                let initialize = *lib
                    .get::<FnInitialize>(b"CAN_Initialize\0")
                    .map_err(|e| CanError::new(format!("CAN_Initialize: {e}")))?;
                let uninitialize = *lib
                    .get::<FnUninitialize>(b"CAN_Uninitialize\0")
                    .map_err(|e| CanError::new(format!("CAN_Uninitialize: {e}")))?;
                let get_status = *lib
                    .get::<FnGetStatus>(b"CAN_GetStatus\0")
                    .map_err(|e| CanError::new(format!("CAN_GetStatus: {e}")))?;
                let read = *lib
                    .get::<FnRead>(b"CAN_Read\0")
                    .map_err(|e| CanError::new(format!("CAN_Read: {e}")))?;
                let write = *lib
                    .get::<FnWrite>(b"CAN_Write\0")
                    .map_err(|e| CanError::new(format!("CAN_Write: {e}")))?;
                let get_value = *lib
                    .get::<FnGetValue>(b"CAN_GetValue\0")
                    .map_err(|e| CanError::new(format!("CAN_GetValue: {e}")))?;
                let set_value = *lib
                    .get::<FnSetValue>(b"CAN_SetValue\0")
                    .map_err(|e| CanError::new(format!("CAN_SetValue: {e}")))?;
                let get_error_text = *lib
                    .get::<FnGetErrorText>(b"CAN_GetErrorText\0")
                    .map_err(|e| CanError::new(format!("CAN_GetErrorText: {e}")))?;
                Api {
                    initialize,
                    uninitialize,
                    get_status,
                    read,
                    write,
                    get_value,
                    set_value,
                    get_error_text,
                    _lib: lib,
                }
            };
            return Ok(api);
        }
        Err(CanError::new(format!("driver not found ({last})")))
    }

    fn error_text(&self, s: TPCANStatus) -> String {
        let mut buf = [0u8; 256];
        // SAFETY: the API writes at most 256 bytes including the terminator.
        let r = unsafe { (self.get_error_text)(s, 0x09, buf.as_mut_ptr()) };
        if r == PCAN_ERROR_OK {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let t = String::from_utf8_lossy(&buf[..end]).trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
        status_text(s)
    }

    fn get_u32(&self, h: TPCANHandle, param: u8) -> Result<u32, TPCANStatus> {
        let mut v: u32 = 0;
        // SAFETY: buffer of 4 bytes, length passed.
        let r = unsafe { (self.get_value)(h, param, (&mut v as *mut u32).cast(), 4) };
        if r == PCAN_ERROR_OK {
            Ok(v)
        } else {
            Err(r)
        }
    }

    fn set_u32(&self, h: TPCANHandle, param: u8, mut v: u32) -> Result<(), TPCANStatus> {
        // SAFETY: buffer of 4 bytes, length passed.
        let r = unsafe { (self.set_value)(h, param, (&mut v as *mut u32).cast(), 4) };
        if r == PCAN_ERROR_OK {
            Ok(())
        } else {
            Err(r)
        }
    }
}

pub struct PcanAdapter {
    api: Api,
    open: Option<TPCANHandle>,
    listen_only: bool,
}

impl PcanAdapter {
    /// Load PCANBasic.dll. Error text starts with "driver not found" when
    /// the DLL is missing.
    pub fn load() -> Result<PcanAdapter, CanError> {
        Ok(PcanAdapter {
            api: Api::load()?,
            open: None,
            listen_only: false,
        })
    }
}

impl CanAdapter for PcanAdapter {
    fn name(&self) -> String {
        "PEAK PCAN".to_string()
    }

    fn list_channels(&mut self) -> Result<Vec<ChannelInfo>, CanError> {
        let mut v = Vec::new();
        for h in PCAN_USB_CHANNELS {
            if let Ok(cond) = self.api.get_u32(h, PCAN_CHANNEL_CONDITION) {
                if cond & PCAN_CHANNEL_AVAILABLE != 0 {
                    let mut name = channel_name(h);
                    if cond & PCAN_CHANNEL_OCCUPIED != 0 {
                        name.push_str(" (occupied)");
                    }
                    v.push(ChannelInfo { handle: h as u32, name, available: true });
                }
            }
        }
        Ok(v)
    }

    fn open(&mut self, channel: u32, bitrate: Bitrate, listen_only: bool) -> Result<(), CanError> {
        self.close();
        let h = u16::try_from(channel).map_err(|_| CanError::new("invalid channel"))?;
        // Listen-only must be set before the initialization.
        let lo = if listen_only { PCAN_PARAMETER_ON } else { PCAN_PARAMETER_OFF };
        if let Err(e) = self.api.set_u32(h, PCAN_LISTEN_ONLY, lo) {
            if listen_only {
                return Err(CanError::new(format!(
                    "{}: listen-only mode could not be set: {}",
                    channel_name(h),
                    self.api.error_text(e)
                )));
            }
        }
        // SAFETY: plug-and-play hardware: type, port and interrupt are 0.
        let r = unsafe { (self.api.initialize)(h, baudrate_code(bitrate), 0, 0, 0) };
        if r != PCAN_ERROR_OK {
            return Err(CanError::new(format!("{}: {}", channel_name(h), self.api.error_text(r))));
        }
        if listen_only {
            // Check that the mode is really active; else close again.
            match self.api.get_u32(h, PCAN_LISTEN_ONLY) {
                Ok(PCAN_PARAMETER_ON) => {}
                _ => {
                    // SAFETY: channel was initialized above.
                    unsafe { (self.api.uninitialize)(h) };
                    return Err(CanError::new(format!(
                        "{}: listen-only mode is not active, channel closed",
                        channel_name(h)
                    )));
                }
            }
        }
        self.open = Some(h);
        self.listen_only = listen_only;
        Ok(())
    }

    fn close(&mut self) {
        if let Some(h) = self.open.take() {
            // SAFETY: handle was initialized by `open`.
            unsafe { (self.api.uninitialize)(h) };
        }
    }

    fn send(&mut self, frame: &CanFrame) -> Result<(), CanError> {
        let h = self.open.ok_or_else(|| CanError::new("channel not open"))?;
        if self.listen_only {
            return Err(CanError::new("listen-only mode: sending is disabled"));
        }
        frame.validate()?;
        let mut msg = TPCANMsg {
            id: frame.id,
            msgtype: (if frame.extended { PCAN_MESSAGE_EXTENDED } else { PCAN_MESSAGE_STANDARD })
                | (if frame.rtr { PCAN_MESSAGE_RTR } else { 0 }),
            len: frame.dlc,
            data: frame.data,
        };
        // SAFETY: msg is a valid TPCANMsg.
        let r = unsafe { (self.api.write)(h, &mut msg) };
        if r == PCAN_ERROR_OK {
            Ok(())
        } else {
            Err(CanError::new(self.api.error_text(r)))
        }
    }

    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, CanError> {
        let h = self.open.ok_or_else(|| CanError::new("channel not open"))?;
        let end = Instant::now() + timeout;
        loop {
            let mut msg = TPCANMsg::default();
            let mut ts = TPCANTimestamp::default();
            // SAFETY: valid out pointers.
            let r = unsafe { (self.api.read)(h, &mut msg, &mut ts) };
            if r == PCAN_ERROR_OK {
                if msg.msgtype & (PCAN_MESSAGE_STATUS | PCAN_MESSAGE_ERRFRAME | PCAN_MESSAGE_FD) != 0 {
                    continue;
                }
                return Ok(Some(CanFrame {
                    id: msg.id,
                    extended: msg.msgtype & PCAN_MESSAGE_EXTENDED != 0,
                    rtr: msg.msgtype & PCAN_MESSAGE_RTR != 0,
                    dlc: msg.len.min(8),
                    data: msg.data,
                    timestamp_us: 0,
                    dir: Dir::Rx,
                }));
            }
            if r & PCAN_ERROR_QRCVEMPTY == 0 {
                // Bus state changes come with the read; report them as status, not as error.
                if r & (PCAN_ERROR_BUSLIGHT | PCAN_ERROR_BUSHEAVY | PCAN_ERROR_BUSOFF | PCAN_ERROR_BUSPASSIVE | PCAN_ERROR_OVERRUN | PCAN_ERROR_QOVERRUN) == 0 {
                    return Err(CanError::new(self.api.error_text(r)));
                }
            }
            if Instant::now() >= end {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn status(&mut self) -> String {
        match self.open {
            None => "closed".to_string(),
            // SAFETY: handle was initialized by `open`.
            Some(h) => status_text(unsafe { (self.api.get_status)(h) }),
        }
    }
}

impl Drop for PcanAdapter {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn struct_layout_matches_pcanbasic_h() {
        assert_eq!(size_of::<TPCANMsg>(), 16);
        assert_eq!(offset_of!(TPCANMsg, id), 0);
        assert_eq!(offset_of!(TPCANMsg, msgtype), 4);
        assert_eq!(offset_of!(TPCANMsg, len), 5);
        assert_eq!(offset_of!(TPCANMsg, data), 6);
        assert_eq!(size_of::<TPCANTimestamp>(), 8);
        assert_eq!(offset_of!(TPCANTimestamp, millis_overflow), 4);
        assert_eq!(offset_of!(TPCANTimestamp, micros), 6);
    }

    #[test]
    fn bitrate_table() {
        let expect = [
            (Bitrate::K10, 0x672F),
            (Bitrate::K20, 0x532F),
            (Bitrate::K50, 0x472F),
            (Bitrate::K125, 0x031C),
            (Bitrate::K250, 0x011C),
            (Bitrate::K500, 0x001C),
            (Bitrate::K800, 0x0016),
            (Bitrate::M1, 0x0014),
        ];
        for (b, code) in expect {
            assert_eq!(baudrate_code(b), code, "{b:?}");
        }
        assert_eq!(Bitrate::ALL.len(), 8);
    }

    #[test]
    fn status_texts() {
        assert_eq!(status_text(PCAN_ERROR_OK), "OK");
        assert_eq!(status_text(PCAN_ERROR_BUSOFF), "Bus off");
        assert!(status_text(PCAN_ERROR_BUSHEAVY).contains("heavy"));
        assert!(status_text(PCAN_ERROR_OVERRUN | PCAN_ERROR_BUSLIGHT).contains("overrun"));
        assert_eq!(status_text(PCAN_ERROR_NODRIVER), "Driver not loaded");
        assert_eq!(channel_name(0x51), "PCAN_USBBUS1");
        assert_eq!(channel_name(0x58), "PCAN_USBBUS8");
    }
}
