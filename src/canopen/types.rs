//! CANopen basic data types (CiA 301, 0x0001..0x001B): bytes <-> value,
//! text -> bytes (little endian), limits and display formatting.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataType {
    Boolean,
    Integer8,
    Integer16,
    Integer32,
    Unsigned8,
    Unsigned16,
    Unsigned32,
    Real32,
    VisibleString,
    OctetString,
    UnicodeString,
    TimeOfDay,
    TimeDifference,
    Domain,
    Integer24,
    Real64,
    Integer40,
    Integer48,
    Integer56,
    Integer64,
    Unsigned24,
    Unsigned40,
    Unsigned48,
    Unsigned56,
    Unsigned64,
}

impl DataType {
    pub const ALL: [DataType; 25] = [
        DataType::Boolean,
        DataType::Integer8,
        DataType::Integer16,
        DataType::Integer24,
        DataType::Integer32,
        DataType::Integer40,
        DataType::Integer48,
        DataType::Integer56,
        DataType::Integer64,
        DataType::Unsigned8,
        DataType::Unsigned16,
        DataType::Unsigned24,
        DataType::Unsigned32,
        DataType::Unsigned40,
        DataType::Unsigned48,
        DataType::Unsigned56,
        DataType::Unsigned64,
        DataType::Real32,
        DataType::Real64,
        DataType::VisibleString,
        DataType::OctetString,
        DataType::UnicodeString,
        DataType::TimeOfDay,
        DataType::TimeDifference,
        DataType::Domain,
    ];

    pub fn from_code(code: u16) -> Option<DataType> {
        use DataType::*;
        Some(match code {
            0x01 => Boolean,
            0x02 => Integer8,
            0x03 => Integer16,
            0x04 => Integer32,
            0x05 => Unsigned8,
            0x06 => Unsigned16,
            0x07 => Unsigned32,
            0x08 => Real32,
            0x09 => VisibleString,
            0x0A => OctetString,
            0x0B => UnicodeString,
            0x0C => TimeOfDay,
            0x0D => TimeDifference,
            0x0F => Domain,
            0x10 => Integer24,
            0x11 => Real64,
            0x12 => Integer40,
            0x13 => Integer48,
            0x14 => Integer56,
            0x15 => Integer64,
            0x16 => Unsigned24,
            0x18 => Unsigned40,
            0x19 => Unsigned48,
            0x1A => Unsigned56,
            0x1B => Unsigned64,
            _ => return None,
        })
    }

    pub fn code(self) -> u16 {
        use DataType::*;
        match self {
            Boolean => 0x01,
            Integer8 => 0x02,
            Integer16 => 0x03,
            Integer32 => 0x04,
            Unsigned8 => 0x05,
            Unsigned16 => 0x06,
            Unsigned32 => 0x07,
            Real32 => 0x08,
            VisibleString => 0x09,
            OctetString => 0x0A,
            UnicodeString => 0x0B,
            TimeOfDay => 0x0C,
            TimeDifference => 0x0D,
            Domain => 0x0F,
            Integer24 => 0x10,
            Real64 => 0x11,
            Integer40 => 0x12,
            Integer48 => 0x13,
            Integer56 => 0x14,
            Integer64 => 0x15,
            Unsigned24 => 0x16,
            Unsigned40 => 0x18,
            Unsigned48 => 0x19,
            Unsigned56 => 0x1A,
            Unsigned64 => 0x1B,
        }
    }

    pub fn name(self) -> &'static str {
        use DataType::*;
        match self {
            Boolean => "BOOLEAN",
            Integer8 => "INTEGER8",
            Integer16 => "INTEGER16",
            Integer24 => "INTEGER24",
            Integer32 => "INTEGER32",
            Integer40 => "INTEGER40",
            Integer48 => "INTEGER48",
            Integer56 => "INTEGER56",
            Integer64 => "INTEGER64",
            Unsigned8 => "UNSIGNED8",
            Unsigned16 => "UNSIGNED16",
            Unsigned24 => "UNSIGNED24",
            Unsigned32 => "UNSIGNED32",
            Unsigned40 => "UNSIGNED40",
            Unsigned48 => "UNSIGNED48",
            Unsigned56 => "UNSIGNED56",
            Unsigned64 => "UNSIGNED64",
            Real32 => "REAL32",
            Real64 => "REAL64",
            VisibleString => "VISIBLE_STRING",
            OctetString => "OCTET_STRING",
            UnicodeString => "UNICODE_STRING",
            TimeOfDay => "TIME_OF_DAY",
            TimeDifference => "TIME_DIFFERENCE",
            Domain => "DOMAIN",
        }
    }

    /// Fixed size in bytes; `None` for strings and DOMAIN.
    pub fn size(self) -> Option<usize> {
        use DataType::*;
        Some(match self {
            Boolean | Integer8 | Unsigned8 => 1,
            Integer16 | Unsigned16 => 2,
            Integer24 | Unsigned24 => 3,
            Integer32 | Unsigned32 | Real32 => 4,
            Integer40 | Unsigned40 => 5,
            Integer48 | Unsigned48 | TimeOfDay | TimeDifference => 6,
            Integer56 | Unsigned56 => 7,
            Integer64 | Unsigned64 | Real64 => 8,
            VisibleString | OctetString | UnicodeString | Domain => return None,
        })
    }

    pub fn is_signed(self) -> bool {
        use DataType::*;
        matches!(self, Integer8 | Integer16 | Integer24 | Integer32 | Integer40 | Integer48 | Integer56 | Integer64)
    }

    pub fn is_unsigned(self) -> bool {
        use DataType::*;
        matches!(self, Boolean | Unsigned8 | Unsigned16 | Unsigned24 | Unsigned32 | Unsigned40 | Unsigned48 | Unsigned56 | Unsigned64)
    }

    pub fn is_integer(self) -> bool {
        self.is_signed() || self.is_unsigned()
    }

    pub fn is_float(self) -> bool {
        matches!(self, DataType::Real32 | DataType::Real64)
    }

    pub fn is_string(self) -> bool {
        matches!(self, DataType::VisibleString | DataType::UnicodeString)
    }

    /// Value range of an integer type as (min, max).
    pub fn int_range(self) -> Option<(i128, i128)> {
        if self == DataType::Boolean {
            return Some((0, 1));
        }
        let bits = (self.size()? * 8) as u32;
        if self.is_signed() {
            Some((-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1))
        } else if self.is_unsigned() {
            Some((0, (1i128 << bits) - 1))
        } else {
            None
        }
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Name of a data type code, also for unknown codes.
pub fn type_name(code: u16) -> String {
    match DataType::from_code(code) {
        Some(t) => t.name().to_string(),
        None => format!("0x{code:04X}"),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    UInt(u64),
    F32(f32),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError(pub String);

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(s: impl Into<String>) -> Result<T, TypeError> {
    Err(TypeError(s.into()))
}

/// Little-endian unsigned of up to 8 bytes.
pub fn le_u64(b: &[u8]) -> u64 {
    b.iter().take(8).enumerate().fold(0u64, |acc, (i, &x)| acc | ((x as u64) << (8 * i)))
}

fn sign_extend(v: u64, bits: u32) -> i64 {
    if bits >= 64 {
        return v as i64;
    }
    let shift = 64 - bits;
    ((v << shift) as i64) >> shift
}

/// Bytes -> value. A fixed-size type takes its first `size` bytes; fewer
/// bytes are an error.
pub fn decode(dt: DataType, b: &[u8]) -> Result<Value, TypeError> {
    use DataType::*;
    if let Some(n) = dt.size() {
        if b.len() < n {
            return err(format!("{} needs {} bytes, got {}", dt.name(), n, b.len()));
        }
    }
    Ok(match dt {
        Boolean => Value::Bool(b[0] != 0),
        Real32 => Value::F32(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        Real64 => Value::F64(f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])),
        VisibleString => Value::Str(b.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect()),
        UnicodeString => {
            let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
            Value::Str(String::from_utf16_lossy(&units))
        }
        OctetString | Domain | TimeOfDay | TimeDifference => {
            let n = dt.size().unwrap_or(b.len());
            Value::Bytes(b[..n].to_vec())
        }
        _ => {
            let n = dt.size().unwrap_or(0);
            let raw = le_u64(&b[..n]);
            if dt.is_signed() {
                Value::Int(sign_extend(raw, (n * 8) as u32))
            } else {
                Value::UInt(raw)
            }
        }
    })
}

/// Value -> bytes, with a range check.
pub fn encode(dt: DataType, v: &Value) -> Result<Vec<u8>, TypeError> {
    use DataType::*;
    match (dt, v) {
        (Boolean, Value::Bool(x)) => Ok(vec![*x as u8]),
        (Real32, Value::F32(x)) => Ok(x.to_le_bytes().to_vec()),
        (Real32, Value::F64(x)) => Ok((*x as f32).to_le_bytes().to_vec()),
        (Real64, Value::F64(x)) => Ok(x.to_le_bytes().to_vec()),
        (Real64, Value::F32(x)) => Ok((*x as f64).to_le_bytes().to_vec()),
        (VisibleString, Value::Str(s)) => Ok(s.as_bytes().to_vec()),
        (UnicodeString, Value::Str(s)) => Ok(s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()),
        (OctetString | Domain, Value::Bytes(b)) => Ok(b.clone()),
        (TimeOfDay | TimeDifference, Value::Bytes(b)) => {
            if b.len() == 6 {
                Ok(b.clone())
            } else {
                err(format!("{} needs 6 bytes", dt.name()))
            }
        }
        (_, Value::Int(_) | Value::UInt(_) | Value::Bool(_)) if dt.is_integer() => {
            let x: i128 = match v {
                Value::Int(i) => *i as i128,
                Value::UInt(u) => *u as i128,
                Value::Bool(b) => *b as i128,
                _ => 0,
            };
            int_to_bytes(dt, x)
        }
        _ => err(format!("value does not fit {}", dt.name())),
    }
}

fn int_to_bytes(dt: DataType, x: i128) -> Result<Vec<u8>, TypeError> {
    let (lo, hi) = dt.int_range().ok_or_else(|| TypeError("not an integer type".into()))?;
    if x < lo || x > hi {
        return err(format!("{x} out of range {lo}..{hi} for {}", dt.name()));
    }
    let n = dt.size().unwrap_or(1);
    let bytes = (x as i64 as u64).to_le_bytes();
    Ok(bytes[..n].to_vec())
}

/// Parse an integer: `0x` hex, `-` sign, decimal; with `octal`, a leading
/// `0` means octal (EDS syntax, CiA 306).
pub fn parse_int(s: &str, octal: bool) -> Option<i128> {
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r.trim()),
        None => (false, t.strip_prefix('+').unwrap_or(t).trim()),
    };
    if t.is_empty() {
        return None;
    }
    let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i128::from_str_radix(&h.replace('_', ""), 16).ok()?
    } else if octal && t.len() > 1 && t.starts_with('0') && t.chars().all(|c| ('0'..='7').contains(&c)) {
        i128::from_str_radix(&t[1..], 8).ok()?
    } else {
        t.replace('_', "").parse::<i128>().ok()?
    };
    Some(if neg { -v } else { v })
}

/// Fit a parsed integer into a type; a value above the signed maximum that
/// fits the unsigned width is taken as two's complement (as some EDS write
/// limits of signed objects in hex).
fn fit_int(dt: DataType, x: i128) -> i128 {
    if dt.is_signed() {
        let bits = (dt.size().unwrap_or(8) * 8) as u32;
        let (_, hi) = dt.int_range().unwrap_or((0, 0));
        if x > hi && x < (1i128 << bits) {
            return x - (1i128 << bits);
        }
    }
    x
}

/// Integer -> bytes with the two's complement rule of `fit_int`.
pub fn encode_int(dt: DataType, x: i128) -> Result<Vec<u8>, TypeError> {
    int_to_bytes(dt, fit_int(dt, x))
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, TypeError> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let digits: String = t.chars().filter(|c| !c.is_whitespace() && *c != ',' && *c != '-' && *c != ':').collect();
    if !digits.len().is_multiple_of(2) {
        return err("hex bytes need an even number of digits");
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| TypeError(format!("bad hex byte '{}'", &digits[i..i + 2]))))
        .collect()
}

/// Text -> bytes for a type. Integers: decimal or `0x` hex (`octal` for EDS
/// text). BOOLEAN also takes true/false. OCTET_STRING / DOMAIN / TIME take
/// hex bytes ("01 02 0A").
pub fn parse_text(dt: DataType, text: &str, octal: bool) -> Result<Vec<u8>, TypeError> {
    use DataType::*;
    match dt {
        VisibleString => Ok(text.as_bytes().to_vec()),
        UnicodeString => encode(dt, &Value::Str(text.to_string())),
        OctetString | Domain | TimeOfDay | TimeDifference => {
            let b = parse_hex_bytes(text)?;
            if let Some(n) = dt.size() {
                if b.len() != n {
                    return err(format!("{} needs {} bytes", dt.name(), n));
                }
            }
            Ok(b)
        }
        Real32 | Real64 => {
            let t = text.trim();
            let f: f64 = if t.starts_with("0x") || t.starts_with("0X") {
                // Raw bits in hex.
                let raw = parse_int(t, false).ok_or_else(|| TypeError(format!("bad number '{t}'")))?;
                if dt == Real32 {
                    f32::from_bits(raw as u32) as f64
                } else {
                    f64::from_bits(raw as u64)
                }
            } else {
                t.parse::<f64>().map_err(|_| TypeError(format!("bad number '{t}'")))?
            };
            if dt == Real32 {
                encode(dt, &Value::F32(f as f32))
            } else {
                encode(dt, &Value::F64(f))
            }
        }
        Boolean => {
            let t = text.trim().to_ascii_lowercase();
            match t.as_str() {
                "true" | "on" => Ok(vec![1]),
                "false" | "off" => Ok(vec![0]),
                _ => {
                    let x = parse_int(&t, octal).ok_or_else(|| TypeError(format!("bad boolean '{text}'")))?;
                    int_to_bytes(dt, x)
                }
            }
        }
        _ => {
            let x = parse_int(text, octal).ok_or_else(|| TypeError(format!("bad number '{}'", text.trim())))?;
            let x = if text.trim().starts_with("0x") || text.trim().starts_with("0X") { fit_int(dt, x) } else { x };
            int_to_bytes(dt, x)
        }
    }
}

/// Numeric value of bytes for limit checks.
fn numeric(dt: DataType, b: &[u8]) -> Option<f64> {
    match decode(dt, b).ok()? {
        Value::Int(i) => Some(i as f64),
        Value::UInt(u) => Some(u as f64),
        Value::Bool(x) => Some(x as u8 as f64),
        Value::F32(f) => Some(f as f64),
        Value::F64(f) => Some(f),
        _ => None,
    }
}

fn limit_value(dt: DataType, s: &str) -> Option<f64> {
    if dt.is_float() {
        if let Ok(f) = s.trim().parse::<f64>() {
            return Some(f);
        }
    }
    parse_int(s, true).map(|x| fit_int(dt, x) as f64)
}

/// Check encoded bytes against EDS low/high limits (texts, `$NODEID`
/// already evaluated). Unparsable limits are ignored.
pub fn check_limits(dt: DataType, b: &[u8], low: Option<&str>, high: Option<&str>) -> Result<(), TypeError> {
    let Some(v) = numeric(dt, b) else { return Ok(()) };
    if let Some(lo) = low.and_then(|s| limit_value(dt, s)) {
        if v < lo {
            return err(format!("value {v} below low limit {lo}"));
        }
    }
    if let Some(hi) = high.and_then(|s| limit_value(dt, s)) {
        if v > hi {
            return err(format!("value {v} above high limit {hi}"));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum DisplayMode {
    #[default]
    Hex,
    Decimal,
    String,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 3] = [DisplayMode::Hex, DisplayMode::Decimal, DisplayMode::String];
    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::Hex => "Hex",
            DisplayMode::Decimal => "Decimal",
            DisplayMode::String => "String",
        }
    }
}

/// Bytes as text: printable ASCII, others as '.'.
pub fn ascii(b: &[u8]) -> String {
    b.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '.' }).collect()
}

pub fn hex_bytes(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 3);
    for (i, x) in b.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{x:02X}"));
    }
    s
}

pub fn dec_bytes(b: &[u8]) -> String {
    b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ")
}

/// Raw bytes in a display mode (the trace data column).
pub fn format_bytes(b: &[u8], mode: DisplayMode) -> String {
    match mode {
        DisplayMode::Hex => hex_bytes(b),
        DisplayMode::Decimal => dec_bytes(b),
        DisplayMode::String => ascii(b),
    }
}

/// A value by its type in a display mode. `None` type = raw bytes.
pub fn format_value(dt: Option<DataType>, b: &[u8], mode: DisplayMode) -> String {
    let Some(dt) = dt else { return format_bytes(b, mode) };
    match mode {
        DisplayMode::String => match decode(dt, b) {
            Ok(Value::Str(s)) => s,
            _ => {
                let n = dt.size().unwrap_or(b.len()).min(b.len());
                ascii(&b[..n])
            }
        },
        DisplayMode::Hex => {
            if dt.is_integer() || dt.is_float() {
                let n = dt.size().unwrap_or(1);
                if b.len() < n {
                    return format!("? ({})", hex_bytes(b));
                }
                let raw = le_u64(&b[..n]);
                format!("0x{:0width$X}", raw, width = n * 2)
            } else {
                hex_bytes(b)
            }
        }
        DisplayMode::Decimal => match decode(dt, b) {
            Ok(Value::Bool(x)) => (x as u8).to_string(),
            Ok(Value::Int(i)) => i.to_string(),
            Ok(Value::UInt(u)) => u.to_string(),
            Ok(Value::F32(f)) => f.to_string(),
            Ok(Value::F64(f)) => f.to_string(),
            Ok(Value::Str(_)) | Ok(Value::Bytes(_)) => dec_bytes(b),
            Err(e) => format!("? {e}"),
        },
    }
}
