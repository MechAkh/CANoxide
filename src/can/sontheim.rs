//! ifm CANfox backend through the Sontheim multithread CAN API (MT-API).
//! `SIECA164.DLL` (64 bit) is loaded at run time with `libloading`; without
//! the DLL the rest of the tool works. `SIECA132.DLL` is the 32-bit API and
//! is never loaded.
//!
//! The declarations below are written by hand from the API manual
//! ("Software description SIECA132", version 3.0.9) and checked field by
//! field against the installed header; the layout tests guard them. No
//! Sontheim file is part of this repository.
//!
//! Hang protection: the API can hang inside a call (seen after a failed
//! CANfox firmware upgrade). Every call into the DLL, the load included,
//! runs on one worker thread that owns the library and the handle. The
//! adapter waits for each answer with a time-out; when one does not come,
//! the adapter (and every later Sontheim adapter of this process) is dead
//! and returns `DEAD_TEXT` at once. The stuck thread is left alone.
//!
//! Nothing is sent by `open`: canOpen, canSetBaudrate, canSetFilterMode and
//! canEnableAllIds only. canSetBaudrateForce, the owner calls, the EEPROM
//! and flash calls are never used.

use super::{Bitrate, CanAdapter, CanError, CanFrame, ChannelInfo, Dir};
use std::collections::VecDeque;
use std::ffi::{c_char, c_void, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// The net number of a CANfox (manual p. 36).
pub const CANFOX_NET: u32 = 105;

/// The 64-bit API library.
pub const DLL_NAME: &str = "SIECA164.DLL";

/// The environment variable with a full path of the DLL (tests on other PCs).
pub const DLL_ENV: &str = "CANOXIDE_SIECA_DLL";

pub const NOT_FOUND_TEXT: &str = "Sontheim CAN driver (SIECA164.DLL) not found. Install the CANfox driver.";

pub const DEAD_TEXT: &str = "The Sontheim CAN driver does not answer. Close CoDeSys and restart the PC, then try again.";

// NTCAN return codes (header SIECA132.h, manual p. 31-32).
pub const NTCAN_SUCCESS: i32 = 0;
pub const NTCAN_RX_TIMEOUT: i32 = -1;
pub const NTCAN_TX_TIMEOUT: i32 = -2;
pub const NTCAN_CONTR_BUSOFF: i32 = -3;
pub const NTCAN_NO_ID_ENABLED: i32 = -4;
pub const NTCAN_ID_ALREADY_ENABLED: i32 = -5;
pub const NTCAN_ID_NOT_ENABLED: i32 = -6;
pub const NTCAN_INVALID_PARAMETER: i32 = -7;
pub const NTCAN_INVALID_HANDLE: i32 = -8;
pub const NTCAN_TOO_MANY_HANDLES: i32 = -9;
pub const NTCAN_INIT_ERROR: i32 = -10;
pub const NTCAN_RESET_ERROR: i32 = -11;
pub const NTCAN_DRIVER_ERROR: i32 = -12;
pub const NTCAN_DLL_ALREADY_INIT: i32 = -13;
pub const NTCAN_CHANNEL_NOT_INITIALIZED: i32 = -14;
pub const NTCAN_TX_ERROR: i32 = -15;
pub const NTCAN_NO_SHAREDMEMORY: i32 = -16;
pub const NTCAN_HARDWARE_NOT_FOUND: i32 = -17;
pub const NTCAN_INVALID_NETNUMBER: i32 = -18;
pub const NTCAN_DRIVER_NOT_INSTALLED: i32 = -21;
pub const NTCAN_NO_OWNER_RIGHTS: i32 = -22;
pub const NTCAN_FIRMWARE_TOO_OLD: i32 = -23;
pub const NTCAN_FIRMWARE_UNSUPPORTED: i32 = -24;
pub const NTCAN_FIRMWAREUPDATE_FAILED: i32 = -25;
pub const NTCAN_HARDWARE_NOT_SUPPORTED: i32 = -26;
pub const NTCAN_FILE_NOT_FOUND: i32 = -27;
pub const NTCAN_GET_MUTEX_FAILED: i32 = -104;
pub const NTCAN_NET_NOT_AVAILABLE: i32 = -106;
pub const NTCAN_HARDWARE_IN_USE: i32 = -110;
pub const NTCAN_API_NOT_RUNNING: i32 = -111;
pub const NTCAN_CHANNEL_CURR_NOT_AVAILABLE: i32 = -112;
pub const NTCAN_BUFFER_TOO_SMALL: i32 = -113;
pub const NTCAN_HARDWARENOTACTIVE: i32 = -200;
pub const NTCAN_TOO_MANY_APPLICATIONS: i32 = -201;
pub const NTCAN_FLUSH_TIMEOUT: i32 = -202;
pub const NTCAN_NOT_POSSIBLE: i32 = -300;
pub const NTCAN_NOSUCCESS: i32 = 0xFFFF_0000_u32 as i32;

/// OR-ed into the baud code: the controller only listens (manual p. 41).
pub const BAUD_SILENTMODE: i32 = 0x4000_0000;

// by_extended bits of CMSG (manual p. 19).
pub const MSG_STANDARD: u8 = 0x01;
pub const MSG_EXTENDED: u8 = 0x02;
pub const MSG_ERRORFRAME: u8 = 0x40;
pub const MSG_ECHO: u8 = 0x80;

/// T_FILTER_MODE filterMode_nofilter (manual p. 64 and 130).
pub const FILTER_MODE_NOFILTER: i32 = 4;

// w_errorflag bits of CAN_IF_STATUS (manual p. 20-21).
pub const ERR_RBUFOVERFLOW: u16 = 0x0001;
pub const ERR_HWRBUFOVERFLOW: u16 = 0x0002;
pub const ERR_BUSOFF: u16 = 0x0004;
pub const ERR_ERRORPASSIVE: u16 = 0x0008;
pub const ERR_TXERROR: u16 = 0x0010;
pub const ERR_USBDATAERROR: u16 = 0x0020;
pub const ERR_USBRESTART: u16 = 0x0040;
pub const ERR_HWUNPLUGGED: u16 = 0x0080;
pub const ERR_BRIDGEERROR: u16 = 0x0100;
pub const ERR_FWRESET: u16 = 0x0200;

/// Frames read with one canReadNoWait call.
const READ_BATCH: usize = 64;
/// Entries of the device list buffer (more than any PC has).
const DEVICE_LIST_LEN: usize = 64;

/// Baud code of canSetBaudrate (manual p. 41-42).
pub fn baud_code(b: Bitrate) -> i32 {
    match b {
        Bitrate::M1 => 0,
        Bitrate::K800 => 1,
        Bitrate::K500 => 2,
        Bitrate::K250 => 3,
        Bitrate::K125 => 4,
        Bitrate::K50 => 6,
        Bitrate::K20 => 7,
        Bitrate::K10 => 8,
    }
}

/// The baud value passed to canSetBaudrate.
pub fn baud_value(b: Bitrate, listen_only: bool) -> i32 {
    baud_code(b) | if listen_only { BAUD_SILENTMODE } else { 0 }
}

/// CMSG of the header (`#pragma pack(8)`, 20 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cmsg {
    pub l_id: i32,
    pub by_len: u8,
    pub by_msg_lost: u8,
    pub by_extended: u8,
    pub by_remote: u8,
    pub aby_data: [u8; 8],
    pub ul_tstamp: u32,
}

/// CAN_IF_STATUS of the header (32 bytes). The manual shows the baud field
/// as a 16-bit `w_baud`, the header as a `long l_baud`; the header layout is
/// used (it is the larger one) and the baud field is not read.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CanIfStatus {
    pub w_hw_rev: u16,
    pub w_fw_rev: u16,
    pub w_drv_rev: u16,
    pub w_dll_rev: u16,
    pub ul_board_status: u32,
    pub by_board_id: [u8; 2],
    pub w_busoffctr: u16,
    pub w_errorflag: u16,
    pub w_errorframectr: u16,
    pub w_netctr: u16,
    pub l_baud: i32,
    pub ui_epld_rev: u32,
}

/// T_DeviceList of the header (104 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceList {
    pub net: i32,
    pub name: [u8; 20],
    pub ul_status: u32,
    pub ul_features: u32,
    pub reserved: [i32; 18],
}

/// A connected device as the API lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub net: i32,
    pub name: String,
}

/// Plain English text of an NTCAN code.
pub fn error_text(code: i32) -> String {
    let t = match code {
        NTCAN_SUCCESS => "No error.",
        NTCAN_RX_TIMEOUT => "No frame received in time.",
        NTCAN_TX_TIMEOUT => "The frame could not be sent in time (no other node acknowledges, or the bus is busy).",
        NTCAN_CONTR_BUSOFF => "The CAN controller is bus off.",
        NTCAN_NO_ID_ENABLED => "No CAN identifier is enabled for reception.",
        NTCAN_ID_ALREADY_ENABLED => "The CAN identifier is already enabled.",
        NTCAN_ID_NOT_ENABLED => "The CAN identifier is not enabled.",
        NTCAN_INVALID_PARAMETER => "The driver refused a parameter.",
        NTCAN_INVALID_HANDLE => "The net handle is not valid (the net is not open).",
        NTCAN_TOO_MANY_HANDLES => "Too many handles are open on this net.",
        NTCAN_INIT_ERROR => "The net could not be initialised.",
        NTCAN_RESET_ERROR => "The CAN controller could not be reset.",
        NTCAN_DRIVER_ERROR => "The driver reported an internal error.",
        NTCAN_DLL_ALREADY_INIT => "The bitrate of this net was already set by another program (for example CoDeSys); this program has no right to change it.",
        NTCAN_CHANNEL_NOT_INITIALIZED => "The net is not initialised (no bitrate set).",
        NTCAN_TX_ERROR => "The frame could not be sent.",
        NTCAN_NO_SHAREDMEMORY => "The driver has no shared memory.",
        NTCAN_HARDWARE_NOT_FOUND => "No CANfox found on this net. Check the USB cable and the net number.",
        NTCAN_INVALID_NETNUMBER => "The net number is not valid.",
        NTCAN_DRIVER_NOT_INSTALLED => "The Sontheim driver is not installed.",
        NTCAN_NO_OWNER_RIGHTS => "Another program (for example CoDeSys) owns this net; this program may not change its bitrate.",
        NTCAN_FIRMWARE_TOO_OLD => "The CANfox firmware is too old for this driver.",
        NTCAN_FIRMWARE_UNSUPPORTED => "The CANfox firmware is not supported by this driver.",
        NTCAN_FIRMWAREUPDATE_FAILED => "The CANfox firmware update failed. Restart the PC and try again.",
        NTCAN_HARDWARE_NOT_SUPPORTED => "This hardware is not supported by the driver.",
        NTCAN_FILE_NOT_FOUND => "A driver file was not found.",
        NTCAN_GET_MUTEX_FAILED => "The driver is busy (mutex not available).",
        NTCAN_NET_NOT_AVAILABLE => "The net is not available.",
        NTCAN_HARDWARE_IN_USE => "The CANfox is in use by another program.",
        NTCAN_API_NOT_RUNNING => "The Sontheim service (SIECE132Svr) is not running.",
        NTCAN_CHANNEL_CURR_NOT_AVAILABLE => "The net is not available at the moment.",
        NTCAN_BUFFER_TOO_SMALL => "The driver buffer is too small.",
        NTCAN_HARDWARENOTACTIVE => "The CANfox is not active.",
        NTCAN_TOO_MANY_APPLICATIONS => "Too many programs use the driver.",
        NTCAN_FLUSH_TIMEOUT => "The driver could not flush its buffer in time.",
        NTCAN_NOT_POSSIBLE => "The driver cannot do this here.",
        NTCAN_NOSUCCESS => "The driver reported no success.",
        x => return format!("Sontheim driver error {x}."),
    };
    t.to_string()
}

/// Short status text from the w_errorflag bits; `lost` adds the loss of
/// frames seen since the net was opened.
pub fn status_text(errorflag: u16, lost: bool) -> String {
    let flags: [(u16, &str); 9] = [
        (ERR_HWUNPLUGGED, "Device unplugged"),
        (ERR_BUSOFF, "Bus off"),
        (ERR_ERRORPASSIVE, "Error passive"),
        (ERR_TXERROR, "Transmit error"),
        (ERR_RBUFOVERFLOW, "Receive buffer overflow"),
        (ERR_HWRBUFOVERFLOW, "Hardware receive overflow"),
        (ERR_USBDATAERROR, "USB data error"),
        (ERR_USBRESTART, "USB restart"),
        (ERR_FWRESET, "Firmware reset"),
    ];
    let mut parts: Vec<&str> = flags.iter().filter(|(b, _)| errorflag & b != 0).map(|(_, t)| *t).collect();
    if errorflag & ERR_BRIDGEERROR != 0 {
        parts.push("Bridge error");
    }
    if parts.is_empty() {
        parts.push("OK");
    }
    let mut s = parts.join(", ");
    if lost {
        s.push_str(", frames lost");
    }
    s
}

/// A frame as CMSG for canSend (call `validate` first).
pub fn frame_to_cmsg(f: &CanFrame) -> Cmsg {
    Cmsg {
        l_id: f.id as i32,
        by_len: f.dlc.min(8),
        by_msg_lost: 0,
        by_extended: if f.extended { MSG_EXTENDED } else { 0 },
        by_remote: u8::from(f.rtr),
        aby_data: f.data,
        ul_tstamp: 0,
    }
}

/// A received CMSG as frame; `None` for an error frame or an echo.
pub fn cmsg_to_frame(m: &Cmsg) -> Option<CanFrame> {
    if m.by_extended & (MSG_ERRORFRAME | MSG_ECHO) != 0 {
        return None;
    }
    let extended = m.by_extended & MSG_EXTENDED != 0;
    let mask = if extended { 0x1FFF_FFFF } else { 0x7FF };
    Some(CanFrame {
        id: (m.l_id as u32) & mask,
        extended,
        rtr: m.by_remote != 0,
        dlc: (m.by_len & 0x0F).min(8),
        data: m.aby_data,
        timestamp_us: 0,
        dir: Dir::Rx,
    })
}

/// The handle of an open net (a pointer of the API, kept as a number so
/// that it can cross threads).
pub type Handle = usize;

/// The raw API calls. The real one calls the DLL; the tests use a fake.
/// Every method returns the NTCAN code of the call.
pub trait RawApi {
    fn open(&mut self, net: i32, tx_timeout_ms: i32, rx_timeout_ms: i32) -> Result<Handle, i32>;
    fn set_baudrate(&mut self, h: Handle, baud: i32) -> i32;
    fn set_filter_mode(&mut self, h: Handle, mode: i32) -> i32;
    fn enable_all_ids(&mut self, h: Handle, on: bool) -> i32;
    /// Read up to `buf.len()` frames without waiting; `Ok(n)` frames read.
    fn read_no_wait(&mut self, h: Handle, buf: &mut [Cmsg]) -> Result<usize, i32>;
    /// Queue one frame; `Ok(n)` frames taken.
    fn send(&mut self, h: Handle, m: &Cmsg) -> Result<usize, i32>;
    fn status(&mut self, h: Handle) -> Result<CanIfStatus, i32>;
    fn close(&mut self, h: Handle) -> i32;
    fn device_list(&mut self) -> Result<Vec<Device>, i32>;
}

/// The time-outs of the worker calls.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// Load, open, close, status, send, list.
    pub call: Duration,
    /// Added to the receive time-out.
    pub receive_margin: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts { call: Duration::from_secs(3), receive_margin: Duration::from_millis(500) }
    }
}

/// The process-wide dead flag: after a hang the DLL stays stuck, so a new
/// adapter must not try again.
fn global_dead() -> Arc<AtomicBool> {
    static DEAD: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    DEAD.get_or_init(|| Arc::new(AtomicBool::new(false))).clone()
}

/// A place where the DLL is looked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Candidate {
    /// A full path; tried only when the file exists.
    Path(PathBuf),
    /// A bare file name for the normal Windows search.
    Name(String),
}

/// The lookup order of the brief: `CANOXIDE_SIECA_DLL`, the DLL by name,
/// then the Sontheim folders under Program Files (x86) and Program Files.
pub fn dll_candidates() -> Vec<Candidate> {
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os(DLL_ENV).filter(|p| !p.is_empty()) {
        v.push(Candidate::Path(PathBuf::from(p)));
    }
    v.push(Candidate::Name(DLL_NAME.to_string()));
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = std::env::var_os(var) {
            v.push(Candidate::Path(PathBuf::from(root).join("Sontheim").join("MT_Api").join(DLL_NAME)));
        }
    }
    v
}

/// Try the candidates in order with `load`; the first success wins. A
/// path that does not exist is skipped without a call. The error is
/// `NOT_FOUND_TEXT`, with the last load error when a file existed.
pub fn find_library<L>(cands: &[Candidate], mut load: impl FnMut(&Candidate) -> Result<L, String>) -> Result<L, CanError> {
    let mut last: Option<String> = None;
    for c in cands {
        if let Candidate::Path(p) = c {
            if !p.is_file() {
                continue;
            }
        }
        match load(c) {
            Ok(l) => return Ok(l),
            Err(e) => {
                if let Candidate::Path(p) = c {
                    last = Some(format!("{}: {e}", p.display()));
                }
            }
        }
    }
    Err(CanError::new(match last {
        Some(e) => format!("{NOT_FOUND_TEXT} ({e})"),
        None => NOT_FOUND_TEXT.to_string(),
    }))
}

// ---- the real DLL ----

type FnOpen = unsafe extern "system" fn(i32, i32, i32, i32, i32, *const c_char, *const c_char, *const c_char, *mut *mut c_void) -> i32;
type FnHandleLong = unsafe extern "system" fn(*mut c_void, i32) -> i32;
type FnEnableAllIds = unsafe extern "system" fn(*mut c_void, bool) -> i32;
type FnMsgs = unsafe extern "system" fn(*mut c_void, *mut Cmsg, *mut i32) -> i32;
type FnStatus = unsafe extern "system" fn(*mut c_void, *mut CanIfStatus) -> i32;
type FnClose = unsafe extern "system" fn(*mut c_void) -> i32;
type FnNumDevices = unsafe extern "system" fn(*mut i32) -> i32;
type FnDeviceList = unsafe extern "system" fn(*mut DeviceList) -> i32;

struct DllApi {
    open: FnOpen,
    set_baudrate: FnHandleLong,
    set_filter_mode: FnHandleLong,
    enable_all_ids: FnEnableAllIds,
    read_no_wait: FnMsgs,
    send: FnMsgs,
    status: FnStatus,
    close: FnClose,
    num_devices: FnNumDevices,
    device_list: FnDeviceList,
    // Never unloaded: the driver may keep threads in the DLL, and the
    // function pointers above must stay valid while the worker runs.
    _lib: std::mem::ManuallyDrop<libloading::Library>,
}

impl DllApi {
    fn load() -> Result<DllApi, CanError> {
        let lib = find_library(&dll_candidates(), |c| {
            // SAFETY: loading a DLL runs its DllMain; the candidates are
            // the Sontheim 64-bit API (fixed paths, or the name for the
            // normal Windows search as the brief asks).
            let r = unsafe {
                match c {
                    Candidate::Path(p) => libloading::Library::new(p),
                    Candidate::Name(n) => libloading::Library::new(n),
                }
            };
            r.map_err(|e| e.to_string())
        })?;
        // SAFETY: the symbol types match the prototypes of the header
        // (__stdcall = "system"; long = i32; bool = 1 byte).
        unsafe {
            let open = *lib.get::<FnOpen>(b"canOpen\0").map_err(|e| CanError::new(format!("canOpen: {e}")))?;
            let set_baudrate = *lib.get::<FnHandleLong>(b"canSetBaudrate\0").map_err(|e| CanError::new(format!("canSetBaudrate: {e}")))?;
            let set_filter_mode = *lib.get::<FnHandleLong>(b"canSetFilterMode\0").map_err(|e| CanError::new(format!("canSetFilterMode: {e}")))?;
            let enable_all_ids = *lib.get::<FnEnableAllIds>(b"canEnableAllIds\0").map_err(|e| CanError::new(format!("canEnableAllIds: {e}")))?;
            let read_no_wait = *lib.get::<FnMsgs>(b"canReadNoWait\0").map_err(|e| CanError::new(format!("canReadNoWait: {e}")))?;
            let send = *lib.get::<FnMsgs>(b"canSend\0").map_err(|e| CanError::new(format!("canSend: {e}")))?;
            let status = *lib.get::<FnStatus>(b"canStatus\0").map_err(|e| CanError::new(format!("canStatus: {e}")))?;
            let close = *lib.get::<FnClose>(b"canClose\0").map_err(|e| CanError::new(format!("canClose: {e}")))?;
            let num_devices = *lib
                .get::<FnNumDevices>(b"canGetNumberOfConnectedDevices\0")
                .map_err(|e| CanError::new(format!("canGetNumberOfConnectedDevices: {e}")))?;
            let device_list = *lib.get::<FnDeviceList>(b"canGetDeviceList\0").map_err(|e| CanError::new(format!("canGetDeviceList: {e}")))?;
            Ok(DllApi {
                open,
                set_baudrate,
                set_filter_mode,
                enable_all_ids,
                read_no_wait,
                send,
                status,
                close,
                num_devices,
                device_list,
                _lib: std::mem::ManuallyDrop::new(lib),
            })
        }
    }
}

fn ptr(h: Handle) -> *mut c_void {
    h as *mut c_void
}

impl RawApi for DllApi {
    fn open(&mut self, net: i32, tx_timeout_ms: i32, rx_timeout_ms: i32) -> Result<Handle, i32> {
        let pid = std::process::id();
        let app = CString::new("CANoxide").expect("no NUL");
        let rx_ev = CString::new(format!("CANoxide_R{pid}_{net}")).expect("no NUL");
        let err_ev = CString::new(format!("CANoxide_E{pid}_{net}")).expect("no NUL");
        let mut h: *mut c_void = std::ptr::null_mut();
        // SAFETY: valid C strings that live through the call, a valid out
        // pointer; mode 0 (no error frames), echo 0 (off).
        let r = unsafe { (self.open)(net, 0, 0, tx_timeout_ms, rx_timeout_ms, app.as_ptr(), rx_ev.as_ptr(), err_ev.as_ptr(), &mut h) };
        if r == NTCAN_SUCCESS && !h.is_null() {
            Ok(h as Handle)
        } else if r == NTCAN_SUCCESS {
            Err(NTCAN_INVALID_HANDLE)
        } else {
            Err(r)
        }
    }

    fn set_baudrate(&mut self, h: Handle, baud: i32) -> i32 {
        // SAFETY: handle from canOpen.
        unsafe { (self.set_baudrate)(ptr(h), baud) }
    }

    fn set_filter_mode(&mut self, h: Handle, mode: i32) -> i32 {
        // SAFETY: handle from canOpen; T_FILTER_MODE is a C enum (int).
        unsafe { (self.set_filter_mode)(ptr(h), mode) }
    }

    fn enable_all_ids(&mut self, h: Handle, on: bool) -> i32 {
        // SAFETY: handle from canOpen.
        unsafe { (self.enable_all_ids)(ptr(h), on) }
    }

    fn read_no_wait(&mut self, h: Handle, buf: &mut [Cmsg]) -> Result<usize, i32> {
        let mut len = buf.len() as i32;
        // SAFETY: the buffer holds `len` CMSG; the API writes at most that.
        let r = unsafe { (self.read_no_wait)(ptr(h), buf.as_mut_ptr(), &mut len) };
        if r == NTCAN_SUCCESS {
            Ok((len.max(0) as usize).min(buf.len()))
        } else {
            Err(r)
        }
    }

    fn send(&mut self, h: Handle, m: &Cmsg) -> Result<usize, i32> {
        let mut msg = *m;
        let mut len: i32 = 1;
        // SAFETY: one valid CMSG, length 1.
        let r = unsafe { (self.send)(ptr(h), &mut msg, &mut len) };
        if r == NTCAN_SUCCESS {
            Ok(len.max(0) as usize)
        } else {
            Err(r)
        }
    }

    fn status(&mut self, h: Handle) -> Result<CanIfStatus, i32> {
        let mut s = CanIfStatus::default();
        // SAFETY: valid out struct of the header layout.
        let r = unsafe { (self.status)(ptr(h), &mut s) };
        if r == NTCAN_SUCCESS {
            Ok(s)
        } else {
            Err(r)
        }
    }

    fn close(&mut self, h: Handle) -> i32 {
        // SAFETY: handle from canOpen, closed once.
        unsafe { (self.close)(ptr(h)) }
    }

    fn device_list(&mut self) -> Result<Vec<Device>, i32> {
        let mut n: i32 = 0;
        // SAFETY: valid out pointer.
        let r = unsafe { (self.num_devices)(&mut n) };
        if r != NTCAN_SUCCESS {
            return Err(r);
        }
        if n <= 0 {
            return Ok(Vec::new());
        }
        // A fixed, large buffer: a device plugged in between the two calls
        // cannot overrun it.
        let n = (n as usize).min(DEVICE_LIST_LEN);
        let mut list = vec![DeviceList::default(); DEVICE_LIST_LEN];
        // SAFETY: the buffer holds DEVICE_LIST_LEN entries.
        let r = unsafe { (self.device_list)(list.as_mut_ptr()) };
        if r != NTCAN_SUCCESS {
            return Err(r);
        }
        Ok(list[..n].iter().map(device_from).collect())
    }
}

fn device_from(d: &DeviceList) -> Device {
    let end = d.name.iter().position(|&b| b == 0).unwrap_or(d.name.len());
    Device { net: d.net, name: String::from_utf8_lossy(&d.name[..end]).trim().to_string() }
}

// ---- the worker ----

/// The state on the worker thread.
struct Core {
    api: Box<dyn RawApi>,
    handle: Option<Handle>,
}

type Job = Box<dyn FnOnce(&mut Core) + Send>;
type Loader = Box<dyn FnOnce() -> Result<Box<dyn RawApi>, CanError> + Send>;

fn worker(loader: Loader, loaded: Sender<Result<(), CanError>>, jobs: Receiver<Job>) {
    let api = match loader() {
        Ok(a) => a,
        Err(e) => {
            let _ = loaded.send(Err(e));
            return;
        }
    };
    let _ = loaded.send(Ok(()));
    let mut core = Core { api, handle: None };
    // Ends when the adapter is dropped (the job sender closes).
    while let Ok(job) = jobs.recv() {
        job(&mut core);
    }
    if let Some(h) = core.handle.take() {
        core.api.close(h);
    }
}

/// tx time-out of canOpen in ms (0 and -1 would mean infinite).
const TX_TIMEOUT_MS: i32 = 100;
/// rx time-out of canOpen in ms (canReadNoWait does not wait).
const RX_TIMEOUT_MS: i32 = 10;

fn open_net(core: &mut Core, net: i32, baud: i32, listen_only: bool) -> Result<(), CanError> {
    if let Some(h) = core.handle.take() {
        core.api.close(h);
    }
    let h = core
        .api
        .open(net, TX_TIMEOUT_MS, RX_TIMEOUT_MS)
        .map_err(|r| CanError::new(format!("CANfox net {net}: {}", error_text(r))))?;
    let fail = |core: &mut Core, what: &str, r: i32| -> CanError {
        core.api.close(h);
        CanError::new(format!("CANfox net {net}: {what}: {}", error_text(r)))
    };
    let r = core.api.set_baudrate(h, baud);
    if r != NTCAN_SUCCESS {
        let what = if r == NTCAN_DLL_ALREADY_INIT || r == NTCAN_NO_OWNER_RIGHTS {
            if listen_only {
                "the bitrate (listen-only) could not be set; the net is in use by another program or runs at another bitrate"
            } else {
                "the bitrate could not be set; the net is in use by another program or runs at another bitrate"
            }
        } else if listen_only {
            "listen-only bitrate"
        } else {
            "bitrate"
        };
        return Err(fail(core, what, r));
    }
    let r = core.api.set_filter_mode(h, FILTER_MODE_NOFILTER);
    if r != NTCAN_SUCCESS {
        return Err(fail(core, "receive filter", r));
    }
    let r = core.api.enable_all_ids(h, true);
    if r != NTCAN_SUCCESS {
        return Err(fail(core, "enable all identifiers", r));
    }
    core.handle = Some(h);
    Ok(())
}

/// The frames of one receive call and whether the API reported a loss.
struct Batch {
    frames: Vec<CanFrame>,
    lost: bool,
}

fn read_batch(core: &mut Core, timeout: Duration) -> Result<Batch, CanError> {
    let h = core.handle.ok_or_else(|| CanError::new("channel not open"))?;
    let end = Instant::now() + timeout;
    let mut buf = [Cmsg::default(); READ_BATCH];
    loop {
        match core.api.read_no_wait(h, &mut buf) {
            Ok(n) if n > 0 => {
                let lost = buf[..n].iter().any(|m| m.by_msg_lost != 0);
                let frames: Vec<CanFrame> = buf[..n].iter().filter_map(cmsg_to_frame).collect();
                if !frames.is_empty() || lost {
                    return Ok(Batch { frames, lost });
                }
            }
            Ok(_) | Err(NTCAN_RX_TIMEOUT) => {}
            // Bus off is shown by status(), not as a receive error.
            Err(NTCAN_CONTR_BUSOFF) => {}
            Err(r) => return Err(CanError::new(error_text(r))),
        }
        if Instant::now() >= end {
            return Ok(Batch { frames: Vec::new(), lost: false });
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// ifm CANfox through the Sontheim MT-API.
pub struct SontheimAdapter {
    jobs: Option<Sender<Job>>,
    dead: Arc<AtomicBool>,
    t: Timeouts,
    open: bool,
    listen_only: bool,
    /// Frames lost since the net was opened.
    lost: bool,
    rx: VecDeque<CanFrame>,
}

impl SontheimAdapter {
    /// Load SIECA164.DLL on a new worker thread. The error is
    /// `NOT_FOUND_TEXT` without the DLL, `DEAD_TEXT` after a hang.
    pub fn load() -> Result<SontheimAdapter, CanError> {
        SontheimAdapter::with_loader(
            Box::new(|| DllApi::load().map(|a| Box::new(a) as Box<dyn RawApi>)),
            Timeouts::default(),
            global_dead(),
        )
    }

    fn with_loader(loader: Loader, t: Timeouts, dead: Arc<AtomicBool>) -> Result<SontheimAdapter, CanError> {
        if dead.load(Ordering::SeqCst) {
            return Err(CanError::new(DEAD_TEXT));
        }
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (loaded_tx, loaded_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("sontheim-api".to_string())
            .spawn(move || worker(loader, loaded_tx, job_rx))
            .map_err(|e| CanError::new(format!("Sontheim driver thread: {e}")))?;
        match loaded_rx.recv_timeout(t.call) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(RecvTimeoutError::Timeout) => {
                dead.store(true, Ordering::SeqCst);
                // The job sender is dropped here; the stuck thread is left.
                return Err(CanError::new(DEAD_TEXT));
            }
            Err(RecvTimeoutError::Disconnected) => return Err(CanError::new("Sontheim driver thread ended")),
        }
        Ok(SontheimAdapter { jobs: Some(job_tx), dead, t, open: false, listen_only: false, lost: false, rx: VecDeque::new() })
    }

    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// Run `f` on the worker and wait at most `wait` for its result.
    fn call<R: Send + 'static>(&mut self, wait: Duration, f: impl FnOnce(&mut Core) -> R + Send + 'static) -> Result<R, CanError> {
        if self.is_dead() {
            return Err(CanError::new(DEAD_TEXT));
        }
        let jobs = self.jobs.as_ref().ok_or_else(|| CanError::new("Sontheim driver thread ended"))?;
        let (tx, rx) = mpsc::sync_channel::<R>(1);
        let job: Job = Box::new(move |core: &mut Core| {
            let _ = tx.send(f(core));
        });
        if jobs.send(job).is_err() {
            return Err(CanError::new("Sontheim driver thread ended"));
        }
        match rx.recv_timeout(wait) {
            Ok(r) => Ok(r),
            Err(RecvTimeoutError::Timeout) => {
                self.dead.store(true, Ordering::SeqCst);
                self.open = false;
                self.rx.clear();
                Err(CanError::new(DEAD_TEXT))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.open = false;
                Err(CanError::new("Sontheim driver thread ended"))
            }
        }
    }
}

/// The channel entries for a device list (one per net); net 105 when the
/// API lists nothing.
fn channels_from(devices: &[Device]) -> Vec<ChannelInfo> {
    let mut v: Vec<ChannelInfo> = Vec::new();
    for d in devices {
        let Ok(net) = u32::try_from(d.net) else { continue };
        if v.iter().any(|c| c.handle == net) {
            continue;
        }
        let name = if d.name.is_empty() { format!("CANfox net {net}") } else { format!("Net {net} ({})", d.name) };
        v.push(ChannelInfo { handle: net, name, available: true });
    }
    if v.is_empty() {
        v.push(ChannelInfo { handle: CANFOX_NET, name: format!("CANfox net {CANFOX_NET}"), available: true });
    }
    v
}

impl CanAdapter for SontheimAdapter {
    fn name(&self) -> String {
        "CANfox".to_string()
    }

    fn list_channels(&mut self) -> Result<Vec<ChannelInfo>, CanError> {
        let t = self.t.call;
        match self.call(t, |core| core.api.device_list())? {
            Ok(devices) => Ok(channels_from(&devices)),
            Err(_) => Ok(channels_from(&[])),
        }
    }

    fn open(&mut self, channel: u32, bitrate: Bitrate, listen_only: bool) -> Result<(), CanError> {
        self.close();
        let net = i32::try_from(channel).map_err(|_| CanError::new("invalid net number"))?;
        let baud = baud_value(bitrate, listen_only);
        let t = self.t.call;
        self.call(t, move |core| open_net(core, net, baud, listen_only))??;
        self.open = true;
        self.listen_only = listen_only;
        self.lost = false;
        self.rx.clear();
        Ok(())
    }

    fn close(&mut self) {
        self.rx.clear();
        if !self.open {
            return;
        }
        self.open = false;
        if self.is_dead() {
            return;
        }
        // Queued without waiting: the worker runs the jobs in order, so a
        // later open comes after this close.
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Box::new(|core: &mut Core| {
                if let Some(h) = core.handle.take() {
                    core.api.close(h);
                }
            }));
        }
    }

    fn send(&mut self, frame: &CanFrame) -> Result<(), CanError> {
        if self.is_dead() {
            return Err(CanError::new(DEAD_TEXT));
        }
        if !self.open {
            return Err(CanError::new("channel not open"));
        }
        if self.listen_only {
            return Err(CanError::new("listen-only mode: sending is disabled"));
        }
        frame.validate()?;
        let m = frame_to_cmsg(frame);
        let t = self.t.call;
        let r = self.call(t, move |core| match core.handle {
            None => Err(NTCAN_INVALID_HANDLE),
            Some(h) => core.api.send(h, &m),
        })?;
        match r {
            Ok(n) if n >= 1 => Ok(()),
            Ok(_) => Err(CanError::new("the driver did not take the frame")),
            Err(code) => Err(CanError::new(error_text(code))),
        }
    }

    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, CanError> {
        if self.is_dead() {
            return Err(CanError::new(DEAD_TEXT));
        }
        if let Some(f) = self.rx.pop_front() {
            return Ok(Some(f));
        }
        if !self.open {
            return Err(CanError::new("channel not open"));
        }
        let wait = timeout + self.t.receive_margin;
        let b = self.call(wait, move |core| read_batch(core, timeout))??;
        self.lost |= b.lost;
        self.rx.extend(b.frames);
        Ok(self.rx.pop_front())
    }

    fn status(&mut self) -> String {
        if self.is_dead() {
            return DEAD_TEXT.to_string();
        }
        if !self.open {
            return "closed".to_string();
        }
        let t = self.t.call;
        let lost = self.lost;
        match self.call(t, |core| match core.handle {
            None => Err(NTCAN_INVALID_HANDLE),
            Some(h) => core.api.status(h),
        }) {
            Ok(Ok(s)) => status_text(s.w_errorflag, lost),
            Ok(Err(code)) => error_text(code),
            Err(e) => e.0,
        }
    }
}

impl Drop for SontheimAdapter {
    fn drop(&mut self) {
        self.close();
        // Dropping the sender ends the worker after its last job; nobody
        // waits for it.
        self.jobs = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};
    use std::sync::Mutex;

    #[test]
    fn struct_layout_matches_the_header() {
        assert_eq!(size_of::<Cmsg>(), 20);
        assert_eq!(offset_of!(Cmsg, l_id), 0);
        assert_eq!(offset_of!(Cmsg, by_len), 4);
        assert_eq!(offset_of!(Cmsg, by_msg_lost), 5);
        assert_eq!(offset_of!(Cmsg, by_extended), 6);
        assert_eq!(offset_of!(Cmsg, by_remote), 7);
        assert_eq!(offset_of!(Cmsg, aby_data), 8);
        assert_eq!(offset_of!(Cmsg, ul_tstamp), 16);
        assert_eq!(size_of::<CanIfStatus>(), 32);
        assert_eq!(offset_of!(CanIfStatus, ul_board_status), 8);
        assert_eq!(offset_of!(CanIfStatus, by_board_id), 12);
        assert_eq!(offset_of!(CanIfStatus, w_busoffctr), 14);
        assert_eq!(offset_of!(CanIfStatus, w_errorflag), 16);
        assert_eq!(offset_of!(CanIfStatus, w_errorframectr), 18);
        assert_eq!(offset_of!(CanIfStatus, w_netctr), 20);
        assert_eq!(offset_of!(CanIfStatus, l_baud), 24);
        assert_eq!(offset_of!(CanIfStatus, ui_epld_rev), 28);
        assert_eq!(size_of::<DeviceList>(), 104);
        assert_eq!(offset_of!(DeviceList, name), 4);
        assert_eq!(offset_of!(DeviceList, ul_status), 24);
        assert_eq!(offset_of!(DeviceList, ul_features), 28);
        assert_eq!(offset_of!(DeviceList, reserved), 32);
        assert_eq!(size_of::<bool>(), 1);
    }

    #[test]
    fn baud_codes_and_silent_flag() {
        let expect = [
            (Bitrate::M1, 0),
            (Bitrate::K800, 1),
            (Bitrate::K500, 2),
            (Bitrate::K250, 3),
            (Bitrate::K125, 4),
            (Bitrate::K50, 6),
            (Bitrate::K20, 7),
            (Bitrate::K10, 8),
        ];
        assert_eq!(expect.len(), Bitrate::ALL.len());
        for (b, code) in expect {
            assert_eq!(baud_code(b), code, "{b:?}");
            assert_eq!(baud_value(b, false), code, "{b:?}");
            assert_eq!(baud_value(b, true), code | 0x4000_0000, "{b:?}");
        }
    }

    #[test]
    fn frame_mapping_both_ways() {
        // Standard, dlc 8.
        let f = CanFrame::new(0x123, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let m = frame_to_cmsg(&f);
        assert_eq!((m.l_id, m.by_len, m.by_extended, m.by_remote), (0x123, 8, 0, 0));
        assert_eq!(m.aby_data, [1, 2, 3, 4, 5, 6, 7, 8]);
        let back = cmsg_to_frame(&Cmsg { by_extended: MSG_STANDARD, ..m }).unwrap();
        assert_eq!((back.id, back.extended, back.rtr, back.dlc, back.data, back.dir), (0x123, false, false, 8, f.data, Dir::Rx));
        // Extended, dlc 0.
        let mut e = CanFrame::new(0x1ABC_DEF0, &[]);
        e.extended = true;
        let m = frame_to_cmsg(&e);
        assert_eq!((m.l_id, m.by_len, m.by_extended, m.by_remote), (0x1ABC_DEF0, 0, MSG_EXTENDED, 0));
        let back = cmsg_to_frame(&m).unwrap();
        assert!(back.extended && !back.rtr);
        assert_eq!((back.id, back.dlc), (0x1ABC_DEF0, 0));
        // RTR with dlc 4.
        let mut r = CanFrame::new(0x701, &[]);
        r.rtr = true;
        r.dlc = 4;
        let m = frame_to_cmsg(&r);
        assert_eq!((m.by_remote, m.by_len), (1, 4));
        let back = cmsg_to_frame(&m).unwrap();
        assert!(back.rtr);
        assert_eq!(back.dlc, 4);
        assert!(back.payload().is_empty());
        // A dlc above 8 is clamped; error frames and echoes are skipped.
        let big = Cmsg { l_id: 0x10, by_len: 0x0F, by_extended: MSG_STANDARD, ..Cmsg::default() };
        assert_eq!(cmsg_to_frame(&big).unwrap().dlc, 8);
        assert!(cmsg_to_frame(&Cmsg { by_extended: MSG_ERRORFRAME, ..Cmsg::default() }).is_none());
        assert!(cmsg_to_frame(&Cmsg { by_extended: MSG_ECHO | MSG_STANDARD, ..Cmsg::default() }).is_none());
    }

    #[test]
    fn error_and_status_texts() {
        assert!(error_text(NTCAN_NO_OWNER_RIGHTS).contains("owns this net"));
        assert!(error_text(NTCAN_DLL_ALREADY_INIT).contains("another program"));
        assert!(error_text(NTCAN_HARDWARE_NOT_FOUND).contains("No CANfox"));
        assert!(error_text(NTCAN_API_NOT_RUNNING).contains("SIECE132Svr"));
        assert_eq!(error_text(-9999), "Sontheim driver error -9999.");
        assert_eq!(status_text(0, false), "OK");
        assert_eq!(status_text(0, true), "OK, frames lost");
        assert_eq!(status_text(ERR_BUSOFF, false), "Bus off");
        assert_eq!(status_text(ERR_ERRORPASSIVE | ERR_RBUFOVERFLOW, false), "Error passive, Receive buffer overflow");
        assert_eq!(status_text(ERR_HWUNPLUGGED, false), "Device unplugged");
    }

    #[test]
    fn lookup_takes_the_candidates_in_order() {
        let dir = std::env::temp_dir().join(format!("canoxide_sieca_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let present = dir.join("SIECA164.DLL");
        std::fs::write(&present, b"not a dll").unwrap();
        let missing = dir.join("missing").join("SIECA164.DLL");
        let mut tried = Vec::new();
        let cands = vec![
            Candidate::Path(missing.clone()),
            Candidate::Name("SIECA164.DLL".into()),
            Candidate::Path(present.clone()),
        ];
        // Nothing loads: the missing path is never tried; the error names
        // the file that existed.
        let r: Result<(), CanError> = find_library(&cands, |c| {
            tried.push(c.clone());
            Err("bad image".to_string())
        });
        let e = r.unwrap_err().0;
        assert!(e.starts_with(NOT_FOUND_TEXT), "{e}");
        assert!(e.contains("bad image"), "{e}");
        assert_eq!(tried, vec![cands[1].clone(), cands[2].clone()]);
        // The first that loads wins.
        let r = find_library(&cands, |c| match c {
            Candidate::Path(p) if p == &present => Ok(7),
            _ => Err("no".to_string()),
        });
        assert_eq!(r.unwrap(), 7);
        // Nothing exists: the plain not-found text.
        let r: Result<(), CanError> = find_library(&[Candidate::Path(missing)], |_| Ok(()));
        assert_eq!(r.unwrap_err().0, NOT_FOUND_TEXT);
        let r: Result<(), CanError> = find_library(&[], |_| Ok(()));
        assert_eq!(r.unwrap_err().0, NOT_FOUND_TEXT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidate_order_of_the_brief() {
        let c = dll_candidates();
        let name = c.iter().position(|x| *x == Candidate::Name(DLL_NAME.to_string())).expect("by name");
        let x86 = c.iter().position(|x| matches!(x, Candidate::Path(p) if p.to_string_lossy().contains("(x86)")));
        if let Some(x86) = x86 {
            assert!(name < x86);
        }
        assert!(c.iter().all(|x| !format!("{x:?}").contains("SIECA132")));
    }

    // ---- the fake API ----

    #[derive(Default)]
    struct Log {
        calls: Vec<String>,
        open_handles: Vec<Handle>,
    }

    #[derive(Default, Clone)]
    struct Script {
        open: Option<i32>,
        baud: Option<i32>,
        filter: Option<i32>,
        enable: Option<i32>,
        /// Frames given by the next reads.
        frames: Vec<Cmsg>,
        read_err: Option<i32>,
        errorflag: u16,
        devices: Option<Vec<Device>>,
        /// Block this call (by name) for this long.
        block: Option<(&'static str, Duration)>,
    }

    struct Fake {
        s: Script,
        log: Arc<Mutex<Log>>,
        next: Handle,
    }

    impl Fake {
        fn note(&mut self, what: &str) {
            if let Some((name, d)) = self.s.block {
                if what.starts_with(name) {
                    std::thread::sleep(d);
                }
            }
            self.log.lock().unwrap().calls.push(what.to_string());
        }
    }

    impl RawApi for Fake {
        fn open(&mut self, net: i32, tx: i32, rx: i32) -> Result<Handle, i32> {
            self.note(&format!("open {net} {tx} {rx}"));
            if let Some(e) = self.s.open {
                return Err(e);
            }
            self.next += 1;
            self.log.lock().unwrap().open_handles.push(self.next);
            Ok(self.next)
        }
        fn set_baudrate(&mut self, _h: Handle, baud: i32) -> i32 {
            self.note(&format!("baud 0x{baud:X}"));
            self.s.baud.unwrap_or(NTCAN_SUCCESS)
        }
        fn set_filter_mode(&mut self, _h: Handle, mode: i32) -> i32 {
            self.note(&format!("filter {mode}"));
            self.s.filter.unwrap_or(NTCAN_SUCCESS)
        }
        fn enable_all_ids(&mut self, _h: Handle, on: bool) -> i32 {
            self.note(&format!("enable {on}"));
            self.s.enable.unwrap_or(NTCAN_SUCCESS)
        }
        fn read_no_wait(&mut self, _h: Handle, buf: &mut [Cmsg]) -> Result<usize, i32> {
            self.note("read");
            if let Some(e) = self.s.read_err {
                return Err(e);
            }
            if self.s.frames.is_empty() {
                return Err(NTCAN_RX_TIMEOUT);
            }
            let n = self.s.frames.len().min(buf.len());
            for (i, m) in self.s.frames.drain(..n).enumerate() {
                buf[i] = m;
            }
            Ok(n)
        }
        fn send(&mut self, _h: Handle, m: &Cmsg) -> Result<usize, i32> {
            self.note(&format!("send 0x{:X}", m.l_id));
            Ok(1)
        }
        fn status(&mut self, _h: Handle) -> Result<CanIfStatus, i32> {
            self.note("status");
            Ok(CanIfStatus { w_errorflag: self.s.errorflag, ..CanIfStatus::default() })
        }
        fn close(&mut self, h: Handle) -> i32 {
            self.note("close");
            self.log.lock().unwrap().open_handles.retain(|&x| x != h);
            NTCAN_SUCCESS
        }
        fn device_list(&mut self) -> Result<Vec<Device>, i32> {
            self.note("list");
            self.s.devices.clone().ok_or(NTCAN_INVALID_PARAMETER)
        }
    }

    fn fast() -> Timeouts {
        Timeouts { call: Duration::from_millis(150), receive_margin: Duration::from_millis(100) }
    }

    fn adapter(s: Script) -> (SontheimAdapter, Arc<Mutex<Log>>, Arc<AtomicBool>) {
        let log = Arc::new(Mutex::new(Log::default()));
        let l2 = log.clone();
        let dead = Arc::new(AtomicBool::new(false));
        let a = SontheimAdapter::with_loader(Box::new(move || Ok(Box::new(Fake { s, log: l2, next: 0x1000 }) as Box<dyn RawApi>)), fast(), dead.clone()).unwrap();
        (a, log, dead)
    }

    fn calls(log: &Arc<Mutex<Log>>) -> Vec<String> {
        log.lock().unwrap().calls.clone()
    }

    /// Wait until the worker has run the queued jobs (close is not awaited).
    fn settle(a: &mut SontheimAdapter) {
        let _ = a.call(Duration::from_secs(1), |_| ());
    }

    #[test]
    fn open_sequence_sends_nothing() {
        let (mut a, log, _) = adapter(Script::default());
        a.open(105, Bitrate::K250, false).unwrap();
        assert_eq!(calls(&log), ["open 105 100 10", "baud 0x3", "filter 4", "enable true"]);
        assert_eq!(a.name(), "CANfox");
        assert!(!a.keeps_timestamps());
        a.close();
        settle(&mut a);
        assert!(log.lock().unwrap().open_handles.is_empty());
    }

    #[test]
    fn listen_only_sets_the_silent_flag_and_refuses_send() {
        let (mut a, log, _) = adapter(Script::default());
        a.open(105, Bitrate::K125, true).unwrap();
        assert!(calls(&log).contains(&"baud 0x40000004".to_string()));
        let e = a.send(&CanFrame::new(0x100, &[1])).unwrap_err();
        assert!(e.0.contains("listen-only"), "{e}");
        assert!(!calls(&log).iter().any(|c| c.starts_with("send")));
    }

    #[test]
    fn each_open_failure_closes_the_handle() {
        for (name, s, text) in [
            ("baud owner", Script { baud: Some(NTCAN_NO_OWNER_RIGHTS), ..Script::default() }, "another program"),
            ("baud init", Script { baud: Some(NTCAN_DLL_ALREADY_INIT), ..Script::default() }, "another bitrate"),
            ("baud other", Script { baud: Some(NTCAN_INVALID_PARAMETER), ..Script::default() }, "bitrate"),
            ("filter", Script { filter: Some(NTCAN_INVALID_HANDLE), ..Script::default() }, "receive filter"),
            ("enable", Script { enable: Some(NTCAN_INVALID_HANDLE), ..Script::default() }, "identifiers"),
        ] {
            let (mut a, log, _) = adapter(s);
            let e = a.open(105, Bitrate::K250, false).unwrap_err();
            assert!(e.0.contains(text), "{name}: {e}");
            assert!(e.0.starts_with("CANfox net 105"), "{name}: {e}");
            assert!(log.lock().unwrap().open_handles.is_empty(), "{name}: handle left open");
            assert!(calls(&log).last().unwrap() == "close", "{name}");
            assert!(a.send(&CanFrame::new(1, &[])).is_err());
        }
        // canOpen itself fails: nothing to close.
        let (mut a, log, _) = adapter(Script { open: Some(NTCAN_HARDWARE_NOT_FOUND), ..Script::default() });
        let e = a.open(105, Bitrate::K250, false).unwrap_err();
        assert!(e.0.contains("No CANfox"), "{e}");
        assert_eq!(calls(&log), ["open 105 100 10"]);
    }

    #[test]
    fn send_and_receive() {
        let rx = vec![
            Cmsg { l_id: 0x181, by_len: 2, by_extended: MSG_STANDARD, aby_data: [9, 8, 0, 0, 0, 0, 0, 0], ..Cmsg::default() },
            Cmsg { l_id: 0x18FF_0001, by_len: 8, by_extended: MSG_EXTENDED, ..Cmsg::default() },
        ];
        let (mut a, log, _) = adapter(Script { frames: rx, ..Script::default() });
        a.open(105, Bitrate::K500, false).unwrap();
        let mut f = CanFrame::new(0x605, &[0x40, 0x00, 0x10, 0]);
        a.send(&f).unwrap();
        assert!(calls(&log).contains(&"send 0x605".to_string()));
        f.id = 0x800;
        assert!(a.send(&f).is_err(), "validate first");
        let r1 = a.receive(Duration::from_millis(5)).unwrap().unwrap();
        assert_eq!((r1.id, r1.dlc, r1.payload()), (0x181, 2, &[9u8, 8][..]));
        // The second frame of the batch comes from the buffer.
        let reads = calls(&log).iter().filter(|c| *c == "read").count();
        let r2 = a.receive(Duration::ZERO).unwrap().unwrap();
        assert!(r2.extended);
        assert_eq!(r2.id, 0x18FF_0001);
        assert_eq!(calls(&log).iter().filter(|c| *c == "read").count(), reads);
        assert_eq!(a.status(), "OK");
    }

    #[test]
    fn receive_timeout_gives_none() {
        let (mut a, _, _) = adapter(Script::default());
        a.open(105, Bitrate::K250, true).unwrap();
        let t0 = Instant::now();
        assert_eq!(a.receive(Duration::from_millis(20)).unwrap(), None);
        let el = t0.elapsed();
        assert!(el >= Duration::from_millis(20) && el < Duration::from_millis(200), "{el:?}");
        assert_eq!(a.receive(Duration::ZERO).unwrap(), None);
    }

    #[test]
    fn receive_error_and_bus_off() {
        let (mut a, _, _) = adapter(Script { read_err: Some(NTCAN_DRIVER_ERROR), ..Script::default() });
        a.open(105, Bitrate::K250, true).unwrap();
        let e = a.receive(Duration::ZERO).unwrap_err();
        assert!(e.0.contains("internal error"), "{e}");
        let (mut a, _, _) = adapter(Script { read_err: Some(NTCAN_CONTR_BUSOFF), errorflag: ERR_BUSOFF, ..Script::default() });
        a.open(105, Bitrate::K250, true).unwrap();
        assert_eq!(a.receive(Duration::ZERO).unwrap(), None);
        assert_eq!(a.status(), "Bus off");
    }

    #[test]
    fn lost_frames_are_kept_and_shown() {
        let rx = vec![Cmsg { l_id: 0x80, by_len: 0, by_msg_lost: 3, by_extended: MSG_STANDARD, ..Cmsg::default() }];
        let (mut a, _, _) = adapter(Script { frames: rx, ..Script::default() });
        a.open(105, Bitrate::K250, true).unwrap();
        assert_eq!(a.status(), "OK");
        let f = a.receive(Duration::ZERO).unwrap().unwrap();
        assert_eq!(f.id, 0x80);
        assert_eq!(a.status(), "OK, frames lost");
    }

    #[test]
    fn channel_list() {
        let devs = vec![Device { net: 105, name: "CANfox".into() }, Device { net: 105, name: "dup".into() }, Device { net: 106, name: String::new() }];
        let (mut a, log, _) = adapter(Script { devices: Some(devs), ..Script::default() });
        let ch = a.list_channels().unwrap();
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0], ChannelInfo { handle: 105, name: "Net 105 (CANfox)".into(), available: true });
        assert_eq!(ch[1].name, "CANfox net 106");
        assert_eq!(calls(&log), ["list"], "no net opened");
        // No list: net 105.
        let (mut a, _, _) = adapter(Script::default());
        assert_eq!(a.list_channels().unwrap(), vec![ChannelInfo { handle: 105, name: "CANfox net 105".into(), available: true }]);
        let (mut a, _, _) = adapter(Script { devices: Some(Vec::new()), ..Script::default() });
        assert_eq!(a.list_channels().unwrap()[0].handle, 105);
    }

    #[test]
    fn not_open_and_closed_status() {
        let (mut a, _, _) = adapter(Script::default());
        assert_eq!(a.status(), "closed");
        assert!(a.send(&CanFrame::new(1, &[])).is_err());
        assert!(a.receive(Duration::ZERO).is_err());
    }

    #[test]
    fn a_hang_makes_the_adapter_dead_at_once() {
        let t0 = Instant::now();
        let (mut a, _, dead) = adapter(Script { block: Some(("status", Duration::from_secs(5))), ..Script::default() });
        a.open(105, Bitrate::K250, true).unwrap();
        let t1 = Instant::now();
        assert_eq!(a.status(), DEAD_TEXT);
        let el = t1.elapsed();
        assert!(el >= Duration::from_millis(150) && el < Duration::from_millis(400), "{el:?}");
        assert!(dead.load(Ordering::SeqCst));
        // Every later call returns at once.
        let t2 = Instant::now();
        assert_eq!(a.receive(Duration::from_millis(50)).unwrap_err().0, DEAD_TEXT);
        assert_eq!(a.send(&CanFrame::new(1, &[])).unwrap_err().0, DEAD_TEXT);
        assert_eq!(a.open(105, Bitrate::K250, true).unwrap_err().0, DEAD_TEXT);
        assert_eq!(a.list_channels().unwrap_err().0, DEAD_TEXT);
        assert_eq!(a.status(), DEAD_TEXT);
        a.close();
        drop(a);
        // A new adapter with the same dead flag fails at once, without a load.
        let r = SontheimAdapter::with_loader(Box::new(|| panic!("must not load")), fast(), dead.clone());
        assert_eq!(r.err().unwrap().0, DEAD_TEXT);
        assert!(t2.elapsed() < Duration::from_millis(20), "{:?}", t2.elapsed());
        assert!(t0.elapsed() < Duration::from_millis(900), "{:?}", t0.elapsed());
    }

    #[test]
    fn a_hang_in_receive_and_in_the_load() {
        let (mut a, _, dead) = adapter(Script { block: Some(("read", Duration::from_secs(5))), ..Script::default() });
        a.open(105, Bitrate::K250, true).unwrap();
        let t = Instant::now();
        assert_eq!(a.receive(Duration::from_millis(10)).unwrap_err().0, DEAD_TEXT);
        // receive timeout + margin = 110 ms.
        assert!(t.elapsed() < Duration::from_millis(350), "{:?}", t.elapsed());
        assert!(dead.load(Ordering::SeqCst));
        // A load that hangs.
        let dead = Arc::new(AtomicBool::new(false));
        let t = Instant::now();
        let r = SontheimAdapter::with_loader(
            Box::new(|| {
                std::thread::sleep(Duration::from_secs(5));
                Err(CanError::new("late"))
            }),
            fast(),
            dead.clone(),
        );
        assert_eq!(r.err().unwrap().0, DEAD_TEXT);
        assert!(t.elapsed() < Duration::from_millis(350), "{:?}", t.elapsed());
        assert!(dead.load(Ordering::SeqCst));
    }

    #[test]
    fn a_load_error_is_returned() {
        let r = SontheimAdapter::with_loader(Box::new(|| Err(CanError::new(NOT_FOUND_TEXT))), fast(), Arc::new(AtomicBool::new(false)));
        assert_eq!(r.err().unwrap().0, NOT_FOUND_TEXT);
    }
}
