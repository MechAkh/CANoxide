//! PEAK `.trc` and Vector `.asc` trace files: the readers for log replay
//! and the writers of the trace export.
//!
//! Written from the public format descriptions: PEAK "PCAN-View / PCAN-Trace
//! file format" (file versions 1.0 to 3.0) and Vector "CAN, Log & Trigger
//! ASC Logging Format". Classic CAN only: CAN FD records, error frames,
//! status and event lines are counted as skipped records (`ParsedLog::
//! skipped`), not as bad lines.

use crate::can::{CanFrame, Dir};
use crate::replay::{progress_set, Errors, LogFormat, ParsedLog};
use std::io::Write;
use std::sync::atomic::AtomicUsize;

/// What one line of a trace file gave.
enum Rec {
    Frame(CanFrame),
    /// A valid record that is not a classic CAN frame.
    Skip,
    /// A header or comment line.
    None,
}

// ------------------------------------------------------------------ helpers

/// Milliseconds with up to 3 decimals as µs (more decimals are cut off).
fn parse_ms(s: &str) -> Option<u64> {
    let (a, b) = s.split_once('.').unwrap_or((s, ""));
    if a.is_empty() || !a.bytes().all(|c| c.is_ascii_digit()) || !b.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let ms: u64 = a.parse().ok()?;
    let mut frac = 0u64;
    for k in 0..3 {
        frac = frac * 10 + b.as_bytes().get(k).map_or(0, |c| u64::from(c - b'0'));
    }
    ms.checked_mul(1000)?.checked_add(frac)
}

fn hex_byte(t: &str) -> Result<u8, String> {
    if t.is_empty() || t.len() > 2 {
        return Err(format!("bad data byte \"{t}\""));
    }
    u8::from_str_radix(t, 16).map_err(|_| format!("bad data byte \"{t}\""))
}

/// Hex ID of a `.trc` line: more than 4 digits = extended (29 bit).
fn trc_id(t: &str) -> Result<(u32, bool), String> {
    if t.is_empty() || t.len() > 8 {
        return Err(format!("bad id \"{t}\""));
    }
    let id = u32::from_str_radix(t, 16).map_err(|_| format!("bad id \"{t}\""))?;
    Ok((id, t.len() > 4))
}

fn dlc_of(t: &str, radix: u32) -> Result<u8, String> {
    u8::from_str_radix(t, radix).ok().filter(|d| *d <= 8).ok_or_else(|| format!("bad DLC \"{t}\" (classic CAN: 0..8)"))
}

fn finish(mut f: CanFrame, data: &[&str], radix: u32) -> Result<CanFrame, String> {
    if !f.rtr {
        if data.len() < f.dlc as usize {
            return Err(format!("the data has {} byte(s), the DLC is {}", data.len(), f.dlc));
        }
        for (k, t) in data.iter().take(f.dlc as usize).enumerate() {
            f.data[k] = if radix == 16 {
                hex_byte(t)?
            } else {
                t.parse().ok().filter(|_| t.len() <= 3).ok_or_else(|| format!("bad data byte \"{t}\""))?
            };
        }
    }
    f.validate().map_err(|e| e.to_string())?;
    Ok(f)
}

fn frame(id: u32, extended: bool, rtr: bool, dlc: u8, t: u64, dir: Dir) -> CanFrame {
    CanFrame { id, extended, rtr, dlc, data: [0; 8], timestamp_us: t, dir }
}

/// Run `f` over the lines of `text` (1-based numbers) and collect.
fn collect(text: &str, progress: Option<&AtomicUsize>, format: LogFormat, mut f: impl FnMut(&str) -> Result<Rec, String>) -> ParsedLog {
    let mut frames = Vec::new();
    let mut errors = Errors::new();
    let mut skipped = 0usize;
    let mut pos = 0usize;
    for (i, line) in text.split('\n').enumerate() {
        pos += line.len() + 1;
        if i % 4096 == 0 {
            progress_set(progress, pos.min(text.len()));
        }
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        match f(l) {
            Ok(Rec::Frame(fr)) => frames.push(fr),
            Ok(Rec::Skip) => skipped += 1,
            Ok(Rec::None) => {}
            Err(e) => errors.add(i + 1, e),
        }
    }
    ParsedLog { format, frames, errors: errors.list, bad_lines: errors.count, skipped }
}

// ------------------------------------------------------------- PEAK .trc

/// True for the first line of a `.trc` file: a `;` header line, or a
/// message line of a file without a header (`1)  ...`).
pub fn is_trc(first: &str) -> bool {
    if first.starts_with(';') {
        return true;
    }
    let digits = first.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && first.as_bytes().get(digits) == Some(&b')')
}

/// Version and column letters from the header of a `.trc` file.
fn trc_header(text: &str) -> Result<((u8, u8), Vec<char>), String> {
    let mut ver = (1u8, 0u8);
    let mut cols: Option<Vec<char>> = None;
    for l in text.lines().map(str::trim) {
        if l.is_empty() {
            continue;
        }
        let Some(h) = l.strip_prefix(';') else { break };
        if let Some(v) = h.trim().strip_prefix("$FILEVERSION=") {
            let (a, b) = v.trim().split_once('.').ok_or_else(|| format!("bad $FILEVERSION \"{v}\""))?;
            ver = (a.parse().map_err(|_| format!("bad $FILEVERSION \"{v}\""))?, b.parse().map_err(|_| format!("bad $FILEVERSION \"{v}\""))?);
        } else if let Some(c) = h.trim().strip_prefix("$COLUMNS=") {
            let v: Vec<char> = c.split(',').filter_map(|s| s.trim().chars().next()).collect();
            if v.is_empty() {
                return Err("$COLUMNS is empty".to_string());
            }
            cols = Some(v);
        }
    }
    let cols = match ver {
        (1, 0..=3) => Vec::new(),
        (2 | 3, _) => {
            let c = cols.unwrap_or_else(|| if ver == (2, 0) { "NOTIdlD" } else { "NOTBIdRLD" }.chars().collect());
            for need in ['O', 'T', 'I'] {
                if !c.contains(&need) {
                    return Err(format!("$COLUMNS has no \"{need}\" column"));
                }
            }
            if !c.contains(&'l') && !c.contains(&'L') {
                return Err("$COLUMNS has no length column (\"l\" or \"L\")".to_string());
            }
            c
        }
        (a, b) => return Err(format!("unsupported .trc file version {a}.{b} (supported: 1.0 to 1.3, 2.0, 2.1, 3.0)")),
    };
    Ok((ver, cols))
}

/// Parse a PEAK `.trc` file (file version 1.0 to 1.3, 2.x, 3.0).
pub fn parse_trc(text: &str, progress: Option<&AtomicUsize>) -> Result<ParsedLog, String> {
    let (ver, cols) = trc_header(text)?;
    Ok(collect(text, progress, LogFormat::PeakTrc(ver.0, ver.1), |l| {
        if l.starts_with(';') {
            return Ok(Rec::None);
        }
        if ver.0 == 1 {
            trc1_line(l, ver.1)
        } else {
            trc2_line(l, &cols)
        }
    }))
}

/// Field `k` of a line; `k` moves on.
fn field<'a>(t: &[&'a str], k: &mut usize, what: &str) -> Result<&'a str, String> {
    let v = t.get(*k).copied().ok_or_else(|| format!("no {what}"))?;
    *k += 1;
    Ok(v)
}

/// A line of file version 1.x: `N) time [bus] [type] id [-] dlc data|RTR`.
fn trc1_line(l: &str, minor: u8) -> Result<Rec, String> {
    let t: Vec<&str> = l.split_whitespace().collect();
    let n = t.first().copied().unwrap_or("");
    if !n.ends_with(')') || n.len() < 2 {
        return Err("the line does not start with a message number \"N)\"".to_string());
    }
    let mut k = 1;
    let ts = field(&t, &mut k, "time")?;
    let time = parse_ms(ts).ok_or_else(|| format!("bad time \"{ts}\""))?;
    if minor >= 2 {
        field(&t, &mut k, "bus")?;
    }
    let mut dir = Dir::Rx;
    if minor >= 1 {
        match field(&t, &mut k, "message type")? {
            "Rx" => dir = Dir::Rx,
            "Tx" => dir = Dir::Tx,
            "Warng" | "Error" => return Ok(Rec::Skip),
            o => return Err(format!("unknown message type \"{o}\"")),
        }
    }
    let id_s = field(&t, &mut k, "id")?;
    if minor == 0 && id_s.eq_ignore_ascii_case("FFFFFFFF") {
        // Version 1.0 writes a bus warning with this ID.
        return Ok(Rec::Skip);
    }
    let (id, ext) = trc_id(id_s)?;
    if minor >= 3 {
        field(&t, &mut k, "reserved column")?;
    }
    let dlc = dlc_of(field(&t, &mut k, "DLC")?, 10)?;
    let rest = &t[k..];
    let rtr = rest.first() == Some(&"RTR");
    if rtr && rest.len() > 1 {
        return Err("an RTR frame has no data".to_string());
    }
    if !rtr && rest.len() != dlc as usize {
        return Err(format!("the data has {} byte(s), the DLC is {dlc}", rest.len()));
    }
    finish(frame(id, ext, rtr, dlc, time, dir), rest, 16).map(Rec::Frame)
}

/// A line of file version 2.x / 3.0 in the order of `$COLUMNS`.
fn trc2_line(l: &str, cols: &[char]) -> Result<Rec, String> {
    let t: Vec<&str> = l.split_whitespace().collect();
    let at = |c: char| cols.iter().position(|x| *x == c).and_then(|p| t.get(p).copied());
    let ty = at('T').ok_or("no message type")?;
    let rtr = match ty {
        "DT" => false,
        "RR" => true,
        "FD" | "FB" | "FE" | "BI" | "ST" | "EC" | "ER" | "EV" => return Ok(Rec::Skip),
        o => return Err(format!("unknown message type \"{o}\"")),
    };
    let ts = at('O').ok_or("no time offset")?;
    let time = parse_ms(ts).ok_or_else(|| format!("bad time \"{ts}\""))?;
    let (id, ext) = trc_id(at('I').ok_or("no id")?)?;
    let dir = match at('d') {
        None if !cols.contains(&'d') => Dir::Rx,
        Some("Rx") => Dir::Rx,
        Some("Tx") => Dir::Tx,
        Some(o) => return Err(format!("bad direction \"{o}\"")),
        None => return Err("no direction".to_string()),
    };
    let len_s = at('L').or_else(|| at('l')).ok_or("no data length")?;
    let dlc = dlc_of(len_s, 10)?;
    let data: &[&str] = match cols.iter().position(|x| *x == 'D') {
        Some(p) if p <= t.len() => &t[p..],
        _ => &[],
    };
    if rtr && !data.is_empty() {
        return Err("an RTR frame has no data".to_string());
    }
    if !rtr && data.len() != dlc as usize {
        return Err(format!("the data has {} byte(s), the DLC is {dlc}", data.len()));
    }
    finish(frame(id, ext, rtr, dlc, time, dir), data, 16).map(Rec::Frame)
}

/// Write `frames` as a `.trc` file of version 1.1 (CR LF line ends, time in
/// ms with one decimal, so the resolution is 100 µs). `start_unix_us` is the
/// wall time of the first frame.
pub fn write_trc(w: &mut impl Write, frames: &[CanFrame], start_unix_us: u64) -> std::io::Result<()> {
    let t0 = frames.first().map_or(0, |f| f.timestamp_us);
    // $STARTTIME: days since 1899-12-30 (25569 days before 1970-01-01).
    let days = 25569.0 + start_unix_us as f64 / 86_400e6;
    write!(w, ";$FILEVERSION=1.1\r\n;$STARTTIME={days:.10}\r\n;\r\n")?;
    write!(w, ";   Written by CANoxide {}\r\n", env!("CARGO_PKG_VERSION"))?;
    write!(w, ";   Start time: {} UTC\r\n", civil(start_unix_us).iso())?;
    write!(w, ";   Columns: message number, time offset (ms), type, ID (hex), DLC, data bytes (hex)\r\n;\r\n")?;
    for (n, f) in frames.iter().enumerate() {
        let off = f.timestamp_us.saturating_sub(t0) / 100;
        let id = if f.extended { format!("{:08X}", f.id) } else { format!("{:04X}", f.id) };
        let data = if f.rtr { "RTR".to_string() } else { f.payload().iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ") };
        write!(w, "{:>6}) {:>11}.{} {:<3} {id:>8}  {}  {data}\r\n", n + 1, off / 10, off % 10, f.dir.label(), f.dlc)?;
    }
    Ok(())
}

// ----------------------------------------------------------- Vector .asc

/// True for the first line of an `.asc` file (`date ...` or `base ...`).
pub fn is_asc(first: &str) -> bool {
    let l = first.to_ascii_lowercase();
    l.starts_with("date ") || l.starts_with("base ")
}

/// Parse a Vector `.asc` file.
pub fn parse_asc(text: &str, progress: Option<&AtomicUsize>) -> Result<ParsedLog, String> {
    let mut radix = 16u32;
    let mut relative = false;
    let mut last_us = 0u64;
    Ok(collect(text, progress, LogFormat::VectorAsc, |l| {
        let t: Vec<&str> = l.split_whitespace().collect();
        let first = t[0];
        let Some(dt) = replay_seconds(first) else {
            let w = first.to_ascii_lowercase();
            if w == "base" {
                radix = match t.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
                    Some("hex") => 16,
                    Some("dec") => 10,
                    o => return Err(format!("bad base \"{}\"", o.unwrap_or(""))),
                };
                if let Some(p) = t.iter().position(|s| s.eq_ignore_ascii_case("timestamps")) {
                    relative = match t.get(p + 1).map(|s| s.to_ascii_lowercase()).as_deref() {
                        Some("absolute") => false,
                        Some("relative") => true,
                        o => return Err(format!("bad timestamps \"{}\"", o.unwrap_or(""))),
                    };
                }
                return Ok(Rec::None);
            }
            if first.starts_with("//") || ["date", "begin", "end", "internal", "no"].contains(&w.as_str()) {
                return Ok(Rec::None);
            }
            return Err(format!("not a time stamp \"{first}\""));
        };
        let time = if relative { last_us.saturating_add(dt) } else { dt };
        last_us = time;
        asc_line(&t, time, radix)
    }))
}

fn replay_seconds(s: &str) -> Option<u64> {
    if !s.contains('.') {
        return None;
    }
    crate::replay::parse_seconds(s)
}

/// The fields after the time stamp: `ch id[x] Rx|Tx d dlc data...` or
/// `ch id[x] Rx|Tx r [dlc]`; other records are skipped.
fn asc_line(t: &[&str], time: u64, radix: u32) -> Result<Rec, String> {
    let ch = t.get(1).copied().ok_or("no channel")?;
    if !ch.bytes().all(|c| c.is_ascii_digit()) {
        // CANFD, events (Start of measurement, Trigger, SV: ...).
        return Ok(Rec::Skip);
    }
    let id_s = t.get(2).copied().ok_or("no id")?;
    let (digits, ext) = match id_s.strip_suffix(['x', 'X']) {
        Some(d) => (d, true),
        None => (id_s, false),
    };
    let id = if digits.is_empty() { None } else { u32::from_str_radix(digits, radix).ok() };
    let Some(id) = id else {
        // ErrorFrame, Statistic:, Status:, OverloadFrame and other records.
        if id_s.bytes().any(|c| c.is_ascii_alphabetic()) && id_s.len() > 3 {
            return Ok(Rec::Skip);
        }
        return Err(format!("bad id \"{id_s}\""));
    };
    let dir = match t.get(3).copied() {
        Some("Rx") => Dir::Rx,
        Some("Tx") => Dir::Tx,
        Some("TxRq") => return Ok(Rec::Skip),
        Some(o) => return Err(format!("bad direction \"{o}\"")),
        None => return Err("no direction".to_string()),
    };
    match t.get(4).copied() {
        Some("d") => {
            let dlc = dlc_of(t.get(5).copied().ok_or("no DLC")?, radix)?;
            finish(frame(id, ext, false, dlc, time, dir), &t[6..], radix).map(Rec::Frame)
        }
        Some("r") => {
            let dlc = t.get(5).and_then(|s| dlc_of(s, radix).ok()).unwrap_or(0);
            finish(frame(id, ext, true, dlc, time, dir), &[], radix).map(Rec::Frame)
        }
        Some(o) => Err(format!("expected \"d\" or \"r\", found \"{o}\"")),
        None => Err("no frame type (d or r)".to_string()),
    }
}

/// Write `frames` as an `.asc` file (`base hex`, absolute time stamps in s
/// with 6 decimals from the first frame, channel 1, CR LF line ends).
/// `start_unix_us` is the wall time of the first frame. An RTR frame is
/// written as `r <dlc>`.
pub fn write_asc(w: &mut impl Write, frames: &[CanFrame], start_unix_us: u64) -> std::io::Result<()> {
    let t0 = frames.first().map_or(0, |f| f.timestamp_us);
    let date = civil(start_unix_us).vector();
    write!(w, "date {date}\r\nbase hex  timestamps absolute\r\nno internal events logged\r\n")?;
    write!(w, "// written by CANoxide {}\r\nBegin Triggerblock {date}\r\n", env!("CARGO_PKG_VERSION"))?;
    for f in frames {
        let t = f.timestamp_us.saturating_sub(t0);
        let id = format!("{:X}{}", f.id, if f.extended { "x" } else { "" });
        let body = if f.rtr {
            format!("r {:X}", f.dlc)
        } else {
            let mut s = format!("d {:X}", f.dlc);
            for b in f.payload() {
                s.push_str(&format!(" {b:02X}"));
            }
            s
        };
        write!(w, "{:>4}.{:06} 1  {id:<15} {:<4} {body}\r\n", t / 1_000_000, t % 1_000_000, f.dir.label())?;
    }
    write!(w, "End TriggerBlock\r\n")
}

// ------------------------------------------------------------ wall time

struct Civil {
    y: i64,
    m: u32,
    d: u32,
    wd: u32,
    hh: u64,
    mm: u64,
    ss: u64,
    ms: u64,
}

/// UTC civil time of a Unix time in µs (H. Hinnant's days-to-civil).
fn civil(us: u64) -> Civil {
    let s = us / 1_000_000;
    let days = (s / 86400) as i64;
    let rem = s % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    // 1970-01-01 was a Thursday (4).
    let wd = ((days + 4).rem_euclid(7)) as u32;
    Civil { y, m, d, wd, hh: rem / 3600, mm: rem / 60 % 60, ss: rem % 60, ms: us / 1000 % 1000 }
}

impl Civil {
    fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", self.y, self.m, self.d, self.hh, self.mm, self.ss, self.ms)
    }

    /// `Wed Nov 25 10:00:00.000 am 2026` (12-hour clock).
    fn vector(&self) -> String {
        const WD: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MO: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let h12 = if self.hh.is_multiple_of(12) { 12 } else { self.hh % 12 };
        let ap = if self.hh < 12 { "am" } else { "pm" };
        format!("{} {} {} {:02}:{:02}:{:02}.{:03} {ap} {}", WD[self.wd as usize], MO[(self.m - 1) as usize], self.d, h12, self.mm, self.ss, self.ms, self.y)
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::parse_log;

    fn data(path: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/data/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn f(id: u32, ext: bool, rtr: bool, d: &[u8], t: u64, dir: Dir) -> CanFrame {
        let mut fr = CanFrame::new(id, d);
        fr.extended = ext;
        fr.rtr = rtr;
        fr.timestamp_us = t;
        fr.dir = dir;
        fr
    }

    #[test]
    fn detection() {
        assert!(is_trc(";$FILEVERSION=1.1"));
        assert!(is_trc("1)  1841  0001  8  00 00 00 00 00 00 00 00"));
        assert!(!is_trc("(1.0) can0 123#00"));
        assert!(is_asc("date Wed Nov 25 10:00:00.000 am 2026"));
        assert!(is_asc("base hex  timestamps absolute"));
        assert!(!is_asc("time_s,delta_ms"));
    }

    #[test]
    fn ms_parse() {
        assert_eq!(parse_ms("1059.9"), Some(1_059_900));
        assert_eq!(parse_ms("12"), Some(12_000));
        assert_eq!(parse_ms("0.1234"), Some(123));
        assert_eq!(parse_ms("x"), None);
        assert_eq!(parse_ms(".5"), None);
    }

    #[test]
    fn trc_v10() {
        let log = parse_log(&data("peak_v10.trc"), None).unwrap();
        assert_eq!(log.format, LogFormat::PeakTrc(1, 0));
        assert_eq!(log.frames.len(), 3);
        assert_eq!(log.skipped, 1, "the FFFFFFFF warning line");
        assert_eq!(log.frames[0], f(0x100, false, false, &[0x11, 0x22], 0, Dir::Rx));
        assert_eq!(log.frames[1].timestamp_us, 50_000);
        assert!(log.frames[2].extended);
        assert_eq!(log.frames[2].id, 0x18FF_0102);
    }

    #[test]
    fn trc_v11() {
        let log = parse_log(&data("peak_v11.trc"), None).unwrap();
        assert_eq!(log.format, LogFormat::PeakTrc(1, 1));
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        assert_eq!(log.frames.len(), 4);
        assert_eq!(log.skipped, 2, "Warng and Error lines");
        assert_eq!(log.frames[0], f(0x300, false, false, &[0, 0, 0, 0, 4, 0, 0, 0], 0, Dir::Rx));
        assert_eq!(log.frames[1].timestamp_us, 100_100);
        assert_eq!(log.frames[1].dir, Dir::Tx);
        let r = log.frames[2];
        assert!(r.rtr && r.dlc == 4 && r.id == 0x701);
        assert!(log.frames[3].extended && log.frames[3].id == 0x0CF0_0400);
    }

    #[test]
    fn trc_v13() {
        let log = parse_log(&data("peak_v13.trc"), None).unwrap();
        assert_eq!(log.format, LogFormat::PeakTrc(1, 3));
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        assert_eq!(log.frames.len(), 2);
        assert_eq!(log.frames[1], f(0x1234_5678, true, false, &[1, 2, 3], 1_500, Dir::Tx));
    }

    #[test]
    fn trc_v20() {
        let log = parse_log(&data("peak_v20.trc"), None).unwrap();
        assert_eq!(log.format, LogFormat::PeakTrc(2, 0));
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        assert_eq!(log.frames.len(), 3);
        assert_eq!(log.skipped, 3, "ER, ST and FD lines");
        assert_eq!(log.frames[0], f(0x181, false, false, &[0x01, 0x02], 0, Dir::Rx));
        assert!(log.frames[1].rtr && log.frames[1].dlc == 2);
        assert_eq!(log.frames[2].dir, Dir::Tx);
        assert!(log.frames[2].extended);
    }

    #[test]
    fn trc_v21_columns() {
        // The columns are in another order than the default: the parser
        // follows $COLUMNS.
        let log = parse_log(&data("peak_v21.trc"), None).unwrap();
        assert_eq!(log.format, LogFormat::PeakTrc(2, 1));
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        assert_eq!(log.frames.len(), 2);
        assert_eq!(log.frames[0], f(0x2A0, false, false, &[0xAA, 0xBB, 0xCC], 0, Dir::Tx));
        assert_eq!(log.frames[1].timestamp_us, 2_345);
        // Without $COLUMNS, 2.1 has the default N,O,T,B,I,d,R,L,D.
        let log = parse_log(";$FILEVERSION=2.1\n      1         7.000 DT 1     0100 Rx - 2    01 02\n", None).unwrap();
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        assert_eq!(log.frames[0], f(0x100, false, false, &[1, 2], 0, Dir::Rx));
    }

    #[test]
    fn trc_bad_lines() {
        let text = ";$FILEVERSION=1.1\n     1)         1.0 Rx 0100 2 11 22\n     2)         2.0 Rx 0100 2 11\n     3)         x.0 Rx 0100 1 11\n     4)         3.0 Qx 0100 1 11\n     5)         4.0 Rx 0100 9 11\n";
        let log = parse_log(text, None).unwrap();
        assert_eq!(log.frames.len(), 1);
        assert_eq!(log.bad_lines, 4);
        assert_eq!(log.errors[0].line, 3);
        assert!(log.errors[0].message.contains("DLC is 2"), "{}", log.errors[0]);
        let e = parse_log(";$FILEVERSION=4.0\n1) 1.0 Rx 0100 0\n", None).unwrap_err();
        assert!(e.contains("unsupported"), "{e}");
        let e = parse_log(";$FILEVERSION=2.0\n;$COLUMNS=N,T,I,d,l,D\n", None).unwrap_err();
        assert!(e.contains("\"O\""), "{e}");
    }

    #[test]
    fn asc_hex_absolute() {
        let log = parse_log(&data("vector_hex.asc"), None).unwrap();
        assert_eq!(log.format, LogFormat::VectorAsc);
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        assert_eq!(log.frames.len(), 4);
        assert_eq!(log.skipped, 4, "Start of measurement, ErrorFrame, CANFD, Statistic");
        assert_eq!(log.frames[0], f(0x123, false, false, &[0xDE, 0xAD], 0, Dir::Rx));
        assert_eq!(log.frames[1], f(0x18FE_F100, true, false, &[1, 2, 3, 4, 5, 6, 7, 8], 10_250, Dir::Tx));
        assert!(log.frames[2].rtr && log.frames[2].dlc == 0);
        assert!(log.frames[3].rtr && log.frames[3].dlc == 8);
    }

    #[test]
    fn asc_dec_relative() {
        let log = parse_log(&data("vector_dec_rel.asc"), None).unwrap();
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        assert_eq!(log.frames.len(), 3);
        assert_eq!(log.frames[0], f(291, false, false, &[255, 16], 0, Dir::Rx));
        // Relative: each stamp counts from the line before (also from a
        // skipped record).
        assert_eq!(log.frames[1].timestamp_us, 100_000);
        assert_eq!(log.frames[2].timestamp_us, 350_000);
        assert!(log.frames[2].extended && log.frames[2].id == 419_361_024);
        assert_eq!(log.skipped, 1);
    }

    #[test]
    fn asc_bad_lines() {
        let text = "date Wed Nov 25 10:00:00.000 am 2026\nbase hex  timestamps absolute\n0.1 1 100 Rx d 2 01 02\n0.2 1 100 Rx d 2 01\n0.3 1 100 Rx d 2 01 ZZ\n0.4 1 100 Qx d 1 01\nhello\n0.5 1 800 Rx d 0\n";
        let log = parse_log(text, None).unwrap();
        assert_eq!(log.frames.len(), 1);
        assert_eq!(log.bad_lines, 5, "{:?}", log.errors);
        assert_eq!(log.errors[0].line, 4);
        let e = parse_log("base oct\n", None).unwrap_err();
        assert!(e.contains("bad base"), "{e}");
    }

    fn sample() -> Vec<CanFrame> {
        vec![
            f(0x181, false, false, &[1, 2, 3, 4, 5, 6, 7, 8], 1_000_000, Dir::Rx),
            f(0x601, false, false, &[], 1_000_250, Dir::Tx),
            f(0x1ABC_DEF0, true, false, &[0xFF], 1_020_399, Dir::Rx),
            f(0x701, false, true, &[], 1_500_000, Dir::Rx),
            f(0x1FFF_FFFF, true, true, &[], 2_000_123, Dir::Tx),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, mut fr)| {
            if i == 3 {
                fr.dlc = 1;
            }
            fr
        })
        .collect()
    }

    fn same(a: &CanFrame, b: &CanFrame) {
        assert_eq!((a.id, a.extended, a.rtr, a.dlc, a.payload(), a.dir), (b.id, b.extended, b.rtr, b.dlc, b.payload(), b.dir));
    }

    #[test]
    fn trc_round_trip() {
        let fr = sample();
        let mut out = Vec::new();
        write_trc(&mut out, &fr, 1_764_064_800_000_000).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with(";$FILEVERSION=1.1\r\n;$STARTTIME=45986.4166666667\r\n"), "{text}");
        assert!(text.contains("Start time: 2025-11-25 10:00:00.000 UTC"));
        let log = parse_log(&text, None).unwrap();
        assert_eq!(log.format, LogFormat::PeakTrc(1, 1));
        assert_eq!((log.bad_lines, log.skipped), (0, 0), "{:?}", log.errors);
        assert_eq!(log.frames.len(), fr.len());
        for (a, b) in fr.iter().zip(&log.frames) {
            same(a, b);
            // 100 µs resolution, cut off.
            assert_eq!(b.timestamp_us, (a.timestamp_us - fr[0].timestamp_us) / 100 * 100);
        }
    }

    #[test]
    fn asc_round_trip() {
        let fr = sample();
        let mut out = Vec::new();
        write_asc(&mut out, &fr, 1_764_064_800_000_000).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("date Tue Nov 25 10:00:00.000 am 2025\r\nbase hex  timestamps absolute\r\n"), "{text}");
        let log = parse_log(&text, None).unwrap();
        assert_eq!(log.format, LogFormat::VectorAsc);
        assert_eq!((log.bad_lines, log.skipped), (0, 0), "{:?}", log.errors);
        assert_eq!(log.frames.len(), fr.len());
        for (a, b) in fr.iter().zip(&log.frames) {
            same(a, b);
            assert_eq!(b.timestamp_us, a.timestamp_us - fr[0].timestamp_us);
        }
    }

    #[test]
    fn vector_date_pm() {
        // 2026-01-01 13:05:09.250 UTC, a Thursday.
        assert_eq!(civil(1_767_272_709_250_000).vector(), "Thu Jan 1 01:05:09.250 pm 2026");
        assert_eq!(civil(0).vector(), "Thu Jan 1 12:00:00.000 am 1970");
    }
}
