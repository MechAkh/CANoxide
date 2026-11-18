//! PDO communication and mapping parameters, and bit packing.

use crate::eds::{eval_int, ObjectDictionary};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PdoKind {
    /// Sent by the node (0x1800 / 0x1A00).
    Tpdo,
    /// Received by the node (0x1400 / 0x1600).
    Rpdo,
}

impl PdoKind {
    pub fn label(self) -> &'static str {
        match self {
            PdoKind::Tpdo => "TPDO",
            PdoKind::Rpdo => "RPDO",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapEntry {
    pub index: u16,
    pub sub: u8,
    pub bits: u8,
}

impl MapEntry {
    pub fn from_u32(v: u32) -> MapEntry {
        MapEntry { index: (v >> 16) as u16, sub: (v >> 8) as u8, bits: v as u8 }
    }
    pub fn to_u32(self) -> u32 {
        ((self.index as u32) << 16) | ((self.sub as u32) << 8) | self.bits as u32
    }
    /// Dummy mapping entries (data type indices 0x0001..0x0007) are padding.
    pub fn is_dummy(self) -> bool {
        (0x0001..=0x0007).contains(&self.index)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdoConfig {
    pub kind: PdoKind,
    /// 1-based PDO number.
    pub number: u16,
    /// 11-bit or 29-bit identifier (flag bits removed).
    pub cob_id: u32,
    /// Bit 31 of the COB-ID entry clear.
    pub valid: bool,
    pub extended: bool,
    pub trans_type: u8,
    pub event_timer_ms: u16,
    pub mapping: Vec<MapEntry>,
}

impl PdoConfig {
    pub fn label(&self) -> String {
        format!("{}{}", self.kind.label(), self.number)
    }
    pub fn total_bits(&self) -> u32 {
        self.mapping.iter().map(|m| m.bits as u32).sum()
    }
}

fn read_u32(od: &ObjectDictionary, index: u16, sub: u8, node_id: u8) -> Option<u32> {
    let e = od.entry(index, sub)?;
    eval_int(e.effective_text()?, node_id).map(|v| v as u32)
}

/// PDO configurations from the defaults (or DCF values) of a dictionary.
pub fn configs_from_od(od: &ObjectDictionary, node_id: u8) -> Vec<PdoConfig> {
    configs_with(|index, sub| read_u32(od, index, sub, node_id), |index| od.objects.contains_key(&index))
}

/// PDO configurations from any value source (the simulator uses its live
/// values). `get(index, sub)` gives a value; `exists(index)` an object.
pub fn configs_with(get: impl Fn(u16, u8) -> Option<u32>, exists: impl Fn(u16) -> bool) -> Vec<PdoConfig> {
    let mut v = Vec::new();
    for (kind, comm_base, map_base) in [(PdoKind::Rpdo, 0x1400u16, 0x1600u16), (PdoKind::Tpdo, 0x1800, 0x1A00)] {
        for n in 0..512u16 {
            let comm = comm_base + n;
            if !exists(comm) {
                continue;
            }
            let Some(raw) = get(comm, 1) else { continue };
            let count = get(map_base + n, 0).unwrap_or(0).min(64) as u8;
            let mapping = (1..=count).filter_map(|s| get(map_base + n, s)).map(MapEntry::from_u32).collect();
            let extended = raw & 0x2000_0000 != 0;
            v.push(PdoConfig {
                kind,
                number: n + 1,
                cob_id: raw & if extended { 0x1FFF_FFFF } else { 0x7FF },
                valid: raw & 0x8000_0000 == 0,
                extended,
                trans_type: get(comm, 2).unwrap_or(255) as u8,
                event_timer_ms: get(comm, 5).unwrap_or(0) as u16,
                mapping,
            });
        }
    }
    v
}

/// Read `bits` bits starting at bit `offset` (little endian, LSB first).
pub fn extract_bits(data: &[u8], offset: u32, bits: u32) -> Option<u64> {
    if bits == 0 || bits > 64 || offset + bits > (data.len() as u32) * 8 {
        return None;
    }
    let mut v: u64 = 0;
    for i in 0..bits {
        let bit = offset + i;
        let b = (data[(bit / 8) as usize] >> (bit % 8)) & 1;
        v |= (b as u64) << i;
    }
    Some(v)
}

/// Write `bits` bits of `value` at bit `offset`. Returns false when it does not fit.
pub fn insert_bits(data: &mut [u8], offset: u32, bits: u32, value: u64) -> bool {
    if bits == 0 || bits > 64 || offset + bits > (data.len() as u32) * 8 {
        return false;
    }
    for i in 0..bits {
        let bit = offset + i;
        let byte = &mut data[(bit / 8) as usize];
        let mask = 1u8 << (bit % 8);
        if (value >> i) & 1 != 0 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
    true
}
