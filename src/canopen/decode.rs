//! Frame classification by COB-ID (CiA 301 predefined connection set) and
//! decoded text, with object names and PDO signals from assigned EDS files.

use super::nmt::{NmtCommand, NodeState};
use super::pdo::{configs_from_od, extract_bits, PdoConfig, PdoKind};
use super::types::{self, DataType, DisplayMode};
use super::*;
use crate::can::CanFrame;
use crate::eds::ObjectDictionary;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MsgClass {
    Nmt,
    Sync,
    Emcy,
    Time,
    Tpdo,
    Rpdo,
    SdoResp,
    SdoReq,
    Heartbeat,
    Lss,
    Other,
}

impl MsgClass {
    pub const ALL: [MsgClass; 11] = [
        MsgClass::Nmt,
        MsgClass::Sync,
        MsgClass::Emcy,
        MsgClass::Time,
        MsgClass::Tpdo,
        MsgClass::Rpdo,
        MsgClass::SdoReq,
        MsgClass::SdoResp,
        MsgClass::Heartbeat,
        MsgClass::Lss,
        MsgClass::Other,
    ];
    pub fn label(self) -> &'static str {
        match self {
            MsgClass::Nmt => "NMT",
            MsgClass::Sync => "SYNC",
            MsgClass::Emcy => "EMCY",
            MsgClass::Time => "TIME",
            MsgClass::Tpdo => "TPDO",
            MsgClass::Rpdo => "RPDO",
            MsgClass::SdoResp => "SDO resp",
            MsgClass::SdoReq => "SDO req",
            MsgClass::Heartbeat => "Heartbeat",
            MsgClass::Lss => "LSS",
            MsgClass::Other => "Other",
        }
    }
    pub fn bit(self) -> u16 {
        1 << (MsgClass::ALL.iter().position(|&c| c == self).unwrap_or(10) as u16)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub class: MsgClass,
    pub node: Option<u8>,
    /// Short type label ("TPDO1", "SDO resp", "EMCY", ...).
    pub label: String,
    pub text: String,
}

/// Classification by the predefined connection set only.
pub fn classify(id: u32, extended: bool) -> (MsgClass, Option<u8>, String) {
    if extended {
        return (MsgClass::Other, None, "Ext".to_string());
    }
    let node = (id & 0x7F) as u8;
    let n = if node == 0 { None } else { Some(node) };
    match id {
        COB_NMT => (MsgClass::Nmt, None, "NMT".into()),
        COB_SYNC => (MsgClass::Sync, None, "SYNC".into()),
        0x081..=0x0FF => (MsgClass::Emcy, n, "EMCY".into()),
        COB_TIME => (MsgClass::Time, None, "TIME".into()),
        COB_LSS_SLAVE | COB_LSS_MASTER => (MsgClass::Lss, None, "LSS".into()),
        0x181..=0x57F if node != 0 => {
            let f = (id >> 7) as u8; // 3 = TPDO1, 4 = RPDO1, 5 = TPDO2 ...
            if f % 2 == 1 {
                (MsgClass::Tpdo, n, format!("TPDO{}", (f - 1) / 2))
            } else {
                (MsgClass::Rpdo, n, format!("RPDO{}", (f - 2) / 2))
            }
        }
        0x581..=0x5FF => (MsgClass::SdoResp, n, "SDO resp".into()),
        0x601..=0x67F => (MsgClass::SdoReq, n, "SDO req".into()),
        0x701..=0x77F => (MsgClass::Heartbeat, n, "Heartbeat".into()),
        _ => (MsgClass::Other, None, "".into()),
    }
}

/// Assigned dictionaries per node and the PDO lookup by COB-ID.
#[derive(Clone, Default)]
pub struct DecodeContext {
    pub nodes: HashMap<u8, Arc<ObjectDictionary>>,
    pdos: HashMap<u32, (u8, PdoConfig)>,
}

impl DecodeContext {
    pub fn new(nodes: HashMap<u8, Arc<ObjectDictionary>>) -> Self {
        let mut pdos: HashMap<u32, (u8, PdoConfig)> = HashMap::new();
        let mut ids: Vec<u8> = nodes.keys().copied().collect();
        ids.sort_unstable();
        for node in ids {
            for c in configs_from_od(&nodes[&node], node) {
                if c.extended || c.mapping.is_empty() {
                    continue;
                }
                match pdos.get(&c.cob_id) {
                    Some((_, old)) if old.valid || !c.valid => {}
                    _ => {
                        pdos.insert(c.cob_id, (node, c));
                    }
                }
            }
        }
        DecodeContext { nodes, pdos }
    }

    pub fn pdo(&self, cob: u32) -> Option<&(u8, PdoConfig)> {
        self.pdos.get(&cob)
    }

    fn name_of(&self, node: Option<u8>, index: u16, sub: u8) -> Option<String> {
        self.nodes.get(&node?)?.name_of(index, sub)
    }

    fn dtype_of(&self, node: Option<u8>, index: u16, sub: u8) -> Option<DataType> {
        self.nodes.get(&node?)?.entry(index, sub)?.dtype()
    }
}

pub fn decode(f: &CanFrame, ctx: &DecodeContext) -> Decoded {
    if !f.extended {
        if let Some((node, cfg)) = ctx.pdo(f.id) {
            let class = if cfg.kind == PdoKind::Tpdo { MsgClass::Tpdo } else { MsgClass::Rpdo };
            return Decoded { class, node: Some(*node), label: cfg.label(), text: pdo_text(f, *node, cfg, ctx) };
        }
    }
    let (class, node, label) = classify(f.id, f.extended);
    let d = f.payload();
    let text = match class {
        MsgClass::Nmt => nmt_text(d),
        MsgClass::Sync => match d.len() {
            0 => "SYNC".to_string(),
            _ => format!("SYNC counter {}", d[0]),
        },
        MsgClass::Emcy => emcy_text(d),
        MsgClass::Time => time_text(d),
        MsgClass::Tpdo | MsgClass::Rpdo => {
            if f.rtr {
                format!("{label} RTR")
            } else {
                format!("{label} node {}", node.unwrap_or(0))
            }
        }
        MsgClass::SdoResp | MsgClass::SdoReq => sdo_text(f, class == MsgClass::SdoReq, node, ctx),
        MsgClass::Heartbeat => {
            if f.rtr {
                "Node guarding request".to_string()
            } else if d.is_empty() {
                "Heartbeat (no data)".to_string()
            } else {
                let st = NodeState::from_byte(d[0]);
                let mut s = if st == NodeState::BootUp { "Boot-up".to_string() } else { format!("Heartbeat {}", st.label()) };
                if d[0] & 0x80 != 0 {
                    s.push_str(" (toggle 1)");
                }
                s
            }
        }
        MsgClass::Lss => {
            let who = if f.id == COB_LSS_MASTER { "LSS master request" } else { "LSS slave response" };
            match d.first() {
                Some(cs) => format!("{who} cs 0x{cs:02X}"),
                None => who.to_string(),
            }
        }
        MsgClass::Other => String::new(),
    };
    Decoded { class, node, label, text }
}

fn nmt_text(d: &[u8]) -> String {
    if d.len() < 2 {
        return "NMT (short frame)".to_string();
    }
    let target = if d[1] == 0 { "all nodes".to_string() } else { format!("node {}", d[1]) };
    match NmtCommand::from_code(d[0]) {
        Some(c) => format!("NMT {} {}", c.label(), target),
        None => format!("NMT command 0x{:02X} {}", d[0], target),
    }
}

/// Text for an EMCY error code (standard classes of CiA 301).
pub fn emcy_code_text(code: u16) -> &'static str {
    match code {
        0x0000 => "Error reset or no error",
        0x8110 => "CAN overrun (objects lost)",
        0x8120 => "CAN in error passive mode",
        0x8130 => "Life guard error or heartbeat error",
        0x8140 => "Recovered from bus off",
        0x8150 => "CAN-ID collision",
        0x8210 => "PDO not processed due to length error",
        0x8220 => "PDO length exceeded",
        0x8230 => "DAM MPDO not processed, destination object not available",
        0x8240 => "Unexpected SYNC data length",
        0x8250 => "RPDO timeout",
        _ => match code >> 8 {
            0x00 => "Error reset or no error",
            0x10 => "Generic error",
            0x20 => "Current",
            0x21 => "Current, device input side",
            0x22 => "Current inside the device",
            0x23 => "Current, device output side",
            0x30 => "Voltage",
            0x31 => "Mains voltage",
            0x32 => "Voltage inside the device",
            0x33 => "Output voltage",
            0x40 => "Temperature",
            0x41 => "Ambient temperature",
            0x42 => "Device temperature",
            0x50 => "Device hardware",
            0x60 => "Device software",
            0x61 => "Internal software",
            0x62 => "User software",
            0x63 => "Data set",
            0x70 => "Additional modules",
            0x80 => "Monitoring",
            0x81 => "Communication",
            0x82 => "Protocol error",
            0x90 => "External error",
            0xF0 => "Additional functions",
            0xFF => "Device specific",
            _ => match code >> 12 {
                0x1 => "Generic error",
                0x2 => "Current",
                0x3 => "Voltage",
                0x4 => "Temperature",
                0x5 => "Device hardware",
                0x6 => "Device software",
                0x7 => "Additional modules",
                0x8 => "Monitoring",
                0x9 => "External error",
                0xF => "Additional functions",
                _ => "Unknown error code",
            },
        },
    }
}

/// Names of the set bits of the error register (0x1001).
pub fn error_register_text(reg: u8) -> String {
    let names = ["generic", "current", "voltage", "temperature", "communication", "profile", "reserved", "manufacturer"];
    let v: Vec<&str> = (0..8).filter(|i| reg & (1 << i) != 0).map(|i| names[i]).collect();
    v.join("+")
}

fn emcy_text(d: &[u8]) -> String {
    if d.len() < 3 {
        return format!("EMCY (short frame) {}", types::hex_bytes(d));
    }
    let code = u16::from_le_bytes([d[0], d[1]]);
    let reg = d[2];
    let mut s = format!("EMCY 0x{code:04X} reg 0x{reg:02X} {}", emcy_code_text(code));
    let r = error_register_text(reg);
    if !r.is_empty() {
        s.push_str(&format!(" [{r}]"));
    }
    if d.len() > 3 {
        s.push_str(&format!(" mfr {}", types::hex_bytes(&d[3..])));
    }
    s
}

fn time_text(d: &[u8]) -> String {
    if d.len() < 6 {
        return "TIME (short frame)".to_string();
    }
    let ms = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) & 0x0FFF_FFFF;
    let days = u16::from_le_bytes([d[4], d[5]]);
    let (h, m, s, milli) = (ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000);
    format!("TIME {h:02}:{m:02}:{s:02}.{milli:03} day {days} (since 1984-01-01)")
}

fn obj_ref(index: u16, sub: u8) -> String {
    format!("0x{index:04X}:{sub:02X}")
}

fn with_name(s: String, ctx: &DecodeContext, node: Option<u8>, index: u16, sub: u8) -> String {
    match ctx.name_of(node, index, sub) {
        Some(n) => format!("{s} ({n})"),
        None => s,
    }
}

fn expedited_value(ctx: &DecodeContext, node: Option<u8>, index: u16, sub: u8, data: &[u8]) -> String {
    match ctx.dtype_of(node, index, sub) {
        Some(dt) if dt.is_string() => format!("\"{}\"", types::format_value(Some(dt), data, DisplayMode::String)),
        Some(dt) if (dt.is_integer() || dt.is_float()) && dt.size().is_some_and(|n| n <= data.len()) => {
            types::format_value(Some(dt), data, DisplayMode::Hex)
        }
        _ => {
            let raw = types::le_u64(data);
            format!("0x{:0w$X}", raw, w = data.len() * 2)
        }
    }
}

fn sdo_text(f: &CanFrame, request: bool, node: Option<u8>, ctx: &DecodeContext) -> String {
    let d = f.data;
    if f.dlc < 1 {
        return "SDO (empty frame)".to_string();
    }
    let cs = d[0] >> 5;
    let index = u16::from_le_bytes([d[1], d[2]]);
    let sub = d[3];
    let r = obj_ref(index, sub);
    if cs == 4 {
        let code = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
        let who = if request { "client" } else { "server" };
        return with_name(format!("SDO abort by {who} {r} 0x{code:08X} {}", abort_text(code)), ctx, node, index, sub);
    }
    let seg_data = |d: &[u8; 8]| {
        let n = ((d[0] >> 1) & 0x07) as usize;
        let last = d[0] & 0x01 != 0;
        let bytes = &d[1..8 - n];
        (bytes.to_vec(), last, (d[0] >> 4) & 1)
    };
    if request {
        match cs {
            1 => {
                let e = d[0] & 0x02 != 0;
                let s = d[0] & 0x01 != 0;
                if e {
                    let len = if s { 4 - ((d[0] >> 2) & 3) as usize } else { 4 };
                    let v = expedited_value(ctx, node, index, sub, &d[4..4 + len]);
                    with_name(format!("SDO download req {r} = {v} ({len} bytes)"), ctx, node, index, sub)
                } else {
                    let size = if s { format!(", size {}", u32::from_le_bytes([d[4], d[5], d[6], d[7]])) } else { String::new() };
                    with_name(format!("SDO download req {r} segmented{size}"), ctx, node, index, sub)
                }
            }
            0 => {
                let (b, last, t) = seg_data(&d);
                format!("SDO download segment t={t} {} bytes \"{}\"{}", b.len(), types::ascii(&b), if last { " last" } else { "" })
            }
            2 => with_name(format!("SDO upload req {r}"), ctx, node, index, sub),
            3 => format!("SDO upload segment req t={}", (d[0] >> 4) & 1),
            5 => "SDO block upload (not decoded)".to_string(),
            6 => "SDO block download (not decoded)".to_string(),
            _ => format!("SDO request cs {cs}"),
        }
    } else {
        match cs {
            2 => {
                let e = d[0] & 0x02 != 0;
                let s = d[0] & 0x01 != 0;
                if e {
                    let len = if s { 4 - ((d[0] >> 2) & 3) as usize } else { 4 };
                    let v = expedited_value(ctx, node, index, sub, &d[4..4 + len]);
                    with_name(format!("SDO upload resp {r} = {v}"), ctx, node, index, sub)
                } else {
                    let size = if s { format!(", size {}", u32::from_le_bytes([d[4], d[5], d[6], d[7]])) } else { String::new() };
                    with_name(format!("SDO upload resp {r} segmented{size}"), ctx, node, index, sub)
                }
            }
            0 => {
                let (b, last, t) = seg_data(&d);
                format!("SDO upload segment t={t} {} bytes \"{}\"{}", b.len(), types::ascii(&b), if last { " last" } else { "" })
            }
            3 => with_name(format!("SDO download resp {r} OK"), ctx, node, index, sub),
            1 => format!("SDO download segment resp t={}", (d[0] >> 4) & 1),
            5 | 6 => "SDO block transfer (not decoded)".to_string(),
            _ => format!("SDO response cs {cs}"),
        }
    }
}

fn pdo_text(f: &CanFrame, node: u8, cfg: &PdoConfig, ctx: &DecodeContext) -> String {
    if f.rtr {
        return format!("{} RTR", cfg.label());
    }
    let od = ctx.nodes.get(&node);
    let data = f.payload();
    let mut parts: Vec<String> = Vec::new();
    let mut offset = 0u32;
    let mut short = false;
    for m in &cfg.mapping {
        let bits = m.bits as u32;
        if m.is_dummy() {
            offset += bits;
            continue;
        }
        let name = od
            .and_then(|o| o.entry(m.index, m.sub).map(|_| o.name_of(m.index, m.sub).unwrap_or_default()))
            .unwrap_or_else(|| obj_ref(m.index, m.sub));
        let Some(raw) = extract_bits(data, offset, bits) else {
            short = true;
            break;
        };
        offset += bits;
        let dt = od.and_then(|o| o.entry(m.index, m.sub)).and_then(|e| e.dtype());
        parts.push(format!("{name} = {}", signal_value(dt, raw, bits)));
    }
    let mut s = parts.join(", ");
    if short {
        if !s.is_empty() {
            s.push_str(", ");
        }
        s.push_str("(frame shorter than mapping)");
    }
    s
}

fn signal_value(dt: Option<DataType>, raw: u64, bits: u32) -> String {
    match dt {
        Some(DataType::Boolean) => (raw != 0).to_string(),
        Some(DataType::Real32) if bits == 32 => f32::from_bits(raw as u32).to_string(),
        Some(DataType::Real64) if bits == 64 => f64::from_bits(raw).to_string(),
        Some(t) if t.is_signed() && bits > 0 && bits < 64 => {
            let shift = 64 - bits;
            (((raw << shift) as i64) >> shift).to_string()
        }
        Some(t) if t.is_signed() => (raw as i64).to_string(),
        Some(t) if t.is_string() || t == DataType::OctetString => {
            let bytes: Vec<u8> = raw.to_le_bytes()[..(bits as usize).div_ceil(8)].to_vec();
            format!("\"{}\"", types::ascii(&bytes))
        }
        _ => raw.to_string(),
    }
}
