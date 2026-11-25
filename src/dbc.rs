//! DBC import: the `BO_` messages and `SG_` signals of a DBC file as user
//! signals (`crate::usersig`), and the merge into the user-signal store.
//!
//! Read: `BO_ <id> <name>: <dlc> <node>` (bit 31 of the ID set = a 29-bit
//! ID; the bit is removed), `SG_ <name> : <start>|<length>@<order><sign>
//! (<factor>,<offset>) [<min>|<max>] "<unit>" <receivers>` and
//! `SIG_VALTYPE_ <id> <signal> : <1|2>;`. `BA_ "ProtocolType" "J1939"` and
//! the `VFrameFormat` value `J1939PG` mark a J1939 file (`DbcImport::j1939`).
//! All other keywords (`VAL_`, other `BA_`, `CM_`, ...) are ignored. Multiplexed signals (`M`, `m<n>`) are
//! skipped and counted.
//!
//! Bit numbering: a DBC start bit is the LSB for `@1` (Intel) and the MSB in
//! the "sawtooth" numbering (bit k = byte * 8 + bit, bit 7 = MSB of a byte)
//! for `@0` (Motorola). `UserSignal::start_bit` uses the same convention, so
//! the start bit is taken over unchanged for both byte orders.

use crate::signals::RawKind;
use crate::usersig::{MatchMode, UserSignal, COLOURS, MAX_USER_SIGNALS};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// A message of a DBC file with the signals that can be user signals.
#[derive(Clone, Debug, PartialEq)]
pub struct DbcMessage {
    pub id: u32,
    pub extended: bool,
    pub name: String,
    pub dlc: u8,
    pub signals: Vec<UserSignal>,
}

/// The result of a DBC file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DbcImport {
    pub messages: Vec<DbcMessage>,
    /// One note per skipped message or signal.
    pub skipped: Vec<String>,
    /// Multiplexed signals (also in `skipped`).
    pub multiplexed: usize,
    /// The file is a J1939 database: `BA_ "ProtocolType" "J1939"`, or a
    /// message with the `VFrameFormat` value `J1939PG`.
    pub j1939: bool,
}

impl DbcImport {
    pub fn signal_count(&self) -> usize {
        self.messages.iter().map(|m| m.signals.len()).sum()
    }

    /// All signals of all messages, in file order.
    pub fn signals(&self) -> impl Iterator<Item = &UserSignal> {
        self.messages.iter().flat_map(|m| m.signals.iter())
    }

    /// The signals of the extended messages match by the J1939 PGN. The
    /// source address of the message ID becomes the SA filter; SA 254 (the
    /// usual placeholder of a J1939 DBC) matches any source address.
    pub fn set_pgn_match(&mut self, on: bool) {
        for m in self.messages.iter_mut().filter(|m| m.extended) {
            for s in &mut m.signals {
                if on {
                    s.match_mode = MatchMode::J1939Pgn;
                    let sa = (m.id & 0xFF) as u8;
                    s.source = (sa != crate::j1939::NULL_ADDR).then_some(sa);
                } else {
                    s.match_mode = MatchMode::Exact;
                    s.source = None;
                }
            }
        }
    }
}

/// Read a DBC file (UTF-8, else Windows-1252).
pub fn load(path: &Path) -> Result<DbcImport, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&crate::eds::decode_text(&b))
}

struct RawSignal {
    sig: UserSignal,
    /// The message ID as written in the file (with bit 31).
    file_id: u32,
}

/// Parse the text of a DBC file. `Err` when it holds no `BO_` line.
pub fn parse(text: &str) -> Result<DbcImport, String> {
    let mut out = DbcImport::default();
    let mut raw: Vec<RawSignal> = Vec::new();
    // The current message: (file ID, index in out.messages or None when the
    // message is skipped).
    let mut cur: Option<(u32, Option<usize>)> = None;
    let mut valtype: HashMap<(u32, String), u8> = HashMap::new();
    let mut in_string = false;
    let mut seen_bo = false;
    // The index of "J1939PG" in the VFrameFormat enum, and the values used.
    let mut vframe_j1939: Option<usize> = None;
    let mut vframe_values: Vec<usize> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let quotes = line.matches('"').count() - line.matches("\\\"").count();
        if in_string {
            in_string = quotes % 2 == 0;
            continue;
        }
        if quotes % 2 == 1 {
            // A string (CM_, BA_DEF_ ...) that goes on in the next lines.
            in_string = true;
        }
        let l = line.trim();
        let kw = l.split_whitespace().next().unwrap_or("");
        match kw {
            "BO_" => {
                seen_bo = true;
                let (file_id, name, dlc) = parse_bo(l).map_err(|e| format!("line {n}: {e}"))?;
                let extended = file_id & 0x8000_0000 != 0;
                let id = file_id & 0x7FFF_FFFF;
                let max = if extended { 0x1FFF_FFFF } else { 0x7FF };
                if id > max {
                    // Also VECTOR__INDEPENDENT_SIG_MSG (0xC0000000).
                    out.skipped.push(format!("message {name} (ID 0x{file_id:X}): not a CAN ID; its signals are skipped"));
                    cur = Some((file_id, None));
                } else {
                    out.messages.push(DbcMessage { id, extended, name, dlc, signals: Vec::new() });
                    cur = Some((file_id, Some(out.messages.len() - 1)));
                }
            }
            "SG_" => {
                let Some((file_id, idx)) = cur else {
                    out.skipped.push(format!("line {n}: a signal outside a message"));
                    continue;
                };
                let Some(idx) = idx else { continue };
                let m = &out.messages[idx];
                match parse_sg(l, m.id, m.extended) {
                    Ok(SgResult::Signal(sig)) => raw.push(RawSignal { sig, file_id }),
                    Ok(SgResult::Multiplexed(name)) => {
                        out.multiplexed += 1;
                        out.skipped.push(format!("{} signal {name}: multiplexed signals are not supported", m.name));
                    }
                    Err(e) => out.skipped.push(format!("line {n}: {} signal: {e}", m.name)),
                }
            }
            "SIG_VALTYPE_" => {
                // SIG_VALTYPE_ <id> <signal> : <1|2> ;
                let t: Vec<&str> = l.trim_end_matches(';').split_whitespace().filter(|s| *s != ":").collect();
                let id = t.get(1).and_then(|s| s.parse::<u32>().ok());
                let name = t.get(2).map(|s| s.trim_end_matches(':').to_string());
                let v = t.last().and_then(|s| s.trim_start_matches(':').parse::<u8>().ok());
                match (id, name, v) {
                    (Some(id), Some(name), Some(v)) => {
                        valtype.insert((id, name), v);
                    }
                    _ => out.skipped.push(format!("line {n}: SIG_VALTYPE_ not understood")),
                }
            }
            "BA_DEF_" if l.contains("\"VFrameFormat\"") => {
                // BA_DEF_ BO_ "VFrameFormat" ENUM "StandardCAN","ExtendedCAN",...,"J1939PG";
                if let Some((_, list)) = l.split_once("ENUM") {
                    let names: Vec<&str> = list.trim_end_matches(';').split(',').map(|s| s.trim().trim_matches('"')).collect();
                    vframe_j1939 = names.iter().position(|s| *s == "J1939PG");
                }
                cur = None;
            }
            "BA_" => {
                let t: Vec<&str> = l.trim_end_matches(';').split_whitespace().collect();
                match t.get(1).copied() {
                    Some("\"ProtocolType\"") if t.get(2).is_some_and(|v| v.trim_matches('"') == "J1939") => out.j1939 = true,
                    Some("\"VFrameFormat\"") => vframe_values.extend(t.last().and_then(|v| v.parse::<usize>().ok())),
                    _ => {}
                }
                cur = None;
            }
            "" => {}
            // An indented line that is not SG_ ends the message.
            _ => cur = None,
        }
    }
    if vframe_j1939.is_some_and(|k| vframe_values.contains(&k)) {
        out.j1939 = true;
    }
    if !seen_bo {
        return Err("no message (BO_) found; is this a DBC file?".to_string());
    }
    let mut colour = 0usize;
    for RawSignal { mut sig, file_id } in raw {
        let m = out.messages.iter_mut().find(|m| m.id == sig.cob_id && m.extended == sig.extended).expect("message of the signal");
        match valtype.get(&(file_id, sig.name.clone())) {
            Some(1) => sig.kind = RawKind::Float,
            Some(2) if sig.bits == 32 => sig.kind = RawKind::Float,
            Some(2) => {
                out.skipped.push(format!("{} signal {}: float64 ({} bits) is not supported (only float32)", m.name, sig.name, sig.bits));
                continue;
            }
            _ => {}
        }
        if let Err(e) = sig.validate() {
            out.skipped.push(format!("{} signal {}: {e}", m.name, sig.name));
            continue;
        }
        if m.signals.iter().any(|s| s.name == sig.name) {
            out.skipped.push(format!("{} signal {}: the name is used twice", m.name, sig.name));
            continue;
        }
        sig.colour = COLOURS[colour % COLOURS.len()];
        colour += 1;
        m.signals.push(sig);
    }
    Ok(out)
}

/// `BO_ <id> <name>: <dlc> <node>`.
fn parse_bo(l: &str) -> Result<(u32, String, u8), String> {
    let t: Vec<&str> = l.split_whitespace().collect();
    let id = t.get(1).and_then(|s| s.parse::<u32>().ok()).ok_or("BO_: bad message ID")?;
    let raw_name = t.get(2).ok_or("BO_: no message name")?;
    let (name, dlc_tok) = match raw_name.strip_suffix(':') {
        Some(n) => (n, t.get(3)),
        None => (*raw_name, t.get(4)),
    };
    let dlc = dlc_tok.and_then(|s| s.parse::<u8>().ok()).ok_or("BO_: bad DLC")?;
    Ok((id, name.to_string(), dlc))
}

enum SgResult {
    Signal(UserSignal),
    Multiplexed(String),
}

/// `SG_ <name> [M|m<n>] : <start>|<len>@<order><sign> (<f>,<o>) [<min>|<max>] "<unit>" ...`.
fn parse_sg(l: &str, id: u32, extended: bool) -> Result<SgResult, String> {
    let body = l.strip_prefix("SG_").unwrap_or(l);
    let (head, rest) = body.split_once(':').ok_or("no \":\"")?;
    let mut h = head.split_whitespace();
    let name = h.next().ok_or("no name")?.to_string();
    if h.next().is_some() {
        return Ok(SgResult::Multiplexed(name));
    }
    let rest = rest.trim();
    let (layout, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let (start, rest_l) = layout.split_once('|').ok_or_else(|| format!("{name}: bad bit layout \"{layout}\""))?;
    let (len, ord) = rest_l.split_once('@').ok_or_else(|| format!("{name}: bad bit layout \"{layout}\""))?;
    let start: u32 = start.parse().map_err(|_| format!("{name}: bad start bit \"{start}\""))?;
    let bits: u32 = len.parse().map_err(|_| format!("{name}: bad length \"{len}\""))?;
    let mut oc = ord.chars();
    let big_endian = match oc.next() {
        Some('0') => true,
        Some('1') => false,
        _ => return Err(format!("{name}: bad byte order in \"{layout}\"")),
    };
    let kind = match oc.next() {
        Some('+') => RawKind::Unsigned,
        Some('-') => RawKind::Signed,
        _ => return Err(format!("{name}: bad sign in \"{layout}\"")),
    };
    let (fo, rest) = between(rest, '(', ')').ok_or_else(|| format!("{name}: no (factor,offset)"))?;
    let (f, o) = fo.split_once(',').ok_or_else(|| format!("{name}: bad (factor,offset)"))?;
    let factor: f64 = f.trim().parse().map_err(|_| format!("{name}: bad factor \"{f}\""))?;
    let offset: f64 = o.trim().parse().map_err(|_| format!("{name}: bad offset \"{o}\""))?;
    let unit = between(rest, '"', '"').map(|(u, _)| u.to_string()).unwrap_or_default();
    let mut s = UserSignal::new(name, id, extended, start, bits);
    s.big_endian = big_endian;
    s.kind = kind;
    s.factor = factor;
    s.offset = offset;
    s.unit = unit;
    Ok(SgResult::Signal(s))
}

/// The text between the first `a` and the next `b`, and the text after `b`.
fn between(s: &str, a: char, b: char) -> Option<(&str, &str)> {
    let i = s.find(a)?;
    let r = &s[i + a.len_utf8()..];
    let j = r.find(b)?;
    Some((&r[..j], &r[j + b.len_utf8()..]))
}

/// What a merge did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub added: usize,
    /// Existing signals removed by "Replace".
    pub removed: usize,
    /// Not added: a signal of the same name on the same ID exists ("Add").
    pub same_name: usize,
    /// Not added: the store is full (`MAX_USER_SIGNALS`).
    pub over_limit: usize,
}

impl MergeReport {
    pub fn text(&self) -> String {
        let mut s = format!("{} signal(s) added", self.added);
        if self.removed > 0 {
            s.push_str(&format!(", {} replaced signal(s) removed", self.removed));
        }
        if self.same_name > 0 {
            s.push_str(&format!(", {} kept (same name on the same ID)", self.same_name));
        }
        if self.over_limit > 0 {
            s.push_str(&format!(", {} not added (at most {MAX_USER_SIGNALS} user signals)", self.over_limit));
        }
        s
    }
}

/// Merge `import` into `existing`. `replace = false` ("Add"): keep every
/// existing signal and skip a new one with the same name on the same ID.
/// `replace = true`: first remove every existing signal of the IDs that the
/// import has signals for.
pub fn merge(existing: &[UserSignal], import: &DbcImport, replace: bool) -> (Vec<UserSignal>, MergeReport) {
    let mut rep = MergeReport::default();
    let ids: BTreeSet<(u32, bool)> = import.messages.iter().filter(|m| !m.signals.is_empty()).map(|m| (m.id, m.extended)).collect();
    let mut out: Vec<UserSignal> = if replace {
        let kept: Vec<UserSignal> = existing.iter().filter(|s| !ids.contains(&(s.cob_id, s.extended))).cloned().collect();
        rep.removed = existing.len() - kept.len();
        kept
    } else {
        existing.to_vec()
    };
    for s in import.signals() {
        if out.iter().any(|e| e.matches(s.cob_id, s.extended) && e.name == s.name) {
            rep.same_name += 1;
        } else if out.len() >= MAX_USER_SIGNALS {
            rep.over_limit += 1;
        } else {
            out.push(s.clone());
            rep.added += 1;
        }
    }
    (out, rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DBC: &str = r#"VERSION ""

NS_ :
	NS_DESC_
	CM_

BS_:

BU_: ECU1 ECU2

BO_ 291 Engine: 8 ECU1
 SG_ S : 7|16@0+ (1,0) [0|65535] "" ECU2
 SG_ T : 3|12@0+ (0.5,-10) [0|0] "degC" ECU2
 SG_ Speed : 12|10@1+ (0.1,0) [0|102.3] "km/h" ECU2
 SG_ Torque : 32|16@1- (1,0) [-32768|32767] "Nm" ECU2

BO_ 2566844672 Ext: 8 ECU2
 SG_ Temp : 0|32@1- (1,0) [0|0] "°C" ECU1
 SG_ Mode M : 32|8@1+ (1,0) [0|255] "" ECU1
 SG_ A m0 : 40|8@1+ (1,0) [0|255] "" ECU1
 SG_ D : 0|64@1- (1,0) [0|0] "" ECU1

BO_ 3221225472 VECTOR__INDEPENDENT_SIG_MSG: 0 Vector__XXX
 SG_ Orphan : 0|8@1+ (1,0) [0|0] "" Vector__XXX

CM_ SG_ 291 S "a comment
SG_ Fake : 0|8@1+ (1,0) [0|0] "" X
that runs over lines";
BA_DEF_ SG_ "GenSigStartValue" INT 0 10000;
VAL_ 291 S 0 "off" 1 "on" ;
SIG_VALTYPE_ 2566844672 Temp : 1;
SIG_VALTYPE_ 2566844672 D : 2;
"#;

    fn sig<'a>(i: &'a DbcImport, name: &str) -> &'a UserSignal {
        i.signals().find(|s| s.name == name).unwrap()
    }

    #[test]
    fn messages_signals_and_skipped() {
        let i = parse(DBC).unwrap();
        assert_eq!(i.messages.len(), 2);
        assert_eq!((i.messages[0].id, i.messages[0].extended, i.messages[0].dlc), (291, false, 8));
        // 2566844672 = 0x98FEF100: bit 31 set, ID 0x18FEF100.
        assert_eq!((i.messages[1].id, i.messages[1].extended), (0x18FE_F100, true));
        assert_eq!(i.signal_count(), 5, "S, T, Speed, Torque, Temp");
        assert_eq!(i.multiplexed, 2);
        let sk = i.skipped.join("\n");
        assert!(sk.contains("Mode: multiplexed"), "{sk}");
        assert!(sk.contains("D: float64"), "{sk}");
        assert!(sk.contains("VECTOR__INDEPENDENT_SIG_MSG"), "{sk}");
        assert!(!sk.contains("Fake") && i.signals().all(|s| s.name != "Fake"), "a line inside a comment string");
        assert_eq!(i.skipped.len(), 4);
        let t = sig(&i, "T");
        assert_eq!((t.factor, t.offset, t.unit.as_str(), t.big_endian), (0.5, -10.0, "degC", true));
        assert_eq!(sig(&i, "Temp").kind, RawKind::Float);
        assert_eq!(sig(&i, "Temp").unit, "°C");
        assert_ne!(sig(&i, "S").colour, sig(&i, "T").colour);
    }

    #[test]
    fn motorola_16_bit_from_7() {
        // 7|16@0+: bytes 0 and 1, byte 0 = high byte.
        let i = parse(DBC).unwrap();
        let s = sig(&i, "S");
        assert_eq!((s.start_bit, s.bits, s.big_endian), (7, 16, true));
        assert_eq!(s.raw(&[0x12, 0x34, 0, 0, 0, 0, 0, 0]), Some(0x1234 as f64));
        assert_eq!(s.raw(&[0xFF, 0x01, 0, 0, 0, 0, 0, 0]), Some(0xFF01 as f64));
    }

    #[test]
    fn motorola_12_bit_from_3() {
        // 3|12@0+: low nibble of byte 0 (high bits), then all of byte 1.
        let i = parse(DBC).unwrap();
        let s = sig(&i, "T");
        assert_eq!(s.raw(&[0xA5, 0xC3, 0, 0, 0, 0, 0, 0]), Some(0x5C3 as f64));
        assert_eq!(s.value(&[0xA5, 0xC3, 0, 0, 0, 0, 0, 0]), Some(0x5C3 as f64 * 0.5 - 10.0));
        assert_eq!(s.bit_list(), vec![3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8]);
    }

    #[test]
    fn intel_across_a_byte_border() {
        // 12|10@1+: bits 12..15 of byte 1 (low part), bits 0..5 of byte 2.
        let i = parse(DBC).unwrap();
        let s = sig(&i, "Speed");
        let data = [0x00, 0xB0, 0x2A, 0, 0, 0, 0, 0];
        // raw = (0x2A & 0x3F) << 4 | 0xB = 0x2AB
        assert_eq!(s.raw(&data), Some(0x2AB as f64));
        assert!((s.value(&data).unwrap() - 68.3).abs() < 1e-9);
    }

    #[test]
    fn signed_intel_and_float32() {
        let i = parse(DBC).unwrap();
        let s = sig(&i, "Torque");
        assert_eq!(s.kind, RawKind::Signed);
        assert_eq!(s.raw(&[0, 0, 0, 0, 0x18, 0xFC, 0, 0]), Some(-1000.0));
        let t = sig(&i, "Temp");
        assert_eq!(t.raw(&(-12.5f32).to_le_bytes()), Some(-12.5));
    }

    #[test]
    fn errors_and_windows_1252() {
        assert!(parse("VERSION \"\"\n").unwrap_err().contains("no message"));
        let e = parse("BO_ x A: 8 N\n").unwrap_err();
        assert!(e.contains("line 1"), "{e}");
        let i = parse("BO_ 100 A: 8 N\n SG_ X : 60|16@1+ (1,0) [0|0] \"\" N\n SG_ Y : 0|8@1+ (0,0) [0|0] \"\" N\n SG_ Z : 0|8@2+ (1,0) [0|0] \"\" N\n").unwrap();
        assert_eq!(i.signal_count(), 0);
        assert_eq!(i.skipped.len(), 3, "{:?}", i.skipped);
        // Windows-1252 degree sign (0xB0).
        let dir = std::env::temp_dir().join(format!("canoxide_dbc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("w.dbc");
        std::fs::write(&p, b"BO_ 100 A: 8 N\n SG_ T : 0|8@1+ (1,0) [0|0] \"\xB0C\" N\n").unwrap();
        assert_eq!(load(&p).unwrap().messages[0].signals[0].unit, "°C");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn j1939_detection_and_pgn_match() {
        assert!(!parse(DBC).unwrap().j1939);
        let base = "BO_ 2364540158 PgA: 8 N\n SG_ V : 24|16@1+ (1,0) [0|0] \"\" N\n\nBO_ 2566843937 PgB: 8 N\n SG_ W : 0|8@1+ (1,0) [0|0] \"\" N\n\nBO_ 100 Std: 8 N\n SG_ X : 0|8@1+ (1,0) [0|0] \"\" N\n";
        let p = format!("{base}BA_DEF_  \"ProtocolType\" STRING ;\nBA_ \"ProtocolType\" \"J1939\";\n");
        assert!(parse(&p).unwrap().j1939);
        let v = format!("{base}BA_DEF_ BO_ \"VFrameFormat\" ENUM \"StandardCAN\",\"ExtendedCAN\",\"reserved\",\"J1939PG\";\nBA_ \"VFrameFormat\" BO_ 2364540158 3;\n");
        assert!(parse(&v).unwrap().j1939);
        let e = format!("{base}BA_DEF_ BO_ \"VFrameFormat\" ENUM \"StandardCAN\",\"ExtendedCAN\",\"reserved\",\"J1939PG\";\nBA_ \"VFrameFormat\" BO_ 2364540158 1;\n");
        assert!(!parse(&e).unwrap().j1939, "ExtendedCAN only");
        let mut i = parse(&p).unwrap();
        i.set_pgn_match(true);
        // 2364540158 = 0x8CF004FE: SA 254 = any; 0x98FEEE21: SA 33.
        assert_eq!((sig(&i, "V").match_mode, sig(&i, "V").source), (MatchMode::J1939Pgn, None));
        assert_eq!((sig(&i, "W").match_mode, sig(&i, "W").source), (MatchMode::J1939Pgn, Some(33)));
        assert_eq!(sig(&i, "X").match_mode, MatchMode::Exact, "a standard ID stays exact");
        assert!(sig(&i, "V").matches_frame(0x0CF0_0400, true, true));
        assert!(!sig(&i, "W").matches_frame(0x18FE_EE00, true, true));
        i.set_pgn_match(false);
        assert_eq!((sig(&i, "V").match_mode, sig(&i, "W").source), (MatchMode::Exact, None));
    }

    #[test]
    fn merge_add_and_replace() {
        let i = parse(DBC).unwrap();
        let mut own = UserSignal::new("S", 291, false, 0, 8);
        own.unit = "mine".to_string();
        let other = UserSignal::new("Q", 291, false, 8, 8);
        let far = UserSignal::new("F", 0x181, false, 0, 8);
        let existing = vec![own.clone(), other.clone(), far.clone()];
        let (v, r) = merge(&existing, &i, false);
        assert_eq!(r, MergeReport { added: 4, removed: 0, same_name: 1, over_limit: 0 });
        assert_eq!(v.len(), 7);
        assert_eq!(v[0], own, "Add keeps the existing signal of the same name");
        let (v, r) = merge(&existing, &i, true);
        assert_eq!(r, MergeReport { added: 5, removed: 2, same_name: 0, over_limit: 0 });
        assert_eq!(v[0], far);
        assert_eq!(v.len(), 6);
        assert!(r.text().contains("2 replaced"));
        let full: Vec<UserSignal> = (0..MAX_USER_SIGNALS).map(|k| UserSignal::new(format!("x{k}"), 0x100, false, 0, 1)).collect();
        let (v, r) = merge(&full, &i, false);
        assert_eq!((v.len(), r.over_limit), (MAX_USER_SIGNALS, 5));
    }
}
