//! Log replay: the parsers of the trace files that CANoxide writes (the
//! continuous log and "Save as CSV", header `time_s,delta_ms,...`, also
//! the files of 0.2.0), of candump logs (`(1700000000.123456) can0
//! 123#DEADBEEF`), of PEAK `.trc` and Vector `.asc` traces (in
//! `crate::tracefmt`), the replay clock (`ReplayPlayer`), the view rebuild
//! after a seek (`Rebuild`) and a file adapter for the command-line tool.
//!
//! The format is found from the content, not from the file extension.
//!
//! Replay never transmits: the adapter refuses every send, and the bus
//! worker runs a replay in listen-only mode.
//!
//! Time stamps: the first frame of a log is at 0; later frames keep their
//! distance to it. A time stamp that goes back is raised to the one before
//! it, so that the log is in time order.

use crate::busload::BusLoad;
use crate::can::{Bitrate, CanAdapter, CanError, CanFrame, ChannelInfo, Dir};
use crate::canopen::decode::{decode, DecodeContext, Decoded};
use crate::canopen::nmt::NodeTracker;
use crate::msgstats::MsgStats;
use crate::netinfo::NetworkOverview;
use crate::signals::SignalStore;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most line errors kept in a `ParsedLog` (all are counted).
pub const MAX_ERRORS_KEPT: usize = 1000;
/// Most frames that one `advance` gives at "as fast as possible".
pub const MAX_BATCH: usize = 2000;
/// The speed factors offered in the transport bar.
pub const SPEEDS: [f64; 7] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogFormat {
    /// A trace CSV of CANoxide (continuous log or "Save as CSV").
    CanoxideCsv,
    /// A candump log (`candump -l`).
    Candump,
    /// A PEAK PCAN-View / PCAN-Trace file (`.trc`), with its file version
    /// (major, minor).
    PeakTrc(u8, u8),
    /// A Vector ASCII log (`.asc`).
    VectorAsc,
}

impl LogFormat {
    pub fn label(self) -> String {
        match self {
            LogFormat::CanoxideCsv => "CANoxide CSV".to_string(),
            LogFormat::Candump => "candump log".to_string(),
            LogFormat::PeakTrc(a, b) => format!("PEAK trace {a}.{b}"),
            LogFormat::VectorAsc => "Vector ASC".to_string(),
        }
    }
}

/// A line that could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineError {
    /// 1-based line number (the first line of a record).
    pub line: usize,
    pub message: String,
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// The frames of a log and the lines that could not be read.
#[derive(Clone, Debug)]
pub struct ParsedLog {
    pub format: LogFormat,
    /// In time order, the first at 0 µs.
    pub frames: Vec<CanFrame>,
    /// The first `MAX_ERRORS_KEPT` bad lines.
    pub errors: Vec<LineError>,
    /// All bad lines.
    pub bad_lines: usize,
    /// Records that are read correctly but are not classic CAN frames
    /// (error frames, status and event lines, CAN FD frames of a `.trc` or
    /// `.asc` file): skipped and counted, not bad lines.
    pub skipped: usize,
}

impl ParsedLog {
    pub fn duration_us(&self) -> u64 {
        self.frames.last().map_or(0, |f| f.timestamp_us)
    }
}

pub(crate) fn progress_set(p: Option<&AtomicUsize>, v: usize) {
    if let Some(p) = p {
        p.store(v, Ordering::Relaxed);
    }
}

/// Read a log file. `progress` gets the bytes done (of the file size).
pub fn load_log(path: &Path, progress: Option<&AtomicUsize>) -> Result<ParsedLog, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&b);
    parse_log(&text, progress)
}

/// Parse a log; the format is found from the first line that is not empty.
/// `Err` when the whole file cannot be read (unknown format, a broken
/// header, String-mode data, no frame at all).
pub fn parse_log(text: &str, progress: Option<&AtomicUsize>) -> Result<ParsedLog, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).ok_or("the file is empty")?;
    let mut log = if first.starts_with("time_s,") || first.starts_with("\"time_s\",") {
        parse_csv(text, progress)?
    } else if first.starts_with('(') {
        parse_candump(text, progress)
    } else if crate::tracefmt::is_trc(first) {
        crate::tracefmt::parse_trc(text, progress)?
    } else if crate::tracefmt::is_asc(first) {
        crate::tracefmt::parse_asc(text, progress)?
    } else {
        let short: String = first.chars().take(40).collect();
        return Err(format!("unknown file format (the first line is \"{short}\"); expected a CANoxide trace CSV, a candump log, a PEAK .trc or a Vector .asc file"));
    };
    progress_set(progress, text.len());
    if log.frames.is_empty() {
        return Err(match log.errors.first() {
            Some(e) => format!("no frame could be read ({} bad line(s); first: {e})", log.bad_lines),
            None if log.skipped > 0 => format!("the file holds no classic CAN frame ({} other record(s) skipped)", log.skipped),
            None => "the file holds no frame".to_string(),
        });
    }
    rebase(&mut log.frames);
    Ok(log)
}

/// First frame at 0; time stamps never go back.
fn rebase(frames: &mut [CanFrame]) {
    let Some(t0) = frames.first().map(|f| f.timestamp_us) else { return };
    let mut prev = 0u64;
    for f in frames {
        let t = f.timestamp_us.saturating_sub(t0).max(prev);
        f.timestamp_us = t;
        prev = t;
    }
}

pub(crate) struct Errors {
    pub(crate) list: Vec<LineError>,
    pub(crate) count: usize,
}

impl Errors {
    pub(crate) fn new() -> Self {
        Errors { list: Vec::new(), count: 0 }
    }
    pub(crate) fn add(&mut self, line: usize, message: impl Into<String>) {
        self.count += 1;
        if self.list.len() < MAX_ERRORS_KEPT {
            self.list.push(LineError { line, message: message.into() });
        }
    }
}

// ------------------------------------------------------------------ CSV

/// RFC 4180 records with their first line number; a quoted field may hold
/// commas, quotes and line ends.
struct CsvRecords<'a> {
    s: &'a str,
    pos: usize,
    line: usize,
}

impl<'a> CsvRecords<'a> {
    fn new(s: &'a str) -> Self {
        CsvRecords { s, pos: 0, line: 1 }
    }

    /// Skip to the start of the next physical line.
    fn skip_line(&mut self) {
        let b = self.s.as_bytes();
        while self.pos < b.len() && b[self.pos] != b'\n' {
            self.pos += 1;
        }
        if self.pos < b.len() {
            self.pos += 1;
            self.line += 1;
        }
    }
}

type CsvRecord<'a> = (usize, Result<Vec<Cow<'a, str>>, String>);

impl<'a> Iterator for CsvRecords<'a> {
    type Item = CsvRecord<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let b = self.s.as_bytes();
        if self.pos >= b.len() {
            return None;
        }
        let start_line = self.line;
        let mut fields: Vec<Cow<'a, str>> = Vec::with_capacity(11);
        let mut i = self.pos;
        loop {
            if i < b.len() && b[i] == b'"' {
                let mut out: Vec<u8> = Vec::new();
                i += 1;
                loop {
                    if i >= b.len() {
                        self.pos = b.len();
                        return Some((start_line, Err("a quoted field is not closed".to_string())));
                    }
                    match b[i] {
                        b'"' if i + 1 < b.len() && b[i + 1] == b'"' => {
                            out.push(b'"');
                            i += 2;
                        }
                        b'"' => {
                            i += 1;
                            break;
                        }
                        c => {
                            if c == b'\n' {
                                self.line += 1;
                            }
                            out.push(c);
                            i += 1;
                        }
                    }
                }
                fields.push(Cow::Owned(String::from_utf8_lossy(&out).into_owned()));
                if i < b.len() && !matches!(b[i], b',' | b'\n' | b'\r') {
                    self.pos = i;
                    self.skip_line();
                    return Some((start_line, Err("unexpected text after a quoted field".to_string())));
                }
            } else {
                let st = i;
                while i < b.len() && !matches!(b[i], b',' | b'\n' | b'\r') {
                    i += 1;
                }
                fields.push(Cow::Borrowed(&self.s[st..i]));
            }
            // Field separator or record end.
            if i < b.len() && b[i] == b',' {
                i += 1;
                continue;
            }
            if i < b.len() && b[i] == b'\r' {
                i += 1;
            }
            if i < b.len() && b[i] == b'\n' {
                i += 1;
                self.line += 1;
            }
            self.pos = i;
            return Some((start_line, Ok(fields)));
        }
    }
}

/// How the data column was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataMode {
    Hex,
    Decimal,
}

struct CsvCols {
    time: usize,
    dir: Option<usize>,
    id: usize,
    ext: usize,
    rtr: usize,
    dlc: usize,
    data: usize,
}

impl CsvCols {
    fn from_header(h: &[Cow<'_, str>]) -> Result<CsvCols, String> {
        let find = |name: &str| h.iter().position(|c| c.trim() == name);
        let need = |name: &str| find(name).ok_or_else(|| format!("the header has no column \"{name}\""));
        Ok(CsvCols { time: need("time_s")?, dir: find("dir"), id: need("id")?, ext: need("ext")?, rtr: need("rtr")?, dlc: need("dlc")?, data: need("data")? })
    }

    fn max(&self) -> usize {
        [self.time, self.id, self.ext, self.rtr, self.dlc, self.data].into_iter().chain(self.dir).max().unwrap_or(0)
    }
}

fn is_blank(r: &[Cow<'_, str>]) -> bool {
    r.len() == 1 && r[0].trim().is_empty()
}

/// Decide hex or decimal from the data column (a hex letter: hex; a value
/// of 1 or 3 digits: decimal; only 2-digit values: hex). `Err` for data
/// written in String mode.
fn detect_mode(text: &str, cols: &CsvCols) -> Result<DataMode, String> {
    let mut recs = CsvRecords::new(text);
    recs.next();
    for (_, r) in recs {
        let Ok(r) = r else { continue };
        if is_blank(&r) || r.len() <= cols.max() {
            continue;
        }
        let data = r[cols.data].trim();
        let dlc: usize = r[cols.dlc].trim().parse().unwrap_or(0);
        let rtr = r[cols.rtr].trim() == "1";
        if !rtr && dlc >= 2 && data.chars().count() == dlc && data.split_whitespace().count() != dlc {
            return Err("the data column was saved in String mode and cannot be read back; save the trace in Hex or Decimal mode".to_string());
        }
        for t in data.split_whitespace() {
            if t.bytes().any(|c| matches!(c, b'a'..=b'f' | b'A'..=b'F')) {
                return Ok(DataMode::Hex);
            }
            if t.len() == 1 || t.len() == 3 {
                return Ok(DataMode::Decimal);
            }
        }
    }
    Ok(DataMode::Hex)
}

fn parse_bit(s: &str, name: &str) -> Result<bool, String> {
    match s.trim() {
        "0" => Ok(false),
        "1" => Ok(true),
        o => Err(format!("{name} must be 0 or 1, not \"{o}\"")),
    }
}

/// Seconds with up to 6 decimals as µs, without float rounding (more
/// decimals are cut off).
pub(crate) fn parse_seconds(s: &str) -> Option<u64> {
    let s = s.trim();
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if int.is_empty() || !int.bytes().all(|c| c.is_ascii_digit()) || !frac.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let sec: u64 = int.parse().ok()?;
    let mut us: u64 = 0;
    for c in frac.bytes().chain(std::iter::repeat(b'0')).take(6) {
        us = us * 10 + (c - b'0') as u64;
    }
    sec.checked_mul(1_000_000)?.checked_add(us)
}

fn csv_frame(r: &[Cow<'_, str>], c: &CsvCols, mode: DataMode) -> Result<CanFrame, String> {
    if r.len() <= c.max() {
        return Err(format!("{} column(s), expected at least {}", r.len(), c.max() + 1));
    }
    let t = parse_seconds(&r[c.time]).ok_or_else(|| format!("bad time \"{}\"", r[c.time].trim()))?;
    let dir = match c.dir.map(|i| r[i].trim()) {
        None | Some("Rx") | Some("") => Dir::Rx,
        Some("Tx") => Dir::Tx,
        Some(o) => return Err(format!("dir must be Rx or Tx, not \"{o}\"")),
    };
    let id_s = r[c.id].trim();
    let id = u32::from_str_radix(id_s, 16).map_err(|_| format!("bad id \"{id_s}\""))?;
    let extended = parse_bit(&r[c.ext], "ext")?;
    let rtr = parse_bit(&r[c.rtr], "rtr")?;
    let dlc_s = r[c.dlc].trim();
    let dlc: u8 = dlc_s.parse().ok().filter(|d| *d <= 8).ok_or_else(|| format!("bad DLC \"{dlc_s}\""))?;
    let mut data = [0u8; 8];
    let toks: Vec<&str> = r[c.data].split_whitespace().collect();
    if !rtr {
        if toks.len() != dlc as usize {
            return Err(format!("the data has {} byte(s), the DLC is {dlc}", toks.len()));
        }
        for (k, t) in toks.iter().enumerate() {
            data[k] = match mode {
                DataMode::Hex if t.len() <= 2 => u8::from_str_radix(t, 16).ok(),
                DataMode::Decimal if t.len() <= 3 => t.parse().ok(),
                _ => None,
            }
            .ok_or_else(|| format!("bad data byte \"{t}\""))?;
        }
    } else if !toks.is_empty() {
        return Err("an RTR frame has no data".to_string());
    }
    let f = CanFrame { id, extended, rtr, dlc, data, timestamp_us: t, dir };
    f.validate().map_err(|e| e.to_string())?;
    Ok(f)
}

fn parse_csv(text: &str, progress: Option<&AtomicUsize>) -> Result<ParsedLog, String> {
    let mut recs = CsvRecords::new(text);
    let header = loop {
        match recs.next() {
            Some((_, Ok(r))) if is_blank(&r) => continue,
            Some((_, Ok(r))) => break r,
            Some((line, Err(e))) => return Err(format!("line {line}: header: {e}")),
            None => return Err("the file is empty".to_string()),
        }
    };
    let cols = CsvCols::from_header(&header)?;
    let mode = detect_mode(text, &cols)?;
    let mut frames = Vec::new();
    let mut errors = Errors::new();
    let mut n = 0usize;
    while let Some((line, r)) = recs.next() {
        n += 1;
        if n.is_multiple_of(4096) {
            progress_set(progress, recs.pos);
        }
        match r {
            Ok(r) if is_blank(&r) => {}
            Ok(r) => match csv_frame(&r, &cols, mode) {
                Ok(f) => frames.push(f),
                Err(e) => errors.add(line, e),
            },
            Err(e) => errors.add(line, e),
        }
    }
    Ok(ParsedLog { format: LogFormat::CanoxideCsv, frames, errors: errors.list, bad_lines: errors.count, skipped: 0 })
}

// -------------------------------------------------------------- candump

/// One candump line: `(seconds.micros) iface ID#DATA [R|T]`.
pub fn candump_frame(line: &str) -> Result<CanFrame, String> {
    let l = line.trim();
    let rest = l.strip_prefix('(').ok_or("the line does not start with \"(time)\"")?;
    let (ts, rest) = rest.split_once(')').ok_or("the time has no closing \")\"")?;
    let t = parse_seconds(ts).ok_or_else(|| format!("bad time \"{ts}\""))?;
    let mut parts = rest.split_whitespace();
    let _iface = parts.next().ok_or("no interface name")?;
    let fr = parts.next().ok_or("no frame (ID#DATA)")?;
    let dir = match parts.next() {
        None | Some("R") => Dir::Rx,
        Some("T") => Dir::Tx,
        Some(o) => return Err(format!("unexpected text \"{o}\" after the frame")),
    };
    let (id_s, data_s) = fr.split_once('#').ok_or_else(|| format!("no \"#\" in \"{fr}\""))?;
    if data_s.starts_with('#') {
        return Err("CAN FD frame (##); CAN FD is not supported".to_string());
    }
    if id_s.is_empty() || id_s.len() > 8 {
        return Err(format!("bad id \"{id_s}\""));
    }
    let id = u32::from_str_radix(id_s, 16).map_err(|_| format!("bad id \"{id_s}\""))?;
    let extended = id_s.len() > 3;
    if extended && id & 0x2000_0000 != 0 {
        return Err("error frame; error frames are not supported".to_string());
    }
    let mut data = [0u8; 8];
    let (rtr, dlc) = if let Some(r) = data_s.strip_prefix(['R', 'r']) {
        let dlc = if r.is_empty() { 0 } else { r.parse::<u8>().ok().filter(|d| *d <= 8).ok_or_else(|| format!("bad RTR length \"{r}\""))? };
        (true, dlc)
    } else {
        let hex: Vec<u8> = data_s.bytes().filter(|c| *c != b'.').collect();
        if !hex.len().is_multiple_of(2) {
            return Err(format!("odd number of hex digits in \"{data_s}\""));
        }
        if hex.len() > 16 {
            return Err("more than 8 data bytes (CAN FD is not supported)".to_string());
        }
        for (k, p) in hex.chunks(2).enumerate() {
            let s = std::str::from_utf8(p).map_err(|_| "bad data".to_string())?;
            data[k] = u8::from_str_radix(s, 16).map_err(|_| format!("bad data byte \"{s}\""))?;
        }
        (false, (hex.len() / 2) as u8)
    };
    let f = CanFrame { id, extended, rtr, dlc, data, timestamp_us: t, dir };
    f.validate().map_err(|e| e.to_string())?;
    Ok(f)
}

fn parse_candump(text: &str, progress: Option<&AtomicUsize>) -> ParsedLog {
    let mut frames = Vec::new();
    let mut errors = Errors::new();
    let mut pos = 0usize;
    for (i, line) in text.split('\n').enumerate() {
        pos += line.len() + 1;
        if i % 4096 == 0 {
            progress_set(progress, pos.min(text.len()));
        }
        if line.trim().is_empty() {
            continue;
        }
        match candump_frame(line) {
            Ok(f) => frames.push(f),
            Err(e) => errors.add(i + 1, e),
        }
    }
    ParsedLog { format: LogFormat::Candump, frames, errors: errors.list, bad_lines: errors.count, skipped: 0 }
}

/// One candump line of a frame (for tests and tools).
pub fn candump_line(f: &CanFrame, base_s: u64) -> String {
    let t = base_s * 1_000_000 + f.timestamp_us;
    let id = if f.extended { format!("{:08X}", f.id) } else { format!("{:03X}", f.id) };
    let data: String = if f.rtr && f.dlc > 0 {
        format!("R{}", f.dlc)
    } else if f.rtr {
        "R".to_string()
    } else { f.payload().iter().map(|b| format!("{b:02X}")).collect() };
    format!("({}.{:06}) can0 {id}#{data}", t / 1_000_000, t % 1_000_000)
}

// -------------------------------------------------------------- player

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    /// Log time runs at this factor of the wall time.
    Factor(f64),
    /// No pacing: `MAX_BATCH` frames per step.
    Max,
}

impl Speed {
    pub fn label(self) -> String {
        match self {
            Speed::Factor(f) => format!("{f}x"),
            Speed::Max => "As fast as possible".to_string(),
        }
    }
}

/// What the caller does after `ReplayPlayer::advance`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tick {
    /// Feed these frames (indices into the log).
    Frames(Range<usize>),
    /// The log looped: reset the views; play continues from the start.
    Restart,
}

/// The replay clock over a log in time order. The caller gives the wall
/// time in µs (any steady base), so that it can be tested without waiting.
/// The state "at time T" is: every frame with a time stamp <= T was fed.
#[derive(Clone, Debug)]
pub struct ReplayPlayer {
    len: usize,
    duration_us: u64,
    /// Index of the next frame to feed.
    cursor: usize,
    pos_us: u64,
    playing: bool,
    speed: Speed,
    looping: bool,
    /// (wall µs, log µs) at the last start or speed change.
    anchor: (u64, u64),
}

impl ReplayPlayer {
    pub fn new(frames: &[CanFrame]) -> ReplayPlayer {
        ReplayPlayer {
            len: frames.len(),
            duration_us: frames.last().map_or(0, |f| f.timestamp_us),
            cursor: 0,
            pos_us: 0,
            playing: false,
            speed: Speed::Factor(1.0),
            looping: false,
            anchor: (0, 0),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn duration_us(&self) -> u64 {
        self.duration_us
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn pos_us(&self) -> u64 {
        self.pos_us
    }
    pub fn playing(&self) -> bool {
        self.playing
    }
    pub fn speed(&self) -> Speed {
        self.speed
    }
    pub fn looping(&self) -> bool {
        self.looping
    }
    pub fn at_end(&self) -> bool {
        self.cursor >= self.len
    }

    pub fn set_looping(&mut self, on: bool) {
        self.looping = on;
    }

    /// Start playing. `true` when the log was at its end: the player went
    /// back to the start, and the caller must reset the views.
    pub fn play(&mut self, wall_us: u64) -> bool {
        let restart = self.at_end();
        if restart {
            self.cursor = 0;
            self.pos_us = 0;
        }
        self.playing = true;
        self.anchor = (wall_us, self.pos_us);
        restart
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Re-anchor the clock (after a speed change or a pause in the feed,
    /// for example while a seek is rebuilt).
    pub fn resume_clock(&mut self, wall_us: u64) {
        self.anchor = (wall_us, self.pos_us);
    }

    pub fn set_speed(&mut self, speed: Speed, wall_us: u64) {
        self.speed = speed;
        self.resume_clock(wall_us);
    }

    /// Move to log time `pos_us` (clamped to the log). Returns the number
    /// of frames up to it: the caller rebuilds the views from frames
    /// `0..cursor` without pacing. Playing goes on from there.
    pub fn seek(&mut self, frames: &[CanFrame], pos_us: u64, wall_us: u64) -> usize {
        let pos = pos_us.min(self.duration_us);
        self.cursor = frames[..self.len].partition_point(|f| f.timestamp_us <= pos);
        self.pos_us = pos;
        self.resume_clock(wall_us);
        self.cursor
    }

    /// One frame forward; pauses. The index of the frame to feed.
    pub fn step(&mut self, frames: &[CanFrame]) -> Option<usize> {
        self.playing = false;
        if self.at_end() {
            return None;
        }
        let i = self.cursor;
        self.cursor += 1;
        self.pos_us = frames[i].timestamp_us;
        Some(i)
    }

    /// The frames due at `wall_us`.
    pub fn advance(&mut self, frames: &[CanFrame], wall_us: u64) -> Tick {
        if !self.playing {
            return Tick::Frames(self.cursor..self.cursor);
        }
        if self.at_end() {
            if self.looping {
                self.cursor = 0;
                self.pos_us = 0;
                self.anchor = (wall_us, 0);
                return Tick::Restart;
            }
            self.playing = false;
            self.pos_us = self.duration_us;
            return Tick::Frames(self.cursor..self.cursor);
        }
        let start = self.cursor;
        let end = match self.speed {
            Speed::Max => {
                let end = (start + MAX_BATCH).min(self.len);
                self.pos_us = frames[end - 1].timestamp_us;
                end
            }
            Speed::Factor(s) => {
                let (w0, l0) = self.anchor;
                let target = l0 + (wall_us.saturating_sub(w0) as f64 * s.max(0.0)) as u64;
                let n = frames[start..self.len].partition_point(|f| f.timestamp_us <= target);
                self.pos_us = target.min(self.duration_us).max(self.pos_us);
                start + n
            }
        };
        self.cursor = end;
        if self.at_end() && !self.looping {
            self.playing = false;
            self.pos_us = self.duration_us;
        }
        Tick::Frames(start..end)
    }
}

// ------------------------------------------------------------- rebuild

/// The view state after a seek, built from the start of the log without
/// pacing, in steps (so that a large log does not block the caller): the
/// Messages statistics, the newest decoded frames for the trace, the node
/// states, the network overview and the bus load. The signal store is fed
/// on the way. Feeding a frame here does the same as the live path, so a
/// seek gives the same state as a play to the same point.
pub struct Rebuild {
    /// Frames `0..target` are fed.
    pub target: usize,
    pub done: usize,
    pub pos_us: u64,
    pub msgs: MsgStats,
    pub tail: VecDeque<(CanFrame, Decoded)>,
    tail_cap: usize,
    pub nodes: NodeTracker,
    pub network: NetworkOverview,
    pub load: BusLoad,
}

impl Rebuild {
    pub fn new(target: usize, pos_us: u64, tail_cap: usize, bitrate_kbit: u32) -> Rebuild {
        Rebuild {
            target,
            done: 0,
            pos_us,
            msgs: MsgStats::new(),
            tail: VecDeque::new(),
            tail_cap: tail_cap.max(1),
            nodes: NodeTracker::default(),
            network: NetworkOverview::new(),
            load: BusLoad::new(bitrate_kbit),
        }
    }

    pub fn finished(&self) -> bool {
        self.done >= self.target
    }

    /// Feed at most `max` more frames. True when all are fed (the bus load
    /// is then advanced to `pos_us`).
    pub fn step(&mut self, frames: &[CanFrame], ctx: &DecodeContext, signals: Option<&mut SignalStore>, max: usize) -> bool {
        let end = (self.done + max).min(self.target).min(frames.len());
        let mut signals = signals;
        for f in &frames[self.done..end] {
            let d = decode(f, ctx);
            self.msgs.on_frame(f, &d);
            if self.tail.len() >= self.tail_cap {
                self.tail.pop_front();
            }
            self.tail.push_back((*f, d));
            self.load.add(f);
            if f.dir == Dir::Rx {
                self.nodes.on_frame(f);
                self.network.on_frame(f);
            }
            if let Some(s) = signals.as_deref_mut() {
                s.on_frame(f, ctx);
            }
        }
        self.done = end;
        if end >= self.target.min(frames.len()) {
            self.done = self.target;
            self.load.advance(self.pos_us);
            true
        } else {
            false
        }
    }
}

// ------------------------------------------------------- file adapter

/// A read-only adapter over a log file for the command-line tool: it gives
/// the frames of the log with their time stamps, paced by `speed` (`None`:
/// as fast as possible). Every send is refused.
pub struct FileAdapter {
    name: String,
    frames: Arc<Vec<CanFrame>>,
    next: usize,
    speed: Option<f64>,
    started: Option<Instant>,
}

impl FileAdapter {
    pub fn new(name: impl Into<String>, frames: Vec<CanFrame>, speed: Option<f64>) -> FileAdapter {
        FileAdapter { name: name.into(), frames: Arc::new(frames), next: 0, speed: speed.filter(|s| *s > 0.0), started: None }
    }

    /// Load a log; bad lines are skipped (their count is in the result).
    pub fn open_file(path: &Path, speed: Option<f64>) -> Result<(FileAdapter, ParsedLog), String> {
        let mut log = load_log(path, None)?;
        let frames = std::mem::take(&mut log.frames);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "log".to_string());
        Ok((FileAdapter::new(format!("replay {name}"), frames, speed), log))
    }
}

impl CanAdapter for FileAdapter {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn list_channels(&mut self) -> Result<Vec<ChannelInfo>, CanError> {
        Ok(vec![ChannelInfo { handle: 0, name: self.name.clone(), available: true }])
    }
    fn open(&mut self, _channel: u32, _bitrate: Bitrate, _listen_only: bool) -> Result<(), CanError> {
        self.next = 0;
        self.started = Some(Instant::now());
        Ok(())
    }
    fn close(&mut self) {
        self.started = None;
    }
    fn send(&mut self, _frame: &CanFrame) -> Result<(), CanError> {
        Err(CanError::new("replay: a log file is read only, nothing is sent"))
    }
    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, CanError> {
        let Some(f) = self.frames.get(self.next).copied() else {
            return Ok(None);
        };
        if let (Some(s), Some(t0)) = (self.speed, self.started) {
            let due = Duration::from_secs_f64(f.timestamp_us as f64 / 1e6 / s);
            let el = t0.elapsed();
            if due > el {
                let wait = due - el;
                if wait > timeout {
                    std::thread::sleep(timeout);
                    return Ok(None);
                }
                std::thread::sleep(wait);
            }
        }
        self.next += 1;
        Ok(Some(f))
    }
    fn status(&mut self) -> String {
        if self.next >= self.frames.len() {
            "end of log".to_string()
        } else {
            "OK".to_string()
        }
    }
    fn keeps_timestamps(&self) -> bool {
        true
    }
    fn finished(&self) -> bool {
        self.next >= self.frames.len()
    }
}
