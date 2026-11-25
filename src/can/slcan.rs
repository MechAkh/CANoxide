//! SLCAN backend: the Lawicel ASCII protocol over a COM port or a TCP
//! stream. Covers CANable / candleLight in slcan firmware, the Lawicel
//! CANUSB, the USBtin and network gateways that speak the same protocol over
//! TCP. Written from public documentation only and NOT TESTED ON HARDWARE.
//!
//! Sources: the LAWICEL CANUSB user manual (canusb.com/files/canusb_manual.pdf,
//! the command list, "1.5 CAN Driver Design Guide"), the CAN232 v3 manual
//! (canusb.com/files/can232_v3.pdf, `L` listen only) and the README of the
//! CANable firmwares (github.com/normaldotcom/canable-fw and canable2-fw).
//!
//! Protocol: a command ends with CR. A reply is CR (OK) or BEL (0x07,
//! error); `t`/`T`/`r`/`R` are answered with `z` CR / `Z` CR. The CANable
//! firmwares send no reply at all, so a missing reply is not an error
//! (except for `V`, which every firmware answers). Received frames come as
//! the same `t`/`T`/`r`/`R` lines, with a 4-digit millisecond time stamp at
//! the end when the device has time stamps on (`Z1`, kept in the EEPROM of a
//! Lawicel device: this module never sends `Z`).
//!
//! Opening sends `C` (its answer is ignored: it empties the command queue of
//! the device), `V` (the device must answer), `S<n>`, then `O` (or `L` for
//! listen only). No CAN frame is sent. `F` (status flags) is asked for by
//! `status()` only on a device that answers commands.
//!
//! The COM transport uses the Win32 API (windows-sys); it is thin and has no
//! automatic test. The TCP transport is tested against a fake device.

use super::{Bitrate, CanAdapter, CanError, CanFrame, ChannelInfo, Dir};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// Where the SLCAN device is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlcanTarget {
    /// A COM port name, for example `COM5`.
    Serial(String),
    /// `host:port` of a network gateway.
    Tcp(String),
}

impl SlcanTarget {
    /// `COM<n>` (1..=255) is a COM port; text with a colon and a port
    /// number (1..=65535) after the last colon is `host:port`.
    pub fn parse(s: &str) -> Result<SlcanTarget, String> {
        let t = s.trim();
        if let Some((host, port)) = t.rsplit_once(':') {
            let host = host.trim();
            return match port.trim().parse::<u16>() {
                Ok(p) if p > 0 && !host.is_empty() && !host.contains(char::is_whitespace) => Ok(SlcanTarget::Tcp(format!("{host}:{p}"))),
                _ => Err(format!("'{s}' is not host:port (port 1..65535)")),
            };
        }
        if let Some(n) = com_number(t) {
            return Ok(SlcanTarget::Serial(format!("COM{n}")));
        }
        Err(format!("'{s}' is not a COM port (COM1..COM255) or host:port"))
    }

    pub fn label(&self) -> &str {
        match self {
            SlcanTarget::Serial(s) | SlcanTarget::Tcp(s) => s,
        }
    }
}

/// The number of `COM<n>` (case does not matter), 1..=255.
pub fn com_number(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.len() < 4 || !t[..3].eq_ignore_ascii_case("COM") {
        return None;
    }
    let digits = &t[3..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|n| (1..=255).contains(n))
}

/// The digit of the `S<n>` command (Lawicel: S0 10k .. S8 1M). Every
/// bitrate of the tool has a code; S3 (100k) is not offered by the tool.
/// The CANable firmwares use S7 for 750 kbit/s, not 800 kbit/s.
pub fn bitrate_code(b: Bitrate) -> char {
    match b {
        Bitrate::K10 => '0',
        Bitrate::K20 => '1',
        Bitrate::K50 => '2',
        Bitrate::K125 => '4',
        Bitrate::K250 => '5',
        Bitrate::K500 => '6',
        Bitrate::K800 => '7',
        Bitrate::M1 => '8',
    }
}

/// The transmit line of a frame, CR included: `tIIIL DD..`, `TIIIIIIIIL DD..`,
/// `rIIIL`, `RIIIIIIIIL` (no spaces; upper-case hex).
pub fn encode_frame(f: &CanFrame) -> Result<String, CanError> {
    f.validate()?;
    let mut s = String::with_capacity(28);
    let kind = match (f.extended, f.rtr) {
        (false, false) => 't',
        (true, false) => 'T',
        (false, true) => 'r',
        (true, true) => 'R',
    };
    s.push(kind);
    if f.extended {
        s.push_str(&format!("{:08X}", f.id));
    } else {
        s.push_str(&format!("{:03X}", f.id));
    }
    s.push(char::from(b'0' + f.dlc));
    if !f.rtr {
        for b in &f.data[..f.dlc as usize] {
            s.push_str(&format!("{b:02X}"));
        }
    }
    s.push('\r');
    Ok(s)
}

/// One element of the byte stream from the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// An empty line: CR alone (the OK reply).
    Ok,
    /// BEL: the device refused a command.
    Bel,
    /// A line without its CR.
    Line(String),
    /// A line longer than `MAX_LINE`, dropped.
    TooLong,
}

/// The longest line kept (an extended frame with 8 bytes and a time stamp
/// is 30 characters).
pub const MAX_LINE: usize = 64;

/// Splits the byte stream into tokens; keeps a partial line until its CR
/// comes. LF is ignored (some devices send CR LF).
#[derive(Debug, Default)]
pub struct LineReader {
    buf: Vec<u8>,
    too_long: bool,
}

impl LineReader {
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<Token>) {
        for &b in bytes {
            match b {
                0x07 => out.push(Token::Bel),
                b'\r' => {
                    if self.too_long {
                        out.push(Token::TooLong);
                    } else if self.buf.is_empty() {
                        out.push(Token::Ok);
                    } else {
                        out.push(Token::Line(String::from_utf8_lossy(&self.buf).into_owned()));
                    }
                    self.buf.clear();
                    self.too_long = false;
                }
                b'\n' => {}
                _ => {
                    if self.buf.len() >= MAX_LINE {
                        self.too_long = true;
                    } else {
                        self.buf.push(b);
                    }
                }
            }
        }
    }

    /// Characters of a line that has no CR yet.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

/// The meaning of one line from the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// A received frame; `timestamp_ms` when the device has time stamps on
    /// (Lawicel: 0..59999 ms, then it starts at 0 again).
    Frame { frame: CanFrame, timestamp_ms: Option<u16> },
    /// `z` / `Z`: a frame was queued for sending.
    TxAck,
    /// `Fxx`: the status flags.
    Flags(u8),
    /// `V...`: the version.
    Version(String),
    /// `N...`: the serial number.
    Serial(String),
    /// Anything else (counted, not an error).
    Unknown(String),
}

fn hex(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(s, 16).ok()
}

fn parse_frame(line: &str) -> Option<(CanFrame, Option<u16>)> {
    let b = line.as_bytes();
    let (extended, rtr) = match b.first()? {
        b't' => (false, false),
        b'T' => (true, false),
        b'r' => (false, true),
        b'R' => (true, true),
        _ => return None,
    };
    if !line.is_ascii() {
        return None;
    }
    let id_len = if extended { 8 } else { 3 };
    let id = hex(line.get(1..1 + id_len)?)?;
    let dlc_ch = *b.get(1 + id_len)?;
    if !dlc_ch.is_ascii_digit() {
        return None;
    }
    let dlc = dlc_ch - b'0';
    if dlc > 8 {
        return None;
    }
    let mut pos = 2 + id_len;
    let mut data = [0u8; 8];
    if !rtr {
        for d in data.iter_mut().take(dlc as usize) {
            *d = hex(line.get(pos..pos + 2)?)? as u8;
            pos += 2;
        }
    }
    let rest = &line[pos..];
    let timestamp_ms = match rest.len() {
        0 => None,
        4 => Some(hex(rest)? as u16),
        _ => return None,
    };
    let frame = CanFrame { id, extended, rtr, dlc, data, timestamp_us: 0, dir: Dir::Rx };
    frame.validate().ok()?;
    Some((frame, timestamp_ms))
}

/// Decode one line (without its CR).
pub fn parse_line(line: &str) -> Reply {
    if let Some((frame, timestamp_ms)) = parse_frame(line) {
        return Reply::Frame { frame, timestamp_ms };
    }
    match line {
        "z" | "Z" => return Reply::TxAck,
        _ => {}
    }
    if let Some(rest) = line.strip_prefix('F') {
        if rest.len() == 2 {
            if let Some(v) = hex(rest) {
                return Reply::Flags(v as u8);
            }
        }
    }
    if let Some(rest) = line.strip_prefix('V') {
        return Reply::Version(rest.to_string());
    }
    if let Some(rest) = line.strip_prefix('N') {
        return Reply::Serial(rest.to_string());
    }
    Reply::Unknown(line.to_string())
}

/// The text of the `F` status flags (CANUSB manual, command F).
pub fn flags_text(flags: u8) -> String {
    let names: [(u8, &str); 7] = [
        (0x80, "Bus error"),
        (0x20, "Error passive"),
        (0x04, "Error warning"),
        (0x08, "Data overrun"),
        (0x01, "Receive FIFO full"),
        (0x02, "Transmit FIFO full"),
        (0x40, "Arbitration lost"),
    ];
    let parts: Vec<&str> = names.iter().filter(|(b, _)| flags & b != 0).map(|(_, t)| *t).collect();
    if parts.is_empty() {
        "OK".to_string()
    } else {
        parts.join(", ")
    }
}

// ---- transports ----

/// A byte stream to the device.
pub trait Transport: Send {
    /// Read what is there, waiting at most about `wait` (one short wait of
    /// the transport when `wait` is zero). `Ok(0)`: nothing came. An error
    /// means the connection is lost.
    fn read(&mut self, buf: &mut [u8], wait: Duration) -> io::Result<usize>;
    /// Write all bytes. `TimedOut` when the device does not take them.
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
}

/// The connect time-out of the TCP transport.
pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// The longest wait for the device to take written bytes.
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);

/// TCP, non-blocking after the connect.
pub struct TcpTransport {
    s: TcpStream,
}

impl TcpTransport {
    pub fn connect(addr: &str) -> io::Result<TcpTransport> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "the name gives no address");
        for a in addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&a, TCP_CONNECT_TIMEOUT) {
                Ok(s) => {
                    s.set_nodelay(true)?;
                    s.set_nonblocking(true)?;
                    return Ok(TcpTransport { s });
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

impl Transport for TcpTransport {
    fn read(&mut self, buf: &mut [u8], wait: Duration) -> io::Result<usize> {
        let end = Instant::now() + wait;
        loop {
            match self.s.read(buf) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed by the other side")),
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= end {
                        return Ok(0);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn write_all(&mut self, mut data: &[u8]) -> io::Result<()> {
        let end = Instant::now() + WRITE_TIMEOUT;
        while !data.is_empty() {
            match self.s.write(data) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "closed by the other side")),
                Ok(n) => data = &data[n..],
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= end {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "the device does not take the data"));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
mod com {
    //! The COM port through the Win32 API. 115200 8N1 (a USB CDC device
    //! ignores it), DTR and RTS on, no flow control; a read returns at once
    //! with the bytes that are there or after at most 10 ms.

    use super::Transport;
    use std::io;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Devices::Communication::{
        GetCommState, PurgeComm, SetCommState, SetCommTimeouts, SetupComm, COMMTIMEOUTS, DCB, NOPARITY, ONESTOPBIT, PURGE_RXCLEAR, PURGE_TXCLEAR,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, OPEN_EXISTING};
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ};

    /// DCB bit fields: fBinary, fDtrControl = DTR_CONTROL_ENABLE, fRtsControl = RTS_CONTROL_ENABLE.
    const DCB_BITS: u32 = 0x0001 | (1 << 4) | (1 << 12);
    /// The longest wait of one ReadFile.
    const READ_WAIT_MS: u32 = 10;
    /// The longest wait of one WriteFile.
    const WRITE_WAIT_MS: u32 = 50;

    pub struct ComTransport {
        h: HANDLE,
    }

    // SAFETY: the handle is a file handle owned by this value only; Win32
    // file handles can be used from any thread.
    unsafe impl Send for ComTransport {}

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    impl ComTransport {
        pub fn open(name: &str) -> io::Result<ComTransport> {
            let path = wide(&format!(r"\\.\{name}"));
            // SAFETY: a valid NUL-terminated path; no security attributes,
            // no template.
            let h = unsafe { CreateFileW(path.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
            if h == INVALID_HANDLE_VALUE || h.is_null() {
                return Err(io::Error::last_os_error());
            }
            let t = ComTransport { h };
            // SAFETY: `h` is an open COM handle; the structs live through the calls.
            unsafe {
                let mut dcb: DCB = std::mem::zeroed();
                dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
                if GetCommState(t.h, &mut dcb) == 0 {
                    return Err(io::Error::last_os_error());
                }
                dcb.BaudRate = 115_200;
                dcb.ByteSize = 8;
                dcb.Parity = NOPARITY;
                dcb.StopBits = ONESTOPBIT;
                dcb._bitfield = DCB_BITS;
                if SetCommState(t.h, &dcb) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let _ = SetupComm(t.h, 4096, 4096);
                let to = COMMTIMEOUTS {
                    ReadIntervalTimeout: u32::MAX,
                    ReadTotalTimeoutMultiplier: u32::MAX,
                    ReadTotalTimeoutConstant: READ_WAIT_MS,
                    WriteTotalTimeoutMultiplier: 0,
                    WriteTotalTimeoutConstant: WRITE_WAIT_MS,
                };
                if SetCommTimeouts(t.h, &to) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let _ = PurgeComm(t.h, PURGE_RXCLEAR | PURGE_TXCLEAR);
            }
            Ok(t)
        }
    }

    impl Transport for ComTransport {
        fn read(&mut self, buf: &mut [u8], wait: Duration) -> io::Result<usize> {
            let end = Instant::now() + wait;
            loop {
                let mut n: u32 = 0;
                let len = buf.len().min(u32::MAX as usize) as u32;
                // SAFETY: the buffer holds `len` bytes; synchronous I/O.
                let ok = unsafe { ReadFile(self.h, buf.as_mut_ptr(), len, &mut n, std::ptr::null_mut()) };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                if n > 0 || Instant::now() >= end {
                    return Ok(n as usize);
                }
            }
        }

        fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
            let mut n: u32 = 0;
            // SAFETY: `data` is valid for its length; synchronous I/O.
            let ok = unsafe { WriteFile(self.h, data.as_ptr(), data.len() as u32, &mut n, std::ptr::null_mut()) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if (n as usize) < data.len() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "the COM port does not take the data"));
            }
            Ok(())
        }
    }

    impl Drop for ComTransport {
        fn drop(&mut self) {
            // SAFETY: the handle is open and closed once.
            unsafe { CloseHandle(self.h) };
        }
    }

    /// The COM port names of `HKLM\HARDWARE\DEVICEMAP\SERIALCOMM` (opens
    /// no port).
    pub fn list_ports() -> Vec<String> {
        let mut out = Vec::new();
        let key = wide(r"HARDWARE\DEVICEMAP\SERIALCOMM");
        let mut hk: HKEY = std::ptr::null_mut();
        // SAFETY: valid key path and out pointer.
        if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, key.as_ptr(), 0, KEY_READ, &mut hk) } != ERROR_SUCCESS {
            return out;
        }
        for i in 0..256u32 {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            let mut data = [0u16; 64];
            let mut data_len = (data.len() * 2) as u32;
            let mut ty: u32 = 0;
            // SAFETY: the buffers and their lengths (in chars / bytes) match.
            let r = unsafe { RegEnumValueW(hk, i, name.as_mut_ptr(), &mut name_len, std::ptr::null(), &mut ty, data.as_mut_ptr().cast(), &mut data_len) };
            if r == ERROR_NO_MORE_ITEMS {
                break;
            }
            if r != ERROR_SUCCESS || ty != REG_SZ {
                continue;
            }
            let chars = (data_len as usize / 2).min(data.len());
            let end = data[..chars].iter().position(|&c| c == 0).unwrap_or(chars);
            let port = String::from_utf16_lossy(&data[..end]).trim().to_string();
            if !port.is_empty() && !out.contains(&port) {
                out.push(port);
            }
        }
        // SAFETY: the key was opened above.
        unsafe { RegCloseKey(hk) };
        out
    }
}

/// The COM ports of this PC, sorted by number (opens no port).
pub fn list_com_ports() -> Vec<String> {
    #[cfg(windows)]
    let mut v = com::list_ports();
    #[cfg(not(windows))]
    let mut v: Vec<String> = Vec::new();
    v.sort_by_key(|p| (com_number(p).unwrap_or(u32::MAX), p.clone()));
    v
}

fn connect(target: &SlcanTarget) -> io::Result<Box<dyn Transport>> {
    match target {
        SlcanTarget::Tcp(a) => Ok(Box::new(TcpTransport::connect(a)?)),
        #[cfg(windows)]
        SlcanTarget::Serial(p) => Ok(Box::new(com::ComTransport::open(p)?)),
        #[cfg(not(windows))]
        SlcanTarget::Serial(_) => Err(io::Error::new(io::ErrorKind::Unsupported, "COM ports need Windows")),
    }
}

// ---- the adapter ----

/// The wait for the answer of `C` before it is thrown away.
const DRAIN_WAIT: Duration = Duration::from_millis(100);
/// The wait for the answer of `V`.
const VERSION_WAIT: Duration = Duration::from_millis(500);
/// The wait for the answer of `S<n>`, `O` and `L`.
const REPLY_WAIT: Duration = Duration::from_millis(200);
/// Received frames kept before the oldest are dropped.
const RX_CAP: usize = 10_000;
/// Commands waiting for their answer; beyond this the device is taken to
/// have stopped answering.
const PENDING_CAP: usize = 1_000;

/// A command whose answer has not come yet (only on a device that answers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Frame,
    Flags,
}

/// The SLCAN adapter. `channel` of `open` is not used: the target is given
/// to `new`.
pub struct SlcanAdapter {
    target: SlcanTarget,
    io: Option<Box<dyn Transport>>,
    reader: LineReader,
    tokens: Vec<Token>,
    rx: VecDeque<CanFrame>,
    listen_only: bool,
    /// The device answers commands (it answered `S` or `O`).
    acks: bool,
    pending: VecDeque<Pending>,
    /// The text of a lost connection; every later call returns it.
    lost: Option<String>,
    flags: Option<u8>,
    refused: u64,
    refused_unreported: u64,
    unknown: u64,
    version: Option<String>,
}

impl SlcanAdapter {
    pub fn new(target: SlcanTarget) -> SlcanAdapter {
        SlcanAdapter {
            target,
            io: None,
            reader: LineReader::default(),
            tokens: Vec::new(),
            rx: VecDeque::new(),
            listen_only: false,
            acks: false,
            pending: VecDeque::new(),
            lost: None,
            flags: None,
            refused: 0,
            refused_unreported: 0,
            unknown: 0,
            version: None,
        }
    }

    pub fn target(&self) -> &SlcanTarget {
        &self.target
    }

    /// Lines that were not understood (and lines that were too long).
    pub fn unknown_lines(&self) -> u64 {
        self.unknown
    }

    /// Frames the device refused with BEL.
    pub fn refused_frames(&self) -> u64 {
        self.refused
    }

    /// The text after `V` of the version answer.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The device answered `S` or `O` (a Lawicel-type device; the CANable
    /// firmwares do not answer).
    pub fn answers_commands(&self) -> bool {
        self.acks
    }

    fn what(&self) -> String {
        format!("SLCAN {}", self.target.label())
    }

    fn reset_state(&mut self) {
        self.reader = LineReader::default();
        self.tokens.clear();
        self.rx.clear();
        self.pending.clear();
        self.lost = None;
        self.flags = None;
        self.refused = 0;
        self.refused_unreported = 0;
        self.acks = false;
        self.listen_only = false;
    }

    fn set_lost(&mut self, e: &io::Error) -> CanError {
        if self.lost.is_none() {
            self.lost = Some(format!("{}: connection lost ({e})", self.what()));
        }
        self.rx.clear();
        CanError::new(self.lost.clone().unwrap_or_default())
    }

    fn write(&mut self, s: &str) -> Result<(), CanError> {
        let Some(io) = self.io.as_mut() else { return Err(CanError::new("channel not open")) };
        match io.write_all(s.as_bytes()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Err(CanError::new(format!("{}: {e}", self.what()))),
            Err(e) => Err(self.set_lost(&e)),
        }
    }

    /// Read once (waiting at most `wait`) and turn the bytes into tokens.
    fn pull(&mut self, wait: Duration) -> Result<(), CanError> {
        let Some(io) = self.io.as_mut() else { return Err(CanError::new("channel not open")) };
        let mut buf = [0u8; 512];
        match io.read(&mut buf, wait) {
            Ok(n) => {
                self.reader.push(&buf[..n], &mut self.tokens);
                Ok(())
            }
            Err(e) => Err(self.set_lost(&e)),
        }
    }

    /// Handle the tokens read so far while the channel is open.
    fn handle_tokens(&mut self) {
        for t in std::mem::take(&mut self.tokens) {
            match t {
                Token::Line(l) => match parse_line(&l) {
                    Reply::Frame { frame, .. } => {
                        if self.rx.len() >= RX_CAP {
                            self.rx.pop_front();
                        }
                        self.rx.push_back(frame);
                    }
                    Reply::TxAck => {
                        self.pending.pop_front();
                    }
                    Reply::Flags(f) => {
                        self.flags = Some(f);
                        self.pending.pop_front();
                    }
                    Reply::Version(v) => self.version = Some(v),
                    Reply::Serial(_) => {}
                    Reply::Unknown(_) => self.unknown += 1,
                },
                Token::Ok => {
                    self.pending.pop_front();
                }
                Token::Bel => {
                    if self.pending.pop_front() != Some(Pending::Flags) {
                        self.refused += 1;
                        self.refused_unreported += 1;
                    }
                }
                Token::TooLong => self.unknown += 1,
            }
        }
    }

    /// Wait for the first answer that is not a received frame.
    fn wait_reply(&mut self, wait: Duration) -> Result<Option<Token>, CanError> {
        let end = Instant::now() + wait;
        loop {
            let mut found = None;
            for t in std::mem::take(&mut self.tokens) {
                if found.is_some() {
                    continue;
                }
                match &t {
                    Token::Line(l) => match parse_line(l) {
                        Reply::Frame { .. } => {}
                        Reply::Version(v) => {
                            self.version = Some(v);
                            found = Some(t);
                        }
                        _ => found = Some(t),
                    },
                    _ => found = Some(t),
                }
            }
            if found.is_some() {
                return Ok(found);
            }
            let now = Instant::now();
            if now >= end {
                return Ok(None);
            }
            self.pull((end - now).min(Duration::from_millis(20)))?;
        }
    }

    fn open_sequence(&mut self, bitrate: Bitrate, listen_only: bool) -> Result<(), CanError> {
        let what = self.what();
        // Close a channel that an earlier session left open; this also
        // empties the command queue of the device. The answer is not used.
        self.write("C\r")?;
        let end = Instant::now() + DRAIN_WAIT;
        while Instant::now() < end {
            self.pull(Duration::from_millis(20))?;
        }
        self.tokens.clear();
        self.write("V\r")?;
        if self.wait_reply(VERSION_WAIT)?.is_none() {
            return Err(CanError::new(format!("{what}: no answer to the version command V; is it an SLCAN device?")));
        }
        let code = bitrate_code(bitrate);
        self.write(&format!("S{code}\r"))?;
        match self.wait_reply(REPLY_WAIT)? {
            Some(Token::Bel) => return Err(CanError::new(format!("{what}: the device refused the bitrate {} (S{code})", bitrate.label()))),
            Some(_) => self.acks = true,
            None => self.acks = false,
        }
        let cmd = if listen_only { "L" } else { "O" };
        self.write(&format!("{cmd}\r"))?;
        match self.wait_reply(REPLY_WAIT)? {
            Some(Token::Bel) if listen_only => return Err(CanError::new(format!("{what}: the device refused listen-only mode (L)"))),
            Some(Token::Bel) => return Err(CanError::new(format!("{what}: the device refused to open the channel (O)"))),
            Some(_) => self.acks = true,
            None if self.acks => return Err(CanError::new(format!("{what}: no answer to the open command {cmd}"))),
            None => {}
        }
        Ok(())
    }
}

impl CanAdapter for SlcanAdapter {
    fn name(&self) -> String {
        self.what()
    }

    fn list_channels(&mut self) -> Result<Vec<ChannelInfo>, CanError> {
        Ok(match self.target {
            SlcanTarget::Serial(_) => list_com_ports()
                .into_iter()
                .map(|p| ChannelInfo { handle: com_number(&p).unwrap_or(0), name: p, available: true })
                .collect(),
            SlcanTarget::Tcp(_) => Vec::new(),
        })
    }

    fn open(&mut self, _channel: u32, bitrate: Bitrate, listen_only: bool) -> Result<(), CanError> {
        self.close();
        self.reset_state();
        let what = self.what();
        let io = connect(&self.target).map_err(|e| CanError::new(format!("{what}: {e}")))?;
        self.io = Some(io);
        if let Err(e) = self.open_sequence(bitrate, listen_only) {
            self.io = None;
            self.lost = None;
            return Err(e);
        }
        self.listen_only = listen_only;
        self.tokens.clear();
        Ok(())
    }

    fn close(&mut self) {
        if self.io.is_some() && self.lost.is_none() {
            let _ = self.write("C\r");
        }
        self.io = None;
        self.rx.clear();
        self.pending.clear();
    }

    fn send(&mut self, frame: &CanFrame) -> Result<(), CanError> {
        if let Some(l) = &self.lost {
            return Err(CanError::new(l.clone()));
        }
        if self.io.is_none() {
            return Err(CanError::new("channel not open"));
        }
        if self.listen_only {
            return Err(CanError::new("listen-only mode: sending is disabled"));
        }
        let line = encode_frame(frame)?;
        self.write(&line)?;
        if self.acks {
            if self.pending.len() >= PENDING_CAP {
                self.pending.clear();
            }
            self.pending.push_back(Pending::Frame);
        }
        Ok(())
    }

    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, CanError> {
        if let Some(l) = &self.lost {
            return Err(CanError::new(l.clone()));
        }
        if self.io.is_none() {
            return Err(CanError::new("channel not open"));
        }
        let end = Instant::now() + timeout;
        loop {
            if let Some(f) = self.rx.pop_front() {
                return Ok(Some(f));
            }
            if self.refused_unreported > 0 {
                let n = std::mem::take(&mut self.refused_unreported);
                return Err(CanError::new(format!("{}: the device refused {n} frame(s) (BEL)", self.what())));
            }
            let now = Instant::now();
            self.pull(end.saturating_duration_since(now))?;
            self.handle_tokens();
            if self.rx.is_empty() && self.refused_unreported == 0 && Instant::now() >= end {
                return Ok(None);
            }
        }
    }

    fn status(&mut self) -> String {
        if self.lost.is_some() {
            return "Connection lost".to_string();
        }
        if self.io.is_none() {
            return "closed".to_string();
        }
        // Ask for the flags on a device that answers commands; the answer
        // comes with a later receive.
        if self.acks && !self.pending.contains(&Pending::Flags) && self.write("F\r").is_ok() {
            self.pending.push_back(Pending::Flags);
        }
        let mut s = self.flags.map(flags_text).unwrap_or_else(|| "OK".to_string());
        if self.refused > 0 {
            s.push_str(&format!(", {} frame(s) refused", self.refused));
        }
        s
    }
}

impl Drop for SlcanAdapter {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(chunks: &[&[u8]]) -> Vec<Token> {
        let mut r = LineReader::default();
        let mut out = Vec::new();
        for c in chunks {
            r.push(c, &mut out);
        }
        out
    }

    fn frame(id: u32, ext: bool, rtr: bool, data: &[u8]) -> CanFrame {
        let mut f = CanFrame::new(id, data);
        f.extended = ext;
        f.rtr = rtr;
        f
    }

    #[test]
    fn encode_standard_extended_rtr() {
        assert_eq!(encode_frame(&frame(0x100, false, false, &[0x11, 0x33])).unwrap(), "t10021133\r");
        assert_eq!(encode_frame(&frame(0x20, false, false, &[])).unwrap(), "t0200\r");
        assert_eq!(encode_frame(&frame(0x100, true, false, &[0x11, 0x33])).unwrap(), "T0000010021133\r");
        let mut r = frame(0x100, false, true, &[]);
        r.dlc = 2;
        assert_eq!(encode_frame(&r).unwrap(), "r1002\r");
        r.extended = true;
        assert_eq!(encode_frame(&r).unwrap(), "R000001002\r");
        let f = frame(0x7FF, false, false, &[0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89]);
        assert_eq!(encode_frame(&f).unwrap(), "t7FF8ABCDEF0123456789\r");
        assert_eq!(encode_frame(&frame(0x1FFF_FFFF, true, false, &[1])).unwrap(), "T1FFFFFFF101\r");
        assert!(encode_frame(&frame(0x800, false, false, &[])).is_err());
    }

    #[test]
    fn decode_round_trip_every_dlc() {
        for ext in [false, true] {
            for rtr in [false, true] {
                for dlc in 0..=8u8 {
                    let data: Vec<u8> = (0..dlc).map(|i| 0xA0 + i).collect();
                    let mut f = frame(if ext { 0x1234_5678 } else { 0x5A5 }, ext, rtr, &data);
                    f.dlc = dlc;
                    if rtr {
                        f.data = [0; 8];
                    }
                    f.dir = Dir::Rx;
                    let line = encode_frame(&f).unwrap();
                    let line = line.trim_end_matches('\r');
                    assert_eq!(parse_line(line), Reply::Frame { frame: f, timestamp_ms: None }, "{line}");
                }
            }
        }
    }

    #[test]
    fn decode_time_stamp_and_lower_case_hex() {
        match parse_line("t100211334D67") {
            Reply::Frame { frame, timestamp_ms } => {
                assert_eq!(frame.id, 0x100);
                assert_eq!(frame.payload(), &[0x11, 0x33]);
                assert_eq!(timestamp_ms, Some(0x4D67));
            }
            r => panic!("{r:?}"),
        }
        match parse_line("T1abcdef03aabbccEA5F") {
            Reply::Frame { frame, timestamp_ms } => {
                assert!(frame.extended);
                assert_eq!(frame.id, 0x1ABC_DEF0);
                assert_eq!(frame.payload(), &[0xAA, 0xBB, 0xCC]);
                assert_eq!(timestamp_ms, Some(0xEA5F));
            }
            r => panic!("{r:?}"),
        }
        match parse_line("r1238FFFF") {
            Reply::Frame { frame, timestamp_ms } => {
                assert!(frame.rtr);
                assert_eq!(frame.dlc, 8);
                assert_eq!(timestamp_ms, Some(0xFFFF));
            }
            r => panic!("{r:?}"),
        }
    }

    #[test]
    fn bad_lines_are_unknown() {
        for l in ["t", "t10", "t100", "t1009", "t1002113", "t100211", "t10021133AB", "t80000", "TFFFFFFFF0", "tXYZ0", "t1002113G", "T123", "x", "", "t1001ÄÄ", "F1", "FGG"] {
            assert!(matches!(parse_line(l), Reply::Unknown(_)), "{l:?} gives {:?}", parse_line(l));
        }
    }

    #[test]
    fn decode_replies() {
        assert_eq!(parse_line("z"), Reply::TxAck);
        assert_eq!(parse_line("Z"), Reply::TxAck);
        assert_eq!(parse_line("F00"), Reply::Flags(0));
        assert_eq!(parse_line("FA4"), Reply::Flags(0xA4));
        assert_eq!(parse_line("V1013"), Reply::Version("1013".to_string()));
        assert_eq!(parse_line("NA123"), Reply::Serial("A123".to_string()));
    }

    #[test]
    fn reader_partial_lines_bel_crlf_too_long() {
        assert_eq!(tokens(&[b"t10", b"0211", b"33\r"]), vec![Token::Line("t10021133".to_string())]);
        assert_eq!(tokens(&[b"\r\x07z\r"]), vec![Token::Ok, Token::Bel, Token::Line("z".to_string())]);
        assert_eq!(tokens(&[b"V1013\r\n", b"\r\n"]), vec![Token::Line("V1013".to_string()), Token::Ok]);
        let long = vec![b'A'; MAX_LINE + 10];
        assert_eq!(tokens(&[&long, b"\rz\r"]), vec![Token::TooLong, Token::Line("z".to_string())]);
        let mut r = LineReader::default();
        let mut out = Vec::new();
        r.push(b"t12", &mut out);
        assert!(out.is_empty());
        assert_eq!(r.pending(), 3);
    }

    #[test]
    fn bitrate_codes() {
        let expect = [
            (Bitrate::K10, '0'),
            (Bitrate::K20, '1'),
            (Bitrate::K50, '2'),
            (Bitrate::K125, '4'),
            (Bitrate::K250, '5'),
            (Bitrate::K500, '6'),
            (Bitrate::K800, '7'),
            (Bitrate::M1, '8'),
        ];
        for (b, c) in expect {
            assert_eq!(bitrate_code(b), c, "{b:?}");
        }
    }

    #[test]
    fn flag_texts() {
        assert_eq!(flags_text(0), "OK");
        assert_eq!(flags_text(0x20), "Error passive");
        assert_eq!(flags_text(0x04 | 0x08), "Error warning, Data overrun");
        assert!(flags_text(0x80).contains("Bus error"));
    }

    #[test]
    fn targets() {
        assert_eq!(SlcanTarget::parse("COM5"), Ok(SlcanTarget::Serial("COM5".to_string())));
        assert_eq!(SlcanTarget::parse(" com12 "), Ok(SlcanTarget::Serial("COM12".to_string())));
        assert_eq!(SlcanTarget::parse("192.168.1.10:2001"), Ok(SlcanTarget::Tcp("192.168.1.10:2001".to_string())));
        assert_eq!(SlcanTarget::parse("gw.local:23"), Ok(SlcanTarget::Tcp("gw.local:23".to_string())));
        assert_eq!(SlcanTarget::parse("[::1]:2001"), Ok(SlcanTarget::Tcp("[::1]:2001".to_string())));
        for bad in ["", "COM", "COM0", "COM256", "COMx", "usb1", "5", "host:", ":2001", "host:0", "host:65536", "host:abc", "a b:1"] {
            assert!(SlcanTarget::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(com_number("COM7"), Some(7));
    }

    #[test]
    fn closed_adapter_refuses_calls() {
        let mut a = SlcanAdapter::new(SlcanTarget::Tcp("127.0.0.1:1".to_string()));
        assert!(a.send(&CanFrame::new(0x123, &[1])).is_err());
        assert!(a.receive(Duration::ZERO).is_err());
        assert_eq!(a.status(), "closed");
        assert_eq!(a.name(), "SLCAN 127.0.0.1:1");
        assert!(a.list_channels().unwrap().is_empty());
    }
}
