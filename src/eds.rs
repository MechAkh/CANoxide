//! Tolerant parser for CiA 306 EDS and DCF files.
//!
//! INI syntax, case-insensitive sections and keys, `;` comments, UTF-8 or
//! Windows-1252 text. The parser never fails: problems become warnings.

use crate::canopen::types::{self, DataType};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectType {
    Null,
    Domain,
    DefType,
    DefStruct,
    Var,
    Array,
    Record,
}

impl ObjectType {
    pub fn from_code(c: i128) -> Option<ObjectType> {
        Some(match c {
            0 => ObjectType::Null,
            2 => ObjectType::Domain,
            5 => ObjectType::DefType,
            6 => ObjectType::DefStruct,
            7 => ObjectType::Var,
            8 => ObjectType::Array,
            9 => ObjectType::Record,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            ObjectType::Null => "NULL",
            ObjectType::Domain => "DOMAIN",
            ObjectType::DefType => "DEFTYPE",
            ObjectType::DefStruct => "DEFSTRUCT",
            ObjectType::Var => "VAR",
            ObjectType::Array => "ARRAY",
            ObjectType::Record => "RECORD",
        }
    }
    /// True when the object has sub-objects of its own sections.
    pub fn has_subs(self) -> bool {
        matches!(self, ObjectType::Array | ObjectType::Record | ObjectType::DefStruct)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessType {
    Ro,
    Wo,
    Rw,
    Rwr,
    Rww,
    Const,
}

impl AccessType {
    pub fn parse(s: &str) -> Option<AccessType> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "ro" => AccessType::Ro,
            "wo" => AccessType::Wo,
            "rw" => AccessType::Rw,
            "rwr" => AccessType::Rwr,
            "rww" => AccessType::Rww,
            "const" => AccessType::Const,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            AccessType::Ro => "ro",
            AccessType::Wo => "wo",
            AccessType::Rw => "rw",
            AccessType::Rwr => "rwr",
            AccessType::Rww => "rww",
            AccessType::Const => "const",
        }
    }
    pub fn readable(self) -> bool {
        self != AccessType::Wo
    }
    pub fn writable(self) -> bool {
        !matches!(self, AccessType::Ro | AccessType::Const)
    }
}

/// One value entry: a VAR object (sub 0) or a sub-object.
#[derive(Clone, Debug, PartialEq)]
pub struct OdEntry {
    pub sub: u8,
    pub name: String,
    pub data_type: u16,
    pub access: AccessType,
    pub default: Option<String>,
    /// DCF ParameterValue.
    pub param_value: Option<String>,
    pub low: Option<String>,
    pub high: Option<String>,
    pub pdo_mapping: bool,
}

impl OdEntry {
    pub fn dtype(&self) -> Option<DataType> {
        DataType::from_code(self.data_type)
    }

    /// ParameterValue (DCF) before DefaultValue.
    pub fn effective_text(&self) -> Option<&str> {
        self.param_value.as_deref().or(self.default.as_deref())
    }

    /// Encode the effective value with `$NODEID` evaluated.
    pub fn effective_bytes(&self, node_id: u8) -> Option<Vec<u8>> {
        let dt = self.dtype()?;
        let text = self.effective_text().unwrap_or("");
        value_bytes(dt, text, node_id)
    }
}

/// Encode an EDS value text for a type (`$NODEID` evaluated for numbers).
pub fn value_bytes(dt: DataType, text: &str, node_id: u8) -> Option<Vec<u8>> {
    match dt {
        DataType::VisibleString | DataType::UnicodeString => types::parse_text(dt, text, true).ok(),
        DataType::OctetString | DataType::Domain if text.trim().is_empty() => Some(Vec::new()),
        _ if text.trim().is_empty() => dt.size().map(|n| vec![0u8; n]),
        _ if dt.is_integer() => types::encode_int(dt, eval_int(text, node_id)?).ok(),
        _ => types::parse_text(dt, text, true).ok(),
    }
}

/// Evaluate an integer text with `$NODEID` terms ("$NODEID+0x180").
pub fn eval_int(text: &str, node_id: u8) -> Option<i128> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if !lower.contains("$nodeid") {
        return types::parse_int(&t, true);
    }
    let mut sum: i128 = 0;
    let mut term = String::new();
    let mut sign: i128 = 1;
    let flush = |term: &str, sign: i128, sum: &mut i128| -> Option<()> {
        if term.is_empty() {
            return Some(());
        }
        let v = if term.eq_ignore_ascii_case("$nodeid") { node_id as i128 } else { types::parse_int(term, true)? };
        *sum += sign * v;
        Some(())
    };
    for c in t.chars() {
        if c == '+' || c == '-' {
            flush(&term, sign, &mut sum)?;
            term.clear();
            sign = if c == '-' { -1 } else { 1 };
        } else {
            term.push(c);
        }
    }
    flush(&term, sign, &mut sum)?;
    Some(sum)
}

#[derive(Clone, Debug, PartialEq)]
pub struct OdObject {
    pub index: u16,
    pub name: String,
    pub object_type: ObjectType,
    /// VAR: one entry at sub 0. ARRAY/RECORD: the sub-objects.
    pub subs: BTreeMap<u8, OdEntry>,
}

impl OdObject {
    pub fn is_var(&self) -> bool {
        !self.object_type.has_subs()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectDictionary {
    pub file_info: Vec<(String, String)>,
    pub device_info: Vec<(String, String)>,
    pub objects: BTreeMap<u16, OdObject>,
    pub warnings: Vec<String>,
    /// [DeviceComissioning] NodeID of a DCF.
    pub node_id: Option<u8>,
    pub baudrate_kbit: Option<u32>,
    pub is_dcf: bool,
}

impl ObjectDictionary {
    pub fn entry(&self, index: u16, sub: u8) -> Option<&OdEntry> {
        self.objects.get(&index)?.subs.get(&sub)
    }

    /// "Object name" for a VAR, "Object name / Sub name" for a sub-object.
    pub fn name_of(&self, index: u16, sub: u8) -> Option<String> {
        let o = self.objects.get(&index)?;
        if o.is_var() {
            return Some(o.name.clone());
        }
        match o.subs.get(&sub) {
            Some(e) => Some(format!("{} / {}", o.name, e.name)),
            None => Some(o.name.clone()),
        }
    }

    pub fn file_info_value(&self, key: &str) -> Option<&str> {
        self.file_info.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn device_info_value(&self, key: &str) -> Option<&str> {
        self.device_info.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn entry_count(&self) -> usize {
        self.objects.values().map(|o| o.subs.len()).sum()
    }
}

/// Read a file (UTF-8 with or without BOM, else Windows-1252).
pub fn load(path: &Path) -> std::io::Result<ObjectDictionary> {
    let bytes = std::fs::read(path)?;
    let mut od = parse_bytes(&bytes);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "dcf" || ext == "xdc" {
        od.is_dcf = true;
    }
    Ok(od)
}

pub fn decode_text(bytes: &[u8]) -> String {
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode_without_bom_handling(b).0.into_owned(),
    }
}

pub fn parse_bytes(bytes: &[u8]) -> ObjectDictionary {
    parse_str(&decode_text(bytes))
}

struct Section {
    name: String,
    /// (lower-case key, original key, value, line)
    entries: Vec<(String, String, String, usize)>,
}

impl Section {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().rev().find(|e| e.0 == key).map(|e| e.2.as_str())
    }
}

const KNOWN_OBJECT_KEYS: [&str; 18] = [
    "parametername",
    "objecttype",
    "datatype",
    "accesstype",
    "defaultvalue",
    "lowlimit",
    "highlimit",
    "pdomapping",
    "subnumber",
    "objflags",
    "compactsubobj",
    "parametervalue",
    "denotation",
    "uploadfile",
    "downloadfile",
    "parameterrefrence",
    "parameterreference",
    "srdomapping",
];

enum SecKind {
    Object(u16),
    Sub(u16, u8),
    Names(u16),
    Values(u16),
    Other,
}

fn section_kind(name: &str) -> SecKind {
    let up = name.to_ascii_uppercase();
    let is_hex4 = |s: &str| s.len() == 4 && s.chars().all(|c| c.is_ascii_hexdigit());
    if is_hex4(&up) {
        return SecKind::Object(u16::from_str_radix(&up, 16).unwrap_or(0));
    }
    if up.len() > 7 && is_hex4(&up[..4]) && up[4..].starts_with("SUB") {
        let sub = &up[7..];
        if !sub.is_empty() && sub.len() <= 2 && sub.chars().all(|c| c.is_ascii_hexdigit()) {
            return SecKind::Sub(u16::from_str_radix(&up[..4], 16).unwrap_or(0), u8::from_str_radix(sub, 16).unwrap_or(0));
        }
    }
    if up.len() == 8 && is_hex4(&up[..4]) && &up[4..] == "NAME" {
        return SecKind::Names(u16::from_str_radix(&up[..4], 16).unwrap_or(0));
    }
    if up.len() == 9 && is_hex4(&up[..4]) && &up[4..] == "VALUE" {
        return SecKind::Values(u16::from_str_radix(&up[..4], 16).unwrap_or(0));
    }
    SecKind::Other
}

fn opt(s: Option<&str>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn parse_str(text: &str) -> ObjectDictionary {
    let mut od = ObjectDictionary::default();
    let mut sections: Vec<Section> = Vec::new();
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    let mut current: Option<usize> = None;

    for (i, raw) in text.lines().enumerate() {
        let ln = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let Some(end) = line.find(']') else {
                od.warnings.push(format!("line {ln}: section header without ']'"));
                current = None;
                continue;
            };
            let name = line[1..end].trim().to_string();
            let key = name.to_ascii_uppercase();
            if let Some(&idx) = by_name.get(&key) {
                od.warnings.push(format!("line {ln}: section [{name}] appears again; entries are merged"));
                current = Some(idx);
            } else {
                sections.push(Section { name, entries: Vec::new() });
                by_name.insert(key, sections.len() - 1);
                current = Some(sections.len() - 1);
            }
            continue;
        }
        let Some(eq) = line.find('=') else {
            od.warnings.push(format!("line {ln}: no '=' in \"{}\"", truncate(line, 40)));
            continue;
        };
        let key = line[..eq].trim();
        let value = line[eq + 1..].trim();
        match current {
            Some(idx) => sections[idx].entries.push((key.to_ascii_lowercase(), key.to_string(), value.to_string(), ln)),
            None => od.warnings.push(format!("line {ln}: key '{key}' outside a section")),
        }
    }

    // Info sections.
    for s in &sections {
        match s.name.to_ascii_uppercase().as_str() {
            "FILEINFO" => od.file_info = s.entries.iter().map(|e| (e.1.clone(), e.2.clone())).collect(),
            "DEVICEINFO" => od.device_info = s.entries.iter().map(|e| (e.1.clone(), e.2.clone())).collect(),
            "DEVICECOMISSIONING" | "DEVICECOMMISSIONING" => {
                od.is_dcf = true;
                if let Some(v) = s.get("nodeid") {
                    match types::parse_int(v, true) {
                        Some(n) if (1..=127).contains(&n) => od.node_id = Some(n as u8),
                        _ => od.warnings.push(format!("[{}]: invalid NodeID '{v}'", s.name)),
                    }
                }
                if let Some(v) = s.get("baudrate") {
                    od.baudrate_kbit = types::parse_int(v, true).map(|x| x as u32);
                }
            }
            _ => {}
        }
    }

    // Objects first, then subs.
    let mut names: BTreeMap<u16, &Section> = BTreeMap::new();
    let mut values: BTreeMap<u16, &Section> = BTreeMap::new();
    let mut compact: BTreeMap<u16, (u8, OdEntry)> = BTreeMap::new();
    let mut sub_numbers: BTreeMap<u16, i128> = BTreeMap::new();
    for s in &sections {
        match section_kind(&s.name) {
            SecKind::Object(index) => {
                check_keys(s, &mut od.warnings);
                let name = opt(s.get("parametername")).unwrap_or_else(|| {
                    od.warnings.push(format!("[{}]: ParameterName missing", s.name));
                    format!("Object {index:04X}")
                });
                let ot = match s.get("objecttype") {
                    None => ObjectType::Var,
                    Some(v) => match types::parse_int(v, true).and_then(ObjectType::from_code) {
                        Some(t) => t,
                        None => {
                            od.warnings.push(format!("[{}]: unknown ObjectType '{v}', taken as VAR", s.name));
                            ObjectType::Var
                        }
                    },
                };
                let mut obj = OdObject { index, name: name.clone(), object_type: ot, subs: BTreeMap::new() };
                if !ot.has_subs() {
                    obj.subs.insert(0, entry_from(s, 0, name.clone(), &mut od.warnings));
                } else {
                    if let Some(v) = s.get("subnumber") {
                        if let Some(n) = types::parse_int(v, true) {
                            sub_numbers.insert(index, n);
                        }
                    }
                    if let Some(v) = s.get("compactsubobj") {
                        match types::parse_int(v, true) {
                            Some(n) if (1..=254).contains(&n) => {
                                compact.insert(index, (n as u8, entry_from(s, 1, String::new(), &mut od.warnings)));
                            }
                            Some(0) => {}
                            _ => od.warnings.push(format!("[{}]: invalid CompactSubObj '{v}'", s.name)),
                        }
                    }
                }
                od.objects.insert(index, obj);
            }
            SecKind::Names(index) => {
                names.insert(index, s);
            }
            SecKind::Values(index) => {
                values.insert(index, s);
            }
            _ => {}
        }
    }
    for s in &sections {
        if let SecKind::Sub(index, sub) = section_kind(&s.name) {
            check_keys(s, &mut od.warnings);
            let name = opt(s.get("parametername")).unwrap_or_else(|| format!("Sub {sub}"));
            let e = entry_from(s, sub, name, &mut od.warnings);
            match od.objects.get_mut(&index) {
                Some(o) => {
                    if !o.object_type.has_subs() {
                        od.warnings.push(format!("[{}]: sub-object of a VAR object", s.name));
                    }
                    o.subs.insert(sub, e);
                }
                None => od.warnings.push(format!("[{}]: no section [{index:04X}] for this sub-object", s.name)),
            }
        }
    }

    // CompactSubObj expansion.
    for (index, (n, tmpl)) in compact {
        let Some(o) = od.objects.get_mut(&index) else { continue };
        o.subs.entry(0).or_insert_with(|| OdEntry {
            sub: 0,
            name: "Number of entries".to_string(),
            data_type: DataType::Unsigned8.code(),
            access: AccessType::Ro,
            default: Some(n.to_string()),
            param_value: None,
            low: None,
            high: None,
            pdo_mapping: false,
        });
        let nsec = names.get(&index);
        let vsec = values.get(&index);
        for sub in 1..=n {
            let key = sub.to_string();
            let name = nsec.and_then(|s| s.get(&key)).map(|s| s.to_string()).unwrap_or_else(|| format!("{}{}", o.name, sub));
            let mut e = tmpl.clone();
            e.sub = sub;
            e.name = name;
            e.param_value = vsec.and_then(|s| s.get(&key)).map(|s| s.to_string());
            o.subs.entry(sub).or_insert(e);
        }
    }

    // Checks of the object lists and SubNumber.
    for s in &sections {
        let up = s.name.to_ascii_uppercase();
        if up == "MANDATORYOBJECTS" || up == "OPTIONALOBJECTS" || up == "MANUFACTUREROBJECTS" {
            let count = s.get("supportedobjects").and_then(|v| types::parse_int(v, true));
            let listed: Vec<&(String, String, String, usize)> = s.entries.iter().filter(|e| e.0 != "supportedobjects").collect();
            if let Some(c) = count {
                if c as usize != listed.len() {
                    od.warnings.push(format!("[{}]: SupportedObjects={} but {} entries listed", s.name, c, listed.len()));
                }
            }
            for e in listed {
                match types::parse_int(&e.2, true) {
                    Some(ix) if (0..=0xFFFF).contains(&ix) => {
                        if !od.objects.contains_key(&(ix as u16)) {
                            od.warnings.push(format!("[{}]: object 0x{:04X} listed but has no section", s.name, ix));
                        }
                    }
                    _ => od.warnings.push(format!("line {}: bad object index '{}'", e.3, e.2)),
                }
            }
        }
    }
    for (index, n) in sub_numbers {
        if let Some(o) = od.objects.get(&index) {
            if o.subs.len() as i128 != n {
                od.warnings.push(format!("[{index:04X}]: SubNumber={n} but {} sub-objects found", o.subs.len()));
            }
        }
    }
    if od.objects.is_empty() {
        od.warnings.push("no objects found".to_string());
    }
    od
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}...")
    }
}

fn check_keys(s: &Section, warnings: &mut Vec<String>) {
    for e in &s.entries {
        if !KNOWN_OBJECT_KEYS.contains(&e.0.as_str()) {
            warnings.push(format!("line {}: [{}] unknown key '{}' ignored", e.3, s.name, e.1));
        }
    }
}

fn entry_from(s: &Section, sub: u8, name: String, warnings: &mut Vec<String>) -> OdEntry {
    let data_type = match s.get("datatype") {
        Some(v) => match types::parse_int(v, true) {
            Some(x) if (0..=0xFFFF).contains(&x) => {
                if DataType::from_code(x as u16).is_none() {
                    warnings.push(format!("[{}]: data type 0x{:04X} not supported, shown as raw bytes", s.name, x));
                }
                x as u16
            }
            _ => {
                warnings.push(format!("[{}]: bad DataType '{v}'", s.name));
                DataType::Domain.code()
            }
        },
        None => {
            warnings.push(format!("[{}]: DataType missing, taken as DOMAIN", s.name));
            DataType::Domain.code()
        }
    };
    let access = match s.get("accesstype") {
        Some(v) => AccessType::parse(v).unwrap_or_else(|| {
            warnings.push(format!("[{}]: unknown AccessType '{v}', taken as rw", s.name));
            AccessType::Rw
        }),
        None => {
            warnings.push(format!("[{}]: AccessType missing, taken as rw", s.name));
            AccessType::Rw
        }
    };
    let pdo_mapping = match s.get("pdomapping") {
        Some(v) => match types::parse_int(v, true) {
            Some(x) => x != 0,
            None => {
                warnings.push(format!("[{}]: bad PDOMapping '{v}'", s.name));
                false
            }
        },
        None => false,
    };
    OdEntry {
        sub,
        name,
        data_type,
        access,
        default: s.get("defaultvalue").map(|v| v.to_string()),
        param_value: s.get("parametervalue").map(|v| v.to_string()),
        low: opt(s.get("lowlimit")),
        high: opt(s.get("highlimit")),
        pdo_mapping,
    }
}
