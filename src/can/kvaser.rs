//! Kvaser backend through CANlib. `canlib32.dll` (the 64-bit DLL has the
//! same name, in System32) is loaded at run time with `libloading`; without
//! the DLL the rest of the tool works. NOT TESTED ON HARDWARE.
//!
//! The declarations below are written by hand from the public CANlib help
//! (kvaser.com/canlib-webhelp/canlib_8h.htm for the functions and the
//! canOPEN_, canBITRATE_, canDRIVER_ and canCHANNELDATA_ constants,
//! kvaser.com/canlib-webhelp/canstat_8h.htm for the status codes and the
//! canMSG_, canMSGERR_ and canSTAT_ flags). No Kvaser file is part of this
//! repository.
//!
//! As with the CANfox, every call into the DLL (the load included) runs on
//! one worker thread with a time-out; a call that does not come back makes
//! this and every later Kvaser adapter of the process dead.
//!
//! Nothing is sent by `open`: canOpenChannel (with canOPEN_ACCEPT_VIRTUAL,
//! never canOPEN_EXCLUSIVE), canSetBusParams, canSetBusOutputControl
//! (canDRIVER_SILENT for listen only) and canBusOn.

use super::{Bitrate, CanAdapter, CanError, CanFrame, ChannelInfo, Dir};
use std::collections::VecDeque;
use std::ffi::{c_char, c_long, c_uint, c_ulong, c_void};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The CANlib DLL.
pub const DLL_NAME: &str = "canlib32.dll";
/// The error without the DLL.
pub const NOT_FOUND_TEXT: &str = "Kvaser driver not found: install the Kvaser drivers";
/// The error after a call that did not come back.
pub const DEAD_TEXT: &str = "The Kvaser driver does not answer. Restart the program, then try again.";

// ---- canstat.h: status codes (canStatus, an int enum) ----
pub const CAN_OK: i32 = 0;
pub const CAN_ERR_PARAM: i32 = -1;
pub const CAN_ERR_NOMSG: i32 = -2;
pub const CAN_ERR_NOTFOUND: i32 = -3;
pub const CAN_ERR_NOMEM: i32 = -4;
pub const CAN_ERR_NOCHANNELS: i32 = -5;
pub const CAN_ERR_TIMEOUT: i32 = -7;
pub const CAN_ERR_NOTINITIALIZED: i32 = -8;
pub const CAN_ERR_NOHANDLES: i32 = -9;
pub const CAN_ERR_INVHANDLE: i32 = -10;
pub const CAN_ERR_DRIVER: i32 = -12;
pub const CAN_ERR_TXBUFOFL: i32 = -13;
pub const CAN_ERR_HARDWARE: i32 = -15;
pub const CAN_ERR_DRIVERLOAD: i32 = -23;
pub const CAN_ERR_NOT_IMPLEMENTED: i32 = -32;
pub const CAN_ERR_DEVICE_FILE: i32 = -33;

// ---- canstat.h: message flags ----
pub const CAN_MSG_RTR: u32 = 0x0001;
pub const CAN_MSG_STD: u32 = 0x0002;
pub const CAN_MSG_EXT: u32 = 0x0004;
pub const CAN_MSG_ERROR_FRAME: u32 = 0x0020;
pub const CAN_MSG_TXACK: u32 = 0x0040;
pub const CAN_MSG_TXRQ: u32 = 0x0080;
pub const CAN_MSGERR_OVERRUN: u32 = 0x0600;
pub const CAN_FDMSG_FDF: u32 = 0x0001_0000;

// ---- canstat.h: bus status flags (canReadStatus) ----
pub const CAN_STAT_ERROR_PASSIVE: u32 = 0x0001;
pub const CAN_STAT_BUS_OFF: u32 = 0x0002;
pub const CAN_STAT_ERROR_WARNING: u32 = 0x0004;
pub const CAN_STAT_ERROR_ACTIVE: u32 = 0x0008;
pub const CAN_STAT_HW_OVERRUN: u32 = 0x0200;
pub const CAN_STAT_SW_OVERRUN: u32 = 0x0400;

// ---- canlib.h ----
pub const CAN_OPEN_EXCLUSIVE: i32 = 0x0008;
pub const CAN_OPEN_ACCEPT_VIRTUAL: i32 = 0x0020;
pub const CAN_DRIVER_NORMAL: u32 = 4;
pub const CAN_DRIVER_SILENT: u32 = 1;
pub const CAN_CHANNELDATA_CHANNEL_NAME: i32 = 13;
pub const CAN_BITRATE_1M: i32 = -1;
pub const CAN_BITRATE_500K: i32 = -2;
pub const CAN_BITRATE_250K: i32 = -3;
pub const CAN_BITRATE_125K: i32 = -4;
pub const CAN_BITRATE_100K: i32 = -5;
pub const CAN_BITRATE_50K: i32 = -7;
pub const CAN_BITRATE_10K: i32 = -9;

/// The arguments of canSetBusParams: `freq` (a negative canBITRATE_
/// constant, or the bitrate in bit/s with the segments), tseg1, tseg2, sjw,
/// noSamp. syncmode is always 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusParams {
    pub freq: i32,
    pub tseg1: u32,
    pub tseg2: u32,
    pub sjw: u32,
    pub nosamp: u32,
}

/// The bus parameters of a bitrate. CANlib has no predefined 20k and
/// 800k: these two are given in bit/s with 16 (20k, sample point 87.5 %)
/// and 10 (800k, sample point 80 %) time quanta per bit (not checked on
/// hardware).
pub fn bus_params(b: Bitrate) -> BusParams {
    let pre = |freq| BusParams { freq, tseg1: 0, tseg2: 0, sjw: 0, nosamp: 0 };
    match b {
        Bitrate::K10 => pre(CAN_BITRATE_10K),
        Bitrate::K20 => BusParams { freq: 20_000, tseg1: 13, tseg2: 2, sjw: 1, nosamp: 1 },
        Bitrate::K50 => pre(CAN_BITRATE_50K),
        Bitrate::K125 => pre(CAN_BITRATE_125K),
        Bitrate::K250 => pre(CAN_BITRATE_250K),
        Bitrate::K500 => pre(CAN_BITRATE_500K),
        Bitrate::K800 => BusParams { freq: 800_000, tseg1: 7, tseg2: 2, sjw: 1, nosamp: 1 },
        Bitrate::M1 => pre(CAN_BITRATE_1M),
    }
}

/// The flags of canOpenChannel: virtual channels accepted, never exclusive.
pub const OPEN_FLAGS: i32 = CAN_OPEN_ACCEPT_VIRTUAL;

/// The driver type of canSetBusOutputControl.
pub fn driver_type(listen_only: bool) -> u32 {
    if listen_only {
        CAN_DRIVER_SILENT
    } else {
        CAN_DRIVER_NORMAL
    }
}

/// A short text of a status code (used when canGetErrorText gives none).
pub fn error_text(code: i32) -> String {
    let t = match code {
        CAN_OK => "OK",
        CAN_ERR_PARAM => "Error in a parameter.",
        CAN_ERR_NOMSG => "No message.",
        CAN_ERR_NOTFOUND => "The channel or device was not found.",
        CAN_ERR_NOMEM => "Out of memory.",
        CAN_ERR_NOCHANNELS => "No channels available.",
        CAN_ERR_TIMEOUT => "Time-out.",
        CAN_ERR_NOTINITIALIZED => "The library is not initialized.",
        CAN_ERR_NOHANDLES => "No more handles.",
        CAN_ERR_INVHANDLE => "Invalid handle.",
        CAN_ERR_DRIVER => "Driver error.",
        CAN_ERR_TXBUFOFL => "The transmit buffer is full.",
        CAN_ERR_HARDWARE => "Hardware error: the device may be removed.",
        CAN_ERR_DRIVERLOAD => "The driver could not be loaded.",
        CAN_ERR_NOT_IMPLEMENTED => "Not implemented by this device.",
        CAN_ERR_DEVICE_FILE => "Device file error: the device may be removed.",
        _ => return format!("CANlib error {code}"),
    };
    t.to_string()
}

/// The bus state text of the canReadStatus flags; `lost` adds the overrun.
pub fn status_text(flags: u32, lost: bool) -> String {
    let mut s = if flags & CAN_STAT_BUS_OFF != 0 {
        "Bus off".to_string()
    } else if flags & CAN_STAT_ERROR_PASSIVE != 0 {
        "Error passive".to_string()
    } else if flags & CAN_STAT_ERROR_WARNING != 0 {
        "Error warning".to_string()
    } else {
        "OK".to_string()
    };
    if lost || flags & (CAN_STAT_HW_OVERRUN | CAN_STAT_SW_OVERRUN) != 0 {
        s.push_str(", frames lost");
    }
    s
}

/// The canWrite flags of a frame.
pub fn tx_flags(f: &CanFrame) -> u32 {
    let mut fl = if f.extended { CAN_MSG_EXT } else { CAN_MSG_STD };
    if f.rtr {
        fl |= CAN_MSG_RTR;
    }
    fl
}

/// A received message as a frame; `None` for an error frame, a transmit
/// acknowledge or request, and a CAN FD frame. The DLC is limited to 8.
pub fn rx_frame(id: i32, data: &[u8; 8], dlc: u32, flags: u32) -> Option<CanFrame> {
    if flags & (CAN_MSG_ERROR_FRAME | CAN_MSG_TXACK | CAN_MSG_TXRQ | CAN_FDMSG_FDF) != 0 {
        return None;
    }
    let extended = flags & CAN_MSG_EXT != 0;
    let id = id as u32;
    let id_ok = if extended { id <= 0x1FFF_FFFF } else { id <= 0x7FF };
    if !id_ok {
        return None;
    }
    let dlc = dlc.min(8) as u8;
    let rtr = flags & CAN_MSG_RTR != 0;
    let mut d = [0u8; 8];
    if !rtr {
        d[..dlc as usize].copy_from_slice(&data[..dlc as usize]);
    }
    Some(CanFrame { id, extended, rtr, dlc, data: d, timestamp_us: 0, dir: Dir::Rx })
}

// ---- the API seen by the adapter (the DLL, or a fake in the tests) ----

/// A received message (canRead / canReadWait).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawMsg {
    pub id: i32,
    pub data: [u8; 8],
    pub dlc: u32,
    pub flags: u32,
}

/// The CANlib calls the adapter uses. Errors are the canStatus codes.
pub trait KvApi {
    fn channel_count(&mut self) -> Result<i32, i32>;
    fn channel_name(&mut self, ch: i32) -> Result<String, i32>;
    fn open_channel(&mut self, ch: i32, flags: i32) -> Result<i32, i32>;
    fn set_bus_params(&mut self, h: i32, p: BusParams) -> i32;
    fn set_output_control(&mut self, h: i32, driver: u32) -> i32;
    fn bus_on(&mut self, h: i32) -> i32;
    fn bus_off(&mut self, h: i32) -> i32;
    fn close(&mut self, h: i32) -> i32;
    /// Waits at most `timeout_ms`; `Err(CAN_ERR_NOMSG)` when nothing came.
    fn read_wait(&mut self, h: i32, timeout_ms: u32) -> Result<RawMsg, i32>;
    /// Does not wait (`Err(CAN_ERR_NOMSG)` when the queue is empty).
    fn read(&mut self, h: i32) -> Result<RawMsg, i32>;
    fn write(&mut self, h: i32, id: i32, data: &[u8], dlc: u32, flags: u32) -> i32;
    fn read_status(&mut self, h: i32) -> Result<u32, i32>;
    fn error_text(&mut self, code: i32) -> String;
}

// ---- the real DLL: every prototype in one place ----
//
// Source: kvaser.com/canlib-webhelp/canlib_8h.htm (CANSTATUS is an int enum;
// CANLIBAPI is __stdcall = "system"; long and unsigned long are 32 bit on
// Windows, c_long / c_ulong keep the C types).

/// `void canInitializeLibrary(void)`
type FnInitializeLibrary = unsafe extern "system" fn();
/// `canStatus canGetNumberOfChannels(int *channelCount)`
type FnGetNumberOfChannels = unsafe extern "system" fn(*mut i32) -> i32;
/// `canStatus canGetChannelData(int channel, int item, void *buffer, size_t bufsize)`
type FnGetChannelData = unsafe extern "system" fn(i32, i32, *mut c_void, usize) -> i32;
/// `CanHandle canOpenChannel(int channel, int flags)`
type FnOpenChannel = unsafe extern "system" fn(i32, i32) -> i32;
/// `canStatus canSetBusParams(const CanHandle hnd, long freq, unsigned int tseg1,
/// unsigned int tseg2, unsigned int sjw, unsigned int noSamp, unsigned int syncmode)`
type FnSetBusParams = unsafe extern "system" fn(i32, c_long, c_uint, c_uint, c_uint, c_uint, c_uint) -> i32;
/// `canStatus canSetBusOutputControl(const CanHandle hnd, const unsigned int drivertype)`
type FnSetBusOutputControl = unsafe extern "system" fn(i32, c_uint) -> i32;
/// `canStatus canBusOn(const CanHandle hnd)`, `canBusOff`, `canClose`
type FnHandle = unsafe extern "system" fn(i32) -> i32;
/// `canStatus canRead(const CanHandle hnd, long *id, void *msg, unsigned int *dlc,
/// unsigned int *flag, unsigned long *time)`
type FnRead = unsafe extern "system" fn(i32, *mut c_long, *mut c_void, *mut c_uint, *mut c_uint, *mut c_ulong) -> i32;
/// `canStatus canReadWait(const CanHandle hnd, long *id, void *msg, unsigned int *dlc,
/// unsigned int *flag, unsigned long *time, unsigned long timeout)`
type FnReadWait = unsafe extern "system" fn(i32, *mut c_long, *mut c_void, *mut c_uint, *mut c_uint, *mut c_ulong, c_ulong) -> i32;
/// `canStatus canWrite(const CanHandle hnd, long id, void *msg, unsigned int dlc, unsigned int flag)`
type FnWrite = unsafe extern "system" fn(i32, c_long, *mut c_void, c_uint, c_uint) -> i32;
/// `canStatus canReadStatus(const CanHandle hnd, unsigned long *const flags)`
type FnReadStatus = unsafe extern "system" fn(i32, *mut c_ulong) -> i32;
/// `canStatus canGetErrorText(canStatus err, char *buf, unsigned int bufsiz)`
type FnGetErrorText = unsafe extern "system" fn(i32, *mut c_char, c_uint) -> i32;

struct DllApi {
    get_number_of_channels: FnGetNumberOfChannels,
    get_channel_data: FnGetChannelData,
    open_channel: FnOpenChannel,
    set_bus_params: FnSetBusParams,
    set_bus_output_control: FnSetBusOutputControl,
    bus_on: FnHandle,
    bus_off: FnHandle,
    close: FnHandle,
    read: FnRead,
    read_wait: FnReadWait,
    write: FnWrite,
    read_status: FnReadStatus,
    get_error_text: FnGetErrorText,
    // Never unloaded: the function pointers must stay valid while the
    // worker runs.
    _lib: std::mem::ManuallyDrop<libloading::Library>,
}

/// The places where the DLL is looked for: System32, then the folder of the
/// program.
pub fn dll_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(root) = std::env::var_os("SystemRoot") {
        v.push(PathBuf::from(root).join("System32").join(DLL_NAME));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join(DLL_NAME));
        }
    }
    v
}

/// Try the candidates that exist, in order, with `load`. The error is
/// `NOT_FOUND_TEXT`, with the last load error when a file existed.
pub fn find_library<L>(cands: &[PathBuf], mut load: impl FnMut(&PathBuf) -> Result<L, String>) -> Result<L, CanError> {
    let mut last: Option<String> = None;
    for p in cands.iter().filter(|p| p.is_file()) {
        match load(p) {
            Ok(l) => return Ok(l),
            Err(e) => last = Some(format!("{}: {e}", p.display())),
        }
    }
    Err(CanError::new(match last {
        Some(e) => format!("{NOT_FOUND_TEXT} ({e})"),
        None => NOT_FOUND_TEXT.to_string(),
    }))
}

impl DllApi {
    fn load() -> Result<DllApi, CanError> {
        // SAFETY: loading the DLL runs its DllMain; the candidates are fixed
        // paths of the Kvaser CANlib.
        let lib = find_library(&dll_candidates(), |p| unsafe { libloading::Library::new(p) }.map_err(|e| e.to_string()))?;
        fn sym<T: Copy>(lib: &libloading::Library, name: &[u8]) -> Result<T, CanError> {
            // SAFETY: each `T` below is the prototype of `name` in canlib.h.
            unsafe { lib.get::<T>(name).map(|s| *s) }.map_err(|e| CanError::new(format!("{}: {e}", String::from_utf8_lossy(&name[..name.len() - 1]))))
        }
        let init: FnInitializeLibrary = sym(&lib, b"canInitializeLibrary\0")?;
        let api = DllApi {
            get_number_of_channels: sym(&lib, b"canGetNumberOfChannels\0")?,
            get_channel_data: sym(&lib, b"canGetChannelData\0")?,
            open_channel: sym(&lib, b"canOpenChannel\0")?,
            set_bus_params: sym(&lib, b"canSetBusParams\0")?,
            set_bus_output_control: sym(&lib, b"canSetBusOutputControl\0")?,
            bus_on: sym(&lib, b"canBusOn\0")?,
            bus_off: sym(&lib, b"canBusOff\0")?,
            close: sym(&lib, b"canClose\0")?,
            read: sym(&lib, b"canRead\0")?,
            read_wait: sym(&lib, b"canReadWait\0")?,
            write: sym(&lib, b"canWrite\0")?,
            read_status: sym(&lib, b"canReadStatus\0")?,
            get_error_text: sym(&lib, b"canGetErrorText\0")?,
            _lib: std::mem::ManuallyDrop::new(lib),
        };
        // SAFETY: no arguments; must be called before any other CANlib call.
        unsafe { init() };
        Ok(api)
    }

    fn read_with(&mut self, h: i32, wait: Option<u32>) -> Result<RawMsg, i32> {
        let mut id: c_long = 0;
        let mut data = [0u8; 64];
        let mut dlc: c_uint = 0;
        let mut flags: c_uint = 0;
        let mut time: c_ulong = 0;
        // SAFETY: valid out pointers; the data buffer holds 64 bytes, the
        // largest CAN FD payload, so no message can overrun it.
        let r = unsafe {
            match wait {
                Some(ms) => (self.read_wait)(h, &mut id, data.as_mut_ptr().cast(), &mut dlc, &mut flags, &mut time, ms as c_ulong),
                None => (self.read)(h, &mut id, data.as_mut_ptr().cast(), &mut dlc, &mut flags, &mut time),
            }
        };
        if r != CAN_OK {
            return Err(r);
        }
        let mut d = [0u8; 8];
        d.copy_from_slice(&data[..8]);
        Ok(RawMsg { id: id as i32, data: d, dlc: dlc as u32, flags: flags as u32 })
    }
}

impl KvApi for DllApi {
    fn channel_count(&mut self) -> Result<i32, i32> {
        let mut n = 0i32;
        // SAFETY: valid out pointer.
        let r = unsafe { (self.get_number_of_channels)(&mut n) };
        if r == CAN_OK {
            Ok(n)
        } else {
            Err(r)
        }
    }

    fn channel_name(&mut self, ch: i32) -> Result<String, i32> {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer and its size match.
        let r = unsafe { (self.get_channel_data)(ch, CAN_CHANNELDATA_CHANNEL_NAME, buf.as_mut_ptr().cast(), buf.len()) };
        if r != CAN_OK {
            return Err(r);
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Ok(String::from_utf8_lossy(&buf[..end]).trim().to_string())
    }

    fn open_channel(&mut self, ch: i32, flags: i32) -> Result<i32, i32> {
        // SAFETY: plain values.
        let h = unsafe { (self.open_channel)(ch, flags) };
        if h >= 0 {
            Ok(h)
        } else {
            Err(h)
        }
    }

    fn set_bus_params(&mut self, h: i32, p: BusParams) -> i32 {
        // SAFETY: plain values.
        unsafe { (self.set_bus_params)(h, p.freq as c_long, p.tseg1, p.tseg2, p.sjw, p.nosamp, 0) }
    }

    fn set_output_control(&mut self, h: i32, driver: u32) -> i32 {
        // SAFETY: plain values.
        unsafe { (self.set_bus_output_control)(h, driver) }
    }

    fn bus_on(&mut self, h: i32) -> i32 {
        // SAFETY: plain value.
        unsafe { (self.bus_on)(h) }
    }

    fn bus_off(&mut self, h: i32) -> i32 {
        // SAFETY: plain value.
        unsafe { (self.bus_off)(h) }
    }

    fn close(&mut self, h: i32) -> i32 {
        // SAFETY: plain value.
        unsafe { (self.close)(h) }
    }

    fn read_wait(&mut self, h: i32, timeout_ms: u32) -> Result<RawMsg, i32> {
        self.read_with(h, Some(timeout_ms))
    }

    fn read(&mut self, h: i32) -> Result<RawMsg, i32> {
        self.read_with(h, None)
    }

    fn write(&mut self, h: i32, id: i32, data: &[u8], dlc: u32, flags: u32) -> i32 {
        let mut d = [0u8; 8];
        let n = data.len().min(8);
        d[..n].copy_from_slice(&data[..n]);
        // SAFETY: the buffer holds 8 bytes and the DLC is at most 8; canWrite
        // copies the data before it returns.
        unsafe { (self.write)(h, id as c_long, d.as_mut_ptr().cast(), dlc, flags) }
    }

    fn read_status(&mut self, h: i32) -> Result<u32, i32> {
        let mut f: c_ulong = 0;
        // SAFETY: valid out pointer.
        let r = unsafe { (self.read_status)(h, &mut f) };
        if r == CAN_OK {
            Ok(f as u32)
        } else {
            Err(r)
        }
    }

    fn error_text(&mut self, code: i32) -> String {
        let mut buf = [0u8; 128];
        // SAFETY: the buffer and its size match.
        let r = unsafe { (self.get_error_text)(code, buf.as_mut_ptr().cast(), buf.len() as c_uint) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let t = String::from_utf8_lossy(&buf[..end]).trim().to_string();
        if r == CAN_OK && !t.is_empty() {
            format!("{t} ({code})")
        } else {
            error_text(code)
        }
    }
}

// ---- the worker ----

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

fn global_dead() -> Arc<AtomicBool> {
    static DEAD: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    DEAD.get_or_init(|| Arc::new(AtomicBool::new(false))).clone()
}

struct Core {
    api: Box<dyn KvApi>,
    handle: Option<i32>,
}

impl Core {
    fn close_handle(&mut self) {
        if let Some(h) = self.handle.take() {
            self.api.bus_off(h);
            self.api.close(h);
        }
    }
}

type Job = Box<dyn FnOnce(&mut Core) + Send>;
type Loader = Box<dyn FnOnce() -> Result<Box<dyn KvApi>, CanError> + Send>;

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
    while let Ok(job) = jobs.recv() {
        job(&mut core);
    }
    core.close_handle();
}

fn open_channel(core: &mut Core, ch: i32, p: BusParams, listen_only: bool) -> Result<(), CanError> {
    core.close_handle();
    let h = core.api.open_channel(ch, OPEN_FLAGS).map_err(|r| CanError::new(format!("Kvaser channel {ch}: {}", core.api.error_text(r))))?;
    let fail = |core: &mut Core, what: &str, r: i32| -> CanError {
        core.api.close(h);
        CanError::new(format!("Kvaser channel {ch}: {what}: {}", core.api.error_text(r)))
    };
    let r = core.api.set_bus_params(h, p);
    if r != CAN_OK {
        return Err(fail(core, "bitrate", r));
    }
    let r = core.api.set_output_control(h, driver_type(listen_only));
    if r != CAN_OK {
        return Err(fail(core, if listen_only { "listen-only (silent) mode" } else { "driver mode" }, r));
    }
    let r = core.api.bus_on(h);
    if r != CAN_OK {
        return Err(fail(core, "bus on", r));
    }
    core.handle = Some(h);
    Ok(())
}

/// The longest batch of one receive call.
const READ_BATCH: usize = 64;

struct Batch {
    frames: Vec<CanFrame>,
    lost: bool,
}

fn read_batch(core: &mut Core, timeout: Duration) -> Result<Batch, CanError> {
    let h = core.handle.ok_or_else(|| CanError::new("channel not open"))?;
    let mut b = Batch { frames: Vec::new(), lost: false };
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let mut first = Some(ms);
    loop {
        let r = match first.take() {
            Some(ms) => core.api.read_wait(h, ms),
            None => core.api.read(h),
        };
        match r {
            Ok(m) => {
                b.lost |= m.flags & CAN_MSGERR_OVERRUN != 0;
                if let Some(f) = rx_frame(m.id, &m.data, m.dlc, m.flags) {
                    b.frames.push(f);
                }
                if b.frames.len() >= READ_BATCH {
                    return Ok(b);
                }
            }
            Err(CAN_ERR_NOMSG) | Err(CAN_ERR_TIMEOUT) => return Ok(b),
            Err(r) => {
                if b.frames.is_empty() {
                    return Err(CanError::new(format!("Kvaser: {}", core.api.error_text(r))));
                }
                return Ok(b);
            }
        }
    }
}

/// A Kvaser interface through CANlib.
pub struct KvaserAdapter {
    jobs: Option<Sender<Job>>,
    dead: Arc<AtomicBool>,
    t: Timeouts,
    open: bool,
    listen_only: bool,
    lost: bool,
    rx: VecDeque<CanFrame>,
}

impl KvaserAdapter {
    /// Load canlib32.dll on a new worker thread. The error is
    /// `NOT_FOUND_TEXT` without the DLL, `DEAD_TEXT` after a hang.
    pub fn load() -> Result<KvaserAdapter, CanError> {
        KvaserAdapter::with_loader(Box::new(|| DllApi::load().map(|a| Box::new(a) as Box<dyn KvApi>)), Timeouts::default(), global_dead())
    }

    fn with_loader(loader: Loader, t: Timeouts, dead: Arc<AtomicBool>) -> Result<KvaserAdapter, CanError> {
        if dead.load(Ordering::SeqCst) {
            return Err(CanError::new(DEAD_TEXT));
        }
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (loaded_tx, loaded_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("kvaser-canlib".to_string())
            .spawn(move || worker(loader, loaded_tx, job_rx))
            .map_err(|e| CanError::new(format!("Kvaser driver thread: {e}")))?;
        match loaded_rx.recv_timeout(t.call) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(RecvTimeoutError::Timeout) => {
                dead.store(true, Ordering::SeqCst);
                return Err(CanError::new(DEAD_TEXT));
            }
            Err(RecvTimeoutError::Disconnected) => return Err(CanError::new("Kvaser driver thread ended")),
        }
        Ok(KvaserAdapter { jobs: Some(job_tx), dead, t, open: false, listen_only: false, lost: false, rx: VecDeque::new() })
    }

    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    fn call<R: Send + 'static>(&mut self, wait: Duration, f: impl FnOnce(&mut Core) -> R + Send + 'static) -> Result<R, CanError> {
        if self.is_dead() {
            return Err(CanError::new(DEAD_TEXT));
        }
        let jobs = self.jobs.as_ref().ok_or_else(|| CanError::new("Kvaser driver thread ended"))?;
        let (tx, rx) = mpsc::sync_channel::<R>(1);
        let job: Job = Box::new(move |core: &mut Core| {
            let _ = tx.send(f(core));
        });
        if jobs.send(job).is_err() {
            return Err(CanError::new("Kvaser driver thread ended"));
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
                Err(CanError::new("Kvaser driver thread ended"))
            }
        }
    }
}

fn list(core: &mut Core) -> Result<Vec<ChannelInfo>, CanError> {
    let n = core.api.channel_count().map_err(|r| CanError::new(format!("Kvaser: {}", core.api.error_text(r))))?;
    let mut v = Vec::new();
    for ch in 0..n.max(0) {
        let name = match core.api.channel_name(ch) {
            Ok(s) if !s.is_empty() => format!("{ch}: {s}"),
            _ => format!("Kvaser channel {ch}"),
        };
        v.push(ChannelInfo { handle: ch as u32, name, available: true });
    }
    Ok(v)
}

impl CanAdapter for KvaserAdapter {
    fn name(&self) -> String {
        "Kvaser".to_string()
    }

    fn list_channels(&mut self) -> Result<Vec<ChannelInfo>, CanError> {
        let t = self.t.call;
        self.call(t, list)?
    }

    fn open(&mut self, channel: u32, bitrate: Bitrate, listen_only: bool) -> Result<(), CanError> {
        self.close();
        let ch = i32::try_from(channel).map_err(|_| CanError::new("invalid Kvaser channel number"))?;
        let p = bus_params(bitrate);
        let t = self.t.call;
        self.call(t, move |core| open_channel(core, ch, p, listen_only))??;
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
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Box::new(|core: &mut Core| core.close_handle()));
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
        let (id, data, dlc, flags) = (frame.id as i32, frame.data, frame.dlc as u32, tx_flags(frame));
        let t = self.t.call;
        let r = self.call(t, move |core| match core.handle {
            None => Err(CAN_ERR_INVHANDLE.to_string()),
            Some(h) => match core.api.write(h, id, &data, dlc, flags) {
                CAN_OK => Ok(()),
                r => Err(core.api.error_text(r)),
            },
        })?;
        r.map_err(|e| CanError::new(format!("Kvaser: {e}")))
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
            None => Err(error_text(CAN_ERR_INVHANDLE)),
            Some(h) => core.api.read_status(h).map_err(|r| core.api.error_text(r)),
        }) {
            Ok(Ok(f)) => status_text(f, lost),
            Ok(Err(e)) => e,
            Err(e) => e.0,
        }
    }
}

impl Drop for KvaserAdapter {
    fn drop(&mut self) {
        self.close();
        self.jobs = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn bitrate_constants() {
        assert_eq!(bus_params(Bitrate::M1).freq, -1);
        assert_eq!(bus_params(Bitrate::K500).freq, -2);
        assert_eq!(bus_params(Bitrate::K250).freq, -3);
        assert_eq!(bus_params(Bitrate::K125).freq, -4);
        assert_eq!(bus_params(Bitrate::K50).freq, -7);
        assert_eq!(bus_params(Bitrate::K10).freq, -9);
        assert_eq!(CAN_BITRATE_100K, -5);
        let p = bus_params(Bitrate::K20);
        assert_eq!((p.freq, 1 + p.tseg1 + p.tseg2), (20_000, 16));
        let p = bus_params(Bitrate::K800);
        assert_eq!((p.freq, 1 + p.tseg1 + p.tseg2), (800_000, 10));
        for b in Bitrate::ALL {
            let p = bus_params(b);
            assert!(p.freq < 0 || (p.sjw >= 1 && p.nosamp == 1 && p.tseg2 >= p.sjw), "{b:?}");
        }
    }

    #[test]
    fn open_flags_and_driver_type() {
        assert_eq!(OPEN_FLAGS & CAN_OPEN_ACCEPT_VIRTUAL, CAN_OPEN_ACCEPT_VIRTUAL);
        assert_eq!(OPEN_FLAGS & CAN_OPEN_EXCLUSIVE, 0);
        assert_eq!(driver_type(true), CAN_DRIVER_SILENT);
        assert_eq!(driver_type(false), CAN_DRIVER_NORMAL);
    }

    #[test]
    fn message_flag_conversion() {
        let mut f = CanFrame::new(0x123, &[1, 2]);
        assert_eq!(tx_flags(&f), CAN_MSG_STD);
        f.extended = true;
        f.rtr = true;
        assert_eq!(tx_flags(&f), CAN_MSG_EXT | CAN_MSG_RTR);

        let data = [1, 2, 3, 4, 5, 6, 7, 8];
        let r = rx_frame(0x123, &data, 3, CAN_MSG_STD).unwrap();
        assert_eq!((r.id, r.extended, r.rtr, r.payload()), (0x123, false, false, &[1u8, 2, 3][..]));
        let r = rx_frame(0x1234_5678, &data, 15, CAN_MSG_EXT).unwrap();
        assert!(r.extended);
        assert_eq!(r.dlc, 8);
        let r = rx_frame(0x10, &data, 4, CAN_MSG_STD | CAN_MSG_RTR).unwrap();
        assert!(r.rtr);
        assert_eq!(r.dlc, 4);
        assert_eq!(r.data, [0; 8]);
        for skip in [CAN_MSG_ERROR_FRAME, CAN_MSG_TXACK, CAN_MSG_TXRQ, CAN_FDMSG_FDF] {
            assert!(rx_frame(0x123, &data, 1, CAN_MSG_STD | skip).is_none());
        }
        assert!(rx_frame(0x800, &data, 1, CAN_MSG_STD).is_none());
    }

    #[test]
    fn status_and_error_texts() {
        assert_eq!(status_text(CAN_STAT_ERROR_ACTIVE, false), "OK");
        assert_eq!(status_text(CAN_STAT_BUS_OFF | CAN_STAT_ERROR_PASSIVE, false), "Bus off");
        assert_eq!(status_text(CAN_STAT_ERROR_PASSIVE, false), "Error passive");
        assert_eq!(status_text(CAN_STAT_ERROR_WARNING, true), "Error warning, frames lost");
        assert_eq!(status_text(CAN_STAT_SW_OVERRUN, false), "OK, frames lost");
        assert!(error_text(CAN_ERR_NOTFOUND).contains("not found"));
        assert_eq!(error_text(-99), "CANlib error -99");
    }

    #[test]
    fn missing_dll_message() {
        assert_eq!(NOT_FOUND_TEXT, "Kvaser driver not found: install the Kvaser drivers");
        let dir = std::env::temp_dir().join(format!("canoxide_kv_missing_{}", std::process::id()));
        let none = vec![dir.join(DLL_NAME)];
        let mut called = false;
        let r = find_library(&none, |_| {
            called = true;
            Ok::<(), String>(())
        });
        assert!(!called, "a path that does not exist is not loaded");
        assert_eq!(r.unwrap_err().0, NOT_FOUND_TEXT);
        // A file that exists but does not load: the reason is added.
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(DLL_NAME);
        std::fs::write(&p, b"not a dll").unwrap();
        let r = find_library(&[p], |_| Err::<(), String>("bad image".to_string()));
        let e = r.unwrap_err().0;
        assert!(e.starts_with(NOT_FOUND_TEXT) && e.contains("bad image"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
        let c = dll_candidates();
        assert!(c.iter().all(|p| p.file_name().unwrap() == DLL_NAME));
    }

    // ---- a fake CANlib ----

    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
        rx: VecDeque<RawMsg>,
        fail_params: Option<i32>,
        status: u32,
        hang_read: bool,
    }

    struct FakeApi(Arc<Mutex<Fake>>);

    impl KvApi for FakeApi {
        fn channel_count(&mut self) -> Result<i32, i32> {
            Ok(2)
        }
        fn channel_name(&mut self, ch: i32) -> Result<String, i32> {
            if ch == 0 {
                Ok("Kvaser Virtual CAN Driver (channel 0)".to_string())
            } else {
                Err(CAN_ERR_PARAM)
            }
        }
        fn open_channel(&mut self, ch: i32, flags: i32) -> Result<i32, i32> {
            self.0.lock().unwrap().log.push(format!("open {ch} {flags:#x}"));
            Ok(7)
        }
        fn set_bus_params(&mut self, h: i32, p: BusParams) -> i32 {
            let mut s = self.0.lock().unwrap();
            s.log.push(format!("params {h} {}", p.freq));
            s.fail_params.unwrap_or(CAN_OK)
        }
        fn set_output_control(&mut self, h: i32, driver: u32) -> i32 {
            self.0.lock().unwrap().log.push(format!("output {h} {driver}"));
            CAN_OK
        }
        fn bus_on(&mut self, h: i32) -> i32 {
            self.0.lock().unwrap().log.push(format!("bus_on {h}"));
            CAN_OK
        }
        fn bus_off(&mut self, h: i32) -> i32 {
            self.0.lock().unwrap().log.push(format!("bus_off {h}"));
            CAN_OK
        }
        fn close(&mut self, h: i32) -> i32 {
            self.0.lock().unwrap().log.push(format!("close {h}"));
            CAN_OK
        }
        fn read_wait(&mut self, h: i32, _timeout_ms: u32) -> Result<RawMsg, i32> {
            if self.0.lock().unwrap().hang_read {
                std::thread::sleep(Duration::from_secs(5));
            }
            self.read(h)
        }
        fn read(&mut self, _h: i32) -> Result<RawMsg, i32> {
            self.0.lock().unwrap().rx.pop_front().ok_or(CAN_ERR_NOMSG)
        }
        fn write(&mut self, h: i32, id: i32, data: &[u8], dlc: u32, flags: u32) -> i32 {
            self.0.lock().unwrap().log.push(format!("write {h} {id:#x} {:?} {dlc} {flags:#x}", &data[..dlc as usize]));
            CAN_OK
        }
        fn read_status(&mut self, _h: i32) -> Result<u32, i32> {
            Ok(self.0.lock().unwrap().status)
        }
        fn error_text(&mut self, code: i32) -> String {
            error_text(code)
        }
    }

    fn fast() -> Timeouts {
        Timeouts { call: Duration::from_millis(500), receive_margin: Duration::from_millis(200) }
    }

    fn adapter() -> (KvaserAdapter, Arc<Mutex<Fake>>) {
        let s = Arc::new(Mutex::new(Fake::default()));
        let s2 = s.clone();
        let a = KvaserAdapter::with_loader(Box::new(move || Ok(Box::new(FakeApi(s2)) as Box<dyn KvApi>)), fast(), Arc::new(AtomicBool::new(false))).unwrap();
        (a, s)
    }

    fn log(s: &Arc<Mutex<Fake>>) -> Vec<String> {
        s.lock().unwrap().log.clone()
    }

    #[test]
    fn open_sequence_sends_nothing_and_accepts_virtual() {
        let (mut a, s) = adapter();
        a.open(0, Bitrate::K250, false).unwrap();
        assert_eq!(log(&s), vec!["open 0 0x20", "params 7 -3", "output 7 4", "bus_on 7"]);
        assert_eq!(a.status(), "OK");
        a.close();
        a.list_channels().unwrap();
        assert_eq!(&log(&s)[4..], ["bus_off 7", "close 7"]);
    }

    #[test]
    fn listen_only_is_silent_and_refuses_send() {
        let (mut a, s) = adapter();
        a.open(1, Bitrate::K125, true).unwrap();
        assert!(log(&s).contains(&"output 7 1".to_string()));
        let e = a.send(&CanFrame::new(0x123, &[1])).unwrap_err();
        assert!(e.0.contains("listen-only"));
        assert!(!log(&s).iter().any(|l| l.starts_with("write")));
    }

    #[test]
    fn a_failed_open_closes_the_handle() {
        let (mut a, s) = adapter();
        s.lock().unwrap().fail_params = Some(CAN_ERR_PARAM);
        let e = a.open(0, Bitrate::K500, false).unwrap_err();
        assert!(e.0.contains("bitrate"), "{e}");
        assert_eq!(log(&s).last().unwrap(), "close 7");
        assert!(a.send(&CanFrame::new(1, &[])).is_err());
    }

    #[test]
    fn send_receive_status_and_channels() {
        let (mut a, s) = adapter();
        a.open(0, Bitrate::M1, false).unwrap();
        a.send(&CanFrame::new(0x601, &[0x40, 0x18, 0x10, 0x01])).unwrap();
        assert!(log(&s).contains(&"write 7 0x601 [64, 24, 16, 1] 4 0x2".to_string()));
        {
            let mut f = s.lock().unwrap();
            f.rx.push_back(RawMsg { id: 0x581, data: [0x43, 0, 0, 0, 0, 0, 0, 0], dlc: 8, flags: CAN_MSG_STD });
            f.rx.push_back(RawMsg { id: 0, data: [0; 8], dlc: 0, flags: CAN_MSG_ERROR_FRAME });
            f.rx.push_back(RawMsg { id: 0x182, data: [1; 8], dlc: 1, flags: CAN_MSG_STD | CAN_MSGERR_OVERRUN });
        }
        assert_eq!(a.receive(Duration::from_millis(10)).unwrap().unwrap().id, 0x581);
        assert_eq!(a.receive(Duration::from_millis(10)).unwrap().unwrap().id, 0x182);
        assert!(a.receive(Duration::from_millis(10)).unwrap().is_none());
        s.lock().unwrap().status = CAN_STAT_BUS_OFF;
        assert_eq!(a.status(), "Bus off, frames lost");
        let ch = a.list_channels().unwrap();
        assert_eq!(ch.len(), 2);
        assert!(ch[0].name.contains("Virtual"));
        assert_eq!(ch[1].name, "Kvaser channel 1");
    }

    #[test]
    fn a_hang_makes_the_adapter_dead() {
        let (mut a, s) = adapter();
        a.open(0, Bitrate::K250, false).unwrap();
        s.lock().unwrap().hang_read = true;
        assert_eq!(a.receive(Duration::from_millis(10)).unwrap_err().0, DEAD_TEXT);
        assert_eq!(a.status(), DEAD_TEXT);
        assert_eq!(a.send(&CanFrame::new(1, &[])).unwrap_err().0, DEAD_TEXT);
    }

    #[test]
    fn a_load_error_is_returned() {
        let r = KvaserAdapter::with_loader(Box::new(|| Err(CanError::new(NOT_FOUND_TEXT))), fast(), Arc::new(AtomicBool::new(false)));
        assert_eq!(r.err().unwrap().0, NOT_FOUND_TEXT);
    }

    /// Loads the real canlib32.dll and opens a VIRTUAL channel only. Runs
    /// only with `CANOXIDE_TEST_KVASER=1` and `--ignored`.
    #[test]
    #[ignore]
    fn real_canlib_virtual_channel() {
        if std::env::var("CANOXIDE_TEST_KVASER").as_deref() != Ok("1") {
            eprintln!("skipped: set CANOXIDE_TEST_KVASER=1");
            return;
        }
        let mut a = KvaserAdapter::load().expect("canlib32.dll");
        let ch = a.list_channels().unwrap();
        let Some(v) = ch.iter().find(|c| c.name.to_ascii_lowercase().contains("virtual")) else {
            eprintln!("skipped: no virtual channel");
            return;
        };
        a.open(v.handle, Bitrate::K500, true).unwrap();
        let _ = a.receive(Duration::from_millis(50));
        a.status();
        a.close();
    }
}
