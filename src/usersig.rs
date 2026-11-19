//! User signals (0.3): bit fields of a COB-ID that the user defines (by a
//! drag over the bit grid of the Messages view, or from Find signal), with
//! byte order, number type, factor, offset, unit and colour. They show in
//! the bit grid, the trace's decoded column and the Graph sources.
//!
//! File `user_signals.json` in the data folder:
//! `{ "version": 1, "signals": [ { "name", "cob_id", "extended",
//! "start_bit", "bits", "big_endian", "kind", "factor", "offset", "unit",
//! "colour" } ] }`. `start_bit` is the LSB for little endian and the MSB
//! (Motorola, as in DBC files) for big endian; bit k = byte * 8 + bit.

use crate::signals::{field_bits, raw_bits, raw_number, RawKind, SignalDef, SignalSource};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const USER_SIGNALS_FILE: &str = "user_signals.json";
pub const USER_SIGNALS_VERSION: u32 = 1;
/// Most user signals (all COB-IDs together).
pub const MAX_USER_SIGNALS: usize = 512;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserSignal {
    pub name: String,
    pub cob_id: u32,
    #[serde(default)]
    pub extended: bool,
    pub start_bit: u32,
    pub bits: u32,
    #[serde(default)]
    pub big_endian: bool,
    pub kind: RawKind,
    #[serde(default = "one")]
    pub factor: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub unit: String,
    /// sRGB.
    #[serde(default = "default_colour")]
    pub colour: [u8; 3],
}

fn one() -> f64 {
    1.0
}

fn default_colour() -> [u8; 3] {
    [0x3B, 0x82, 0xF6]
}

/// Colours offered in the Define signal dialog.
pub const COLOURS: [[u8; 3]; 8] = [
    [0x3B, 0x82, 0xF6],
    [0xF5, 0x9E, 0x0B],
    [0x10, 0xB9, 0x81],
    [0xEF, 0x44, 0x44],
    [0x8B, 0x5C, 0xF6],
    [0x06, 0xB6, 0xD4],
    [0xEC, 0x48, 0x99],
    [0x84, 0xCC, 0x16],
];

impl UserSignal {
    /// A 1-bit unsigned signal at bit `k` of `cob_id` (a starting point for
    /// the dialog).
    pub fn new(name: impl Into<String>, cob_id: u32, extended: bool, start_bit: u32, bits: u32) -> UserSignal {
        UserSignal { name: name.into(), cob_id, extended, start_bit, bits, big_endian: false, kind: RawKind::Unsigned, factor: 1.0, offset: 0.0, unit: String::new(), colour: default_colour() }
    }

    pub fn matches(&self, id: u32, extended: bool) -> bool {
        self.cob_id == id && self.extended == extended
    }

    /// Check the fields; the error names the first bad one.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("name: required".to_string());
        }
        let max = if self.extended { 0x1FFF_FFFF } else { 0x7FF };
        if self.cob_id > max {
            return Err(format!("COB-ID above 0x{max:X}"));
        }
        if self.bits == 0 || self.bits > 64 {
            return Err("length: 1..64 bits".to_string());
        }
        if self.kind == RawKind::Float && self.bits != 32 {
            return Err("a float32 has 32 bits".to_string());
        }
        if field_bits(self.start_bit, self.bits, self.big_endian).is_none() {
            return Err("the field must fit in 8 data bytes".to_string());
        }
        if !self.factor.is_finite() || self.factor == 0.0 || !self.offset.is_finite() {
            return Err("factor and offset: finite numbers, factor not 0".to_string());
        }
        Ok(())
    }

    /// The bit indices of the field (empty when it does not fit).
    pub fn bit_list(&self) -> Vec<u32> {
        field_bits(self.start_bit, self.bits, self.big_endian).unwrap_or_default()
    }

    /// The field before factor and offset (`None`: the frame is too short).
    pub fn raw(&self, data: &[u8]) -> Option<f64> {
        raw_number(raw_bits(data, self.start_bit, self.bits, self.big_endian)?, self.bits, self.kind)
    }

    /// `raw * factor + offset`.
    pub fn value(&self, data: &[u8]) -> Option<f64> {
        Some(self.raw(data)? * self.factor + self.offset)
    }

    /// "name=value unit" for the trace's decoded column.
    pub fn text(&self, data: &[u8]) -> Option<String> {
        let v = self.value(data)?;
        let u = if self.unit.is_empty() { String::new() } else { format!(" {}", self.unit) };
        Some(format!("{}={}{u}", self.name, fmt_value(v)))
    }

    /// The Graph signal of this field.
    pub fn to_def(&self) -> SignalDef {
        let source = SignalSource::Raw { cob_id: self.cob_id, extended: self.extended, start_bit: self.start_bit, bits: self.bits, raw_kind: self.kind, big_endian: self.big_endian };
        let mut d = SignalDef::new(self.name.clone(), source);
        d.unit = self.unit.clone();
        d.scale = self.factor;
        d.offset = self.offset;
        d
    }
}

/// A value as text: integers without a fraction, else up to 4 decimals.
pub fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.4}");
        let s = s.trim_end_matches('0');
        s.trim_end_matches('.').to_string()
    }
}

#[derive(Serialize, Deserialize)]
struct UserSignalsFile {
    version: u32,
    #[serde(default)]
    signals: Vec<UserSignal>,
}

/// Load the user signals. A missing file gives none. A broken file (or one
/// of a newer version) gives none and a note; the file is copied to
/// `user_signals.json.bad` first, as the library does with its file.
pub fn load(path: &Path) -> (Vec<UserSignal>, Option<String>) {
    let b = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), None),
        Err(e) => return (Vec::new(), Some(format!("{} not read: {e}", path.display()))),
    };
    let bad = |why: String| {
        let backup = path.with_extension("json.bad");
        let _ = std::fs::copy(path, &backup);
        (Vec::new(), Some(format!("{} could not be read ({why}); a copy is in {}", path.display(), backup.display())))
    };
    match serde_json::from_slice::<UserSignalsFile>(&b) {
        Ok(f) if f.version > USER_SIGNALS_VERSION => bad(format!("version {} is newer than {USER_SIGNALS_VERSION}", f.version)),
        Ok(f) => {
            let mut v: Vec<UserSignal> = f.signals.into_iter().filter(|s| s.validate().is_ok()).collect();
            v.truncate(MAX_USER_SIGNALS);
            (v, None)
        }
        Err(e) => bad(e.to_string()),
    }
}

pub fn save(path: &Path, signals: &[UserSignal]) -> Result<(), String> {
    let f = UserSignalsFile { version: USER_SIGNALS_VERSION, signals: signals.to_vec() };
    let t = serde_json::to_vec_pretty(&f).map_err(|e| e.to_string())?;
    crate::library::write_atomic(path, &t).map_err(|e| e.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_value_across_a_byte_border() {
        // Bits 4..12 (LE): low nibble from byte 0 bits 4..7, high from byte 1 bits 0..3.
        let mut s = UserSignal::new("x", 0x181, false, 4, 8);
        let data = [0xA0, 0x0B, 0, 0, 0, 0, 0, 0];
        assert_eq!(s.raw(&data), Some(0xBA as f64));
        s.kind = RawKind::Signed;
        assert_eq!(s.raw(&data), Some((0xBAu8 as i8) as f64));
        s.factor = 0.5;
        s.offset = 10.0;
        assert_eq!(s.value(&data), Some((0xBAu8 as i8) as f64 * 0.5 + 10.0));
        assert_eq!(s.bit_list(), (4..12).collect::<Vec<_>>());
    }

    #[test]
    fn be_value_across_a_byte_border_and_signed() {
        // Motorola 12 bit from bit 3 (MSB) of byte 0: byte 0 bits 3..0, then byte 1 bits 7..0.
        let mut s = UserSignal::new("x", 0x181, false, 3, 12);
        s.big_endian = true;
        let data = [0x0F, 0xFE, 0, 0, 0, 0, 0, 0];
        assert_eq!(s.raw(&data), Some(0xFFE as f64));
        s.kind = RawKind::Signed;
        assert_eq!(s.raw(&data), Some(-2.0));
        assert_eq!(s.bit_list(), vec![3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8]);
        // 16 bit big endian from bit 7: bytes 0, 1 as MSB, LSB.
        let mut w = UserSignal::new("w", 0x181, false, 7, 16);
        w.big_endian = true;
        assert_eq!(w.raw(&[0x12, 0x34]), Some(0x1234 as f64));
        // Too short frame: no value.
        assert_eq!(w.raw(&[0x12]), None);
        // Does not fit: start at bit 56 (byte 7) and 16 bits big endian.
        w.start_bit = 63;
        assert!(w.validate().is_err());
    }

    #[test]
    fn float32_and_text() {
        let mut s = UserSignal::new("t", 0x281, false, 0, 32);
        s.kind = RawKind::Float;
        s.unit = "°C".to_string();
        let data = 21.5f32.to_le_bytes();
        assert_eq!(s.text(&data).as_deref(), Some("t=21.5 °C"));
        s.bits = 16;
        assert!(s.validate().unwrap_err().contains("float32"));
    }

    #[test]
    fn file_round_trip_missing_and_broken() {
        let dir = std::env::temp_dir().join(format!("canoxide_usersig_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(USER_SIGNALS_FILE);
        let _ = std::fs::remove_file(&p);
        assert_eq!(load(&p), (Vec::new(), None));
        let mut a = UserSignal::new("speed", 0x181, false, 8, 16);
        a.factor = 0.1;
        a.unit = "km/h".to_string();
        let mut b = UserSignal::new("flag", 0x1FFF_0000, true, 63, 1);
        b.big_endian = true;
        save(&p, &[a.clone(), b.clone()]).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("\"version\": 1"));
        assert_eq!(load(&p), (vec![a, b], None));
        std::fs::write(&p, b"{ broken").unwrap();
        let (v, note) = load(&p);
        assert!(v.is_empty() && note.unwrap().contains("a copy is in"));
        assert!(dir.join("user_signals.json.bad").exists());
        std::fs::write(&p, br#"{"version": 9, "signals": []}"#).unwrap();
        assert!(load(&p).1.unwrap().contains("newer"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
