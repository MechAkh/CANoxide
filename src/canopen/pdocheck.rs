//! The check of one PDO configuration before it is written (CiA 301): the
//! mapping length, the mapped objects against the EDS, the COB-ID and the
//! transmission type. An error means that the node will refuse the
//! configuration or that it breaks the network; a warning is something to
//! look at.

use super::pdo::{PdoConfig, PdoKind};
use super::types::DataType;
use crate::eds::ObjectDictionary;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// A short fixed key, for example `too_long` or `cob_restricted`.
    pub code: &'static str,
    /// The 0-based mapping entry the finding is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<usize>,
    pub message: String,
}

impl Finding {
    fn new(severity: Severity, code: &'static str, entry: Option<usize>, message: impl Into<String>) -> Finding {
        Finding { severity, code, entry, message: message.into() }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// "error: ..." / "warning: entry 2: ...".
    pub fn text(&self) -> String {
        match self.entry {
            Some(i) => format!("{}: entry {}: {}", self.severity.label(), i + 1, self.message),
            None => format!("{}: {}", self.severity.label(), self.message),
        }
    }
}

/// A PDO of a node, for the COB-ID comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtherPdo {
    pub node: u8,
    pub config: PdoConfig,
}

/// The CiA 301 COB-ID ranges that a PDO must not use (11-bit IDs).
pub const RESTRICTED: [(u32, u32); 8] = [(0x000, 0x000), (0x001, 0x07F), (0x101, 0x180), (0x581, 0x5FF), (0x601, 0x67F), (0x6E0, 0x6FF), (0x701, 0x77F), (0x780, 0x7FF)];

/// The default PDO COB-ID bases of CiA 301 (+ node-ID): TPDO1, RPDO1, ...
/// TPDO4, RPDO4.
const DEFAULT_BASES: [(u32, &str); 8] = [(0x180, "TPDO1"), (0x200, "RPDO1"), (0x280, "TPDO2"), (0x300, "RPDO2"), (0x380, "TPDO3"), (0x400, "RPDO3"), (0x480, "TPDO4"), (0x500, "RPDO4")];

/// Check the configuration `cfg` of node `node` before it is written. `od`
/// is the EDS of the node (`None`: the object checks are skipped, and one
/// finding says so). `others` are the known PDOs of this node and of other
/// devices; the entry of `cfg` itself (same node, kind and number) is
/// ignored. Errors come first.
pub fn check(node: u8, cfg: &PdoConfig, od: Option<&ObjectDictionary>, others: &[OtherPdo]) -> Vec<Finding> {
    let mut out = Vec::new();
    check_mapping(cfg, od, &mut out);
    if cfg.valid {
        check_cob_id(node, cfg, others, &mut out);
    }
    check_trans_type(cfg, &mut out);
    out.sort_by_key(|f| f.severity);
    out
}

/// True when one of `findings` is an error.
pub fn has_errors(findings: &[Finding]) -> bool {
    findings.iter().any(Finding::is_error)
}

fn check_mapping(cfg: &PdoConfig, od: Option<&ObjectDictionary>, out: &mut Vec<Finding>) {
    use Severity::*;
    let total = cfg.total_bits();
    if total > 64 {
        out.push(Finding::new(Error, "too_long", None, format!("the mapped objects have {total} bits; a PDO holds at most 64")));
    }
    let n = cfg.mapping.len();
    if n > 64 {
        out.push(Finding::new(Error, "too_many_entries", None, format!("{n} mapping entries; at most 64")));
    } else if n > 8 && cfg.mapping.iter().all(|m| m.bits % 8 == 0) {
        out.push(Finding::new(Error, "too_many_entries", None, format!("{n} mapping entries with byte mapping; at most 8")));
    }
    for (i, m) in cfg.mapping.iter().enumerate() {
        if m.bits == 0 && !m.is_dummy() {
            out.push(Finding::new(Error, "zero_length", Some(i), format!("{:04X}:{:02X} has length 0", m.index, m.sub)));
        }
    }
    let Some(od) = od else {
        if !cfg.mapping.is_empty() {
            out.push(Finding::new(Warning, "no_eds", None, "no EDS: object checks skipped"));
        }
        return;
    };
    for (i, m) in cfg.mapping.iter().enumerate() {
        if m.is_dummy() {
            continue;
        }
        let key = format!("{:04X}:{:02X}", m.index, m.sub);
        let Some(e) = od.entry(m.index, m.sub) else {
            out.push(Finding::new(Error, "no_object", Some(i), format!("{key} is not in the EDS")));
            continue;
        };
        if let Some(dt) = e.dtype() {
            if let Some(size) = dt.size() {
                let want = size as u32 * 8;
                let ok = u32::from(m.bits) == want || (dt == DataType::Boolean && m.bits == 1);
                if !ok && m.bits != 0 {
                    out.push(Finding::new(Error, "length", Some(i), format!("{key} ({}) has {want} bits; the entry maps {}", dt.name(), m.bits)));
                }
            }
        }
        if !e.pdo_mapping {
            out.push(Finding::new(Warning, "not_mappable", Some(i), format!("{key}: the EDS says PDOMapping=0")));
        }
        match cfg.kind {
            PdoKind::Tpdo if !e.access.readable() => {
                out.push(Finding::new(Warning, "access", Some(i), format!("{key} is {} (a TPDO sends it: it must be readable)", e.access.label())));
            }
            PdoKind::Rpdo if !e.access.writable() => {
                out.push(Finding::new(Warning, "access", Some(i), format!("{key} is {} (an RPDO writes it: it must be writable)", e.access.label())));
            }
            _ => {}
        }
    }
}

fn check_cob_id(node: u8, cfg: &PdoConfig, others: &[OtherPdo], out: &mut Vec<Finding>) {
    use Severity::*;
    let id = cfg.cob_id;
    if !cfg.extended {
        if let Some((a, b)) = RESTRICTED.iter().find(|(a, b)| (*a..=*b).contains(&id)) {
            let range = if a == b { format!("0x{a:03X}") } else { format!("0x{a:03X}..0x{b:03X}") };
            out.push(Finding::new(Error, "cob_restricted", None, format!("COB-ID 0x{id:03X} is in the restricted range {range} of CiA 301")));
        }
        for (base, name) in DEFAULT_BASES {
            if id > base && id <= base + 0x7F && id - base != u32::from(node) {
                out.push(Finding::new(Warning, "cob_default_other", None, format!("COB-ID 0x{id:03X} is the default {name} COB-ID of node {}", id - base)));
            }
        }
    }
    for o in others {
        let c = &o.config;
        if o.node == node && c.kind == cfg.kind && c.number == cfg.number {
            continue;
        }
        if c.valid && c.cob_id == id && c.extended == cfg.extended {
            let who = if o.node == node { format!("{} of this node", c.label()) } else { format!("{} of node {}", c.label(), o.node) };
            let pair = if o.node != node && c.kind != cfg.kind { " (a TPDO and an RPDO on one COB-ID are a normal link)" } else { "" };
            out.push(Finding::new(Warning, "cob_duplicate", None, format!("COB-ID 0x{id:03X} is also used by {who}{pair}")));
        }
    }
}

fn check_trans_type(cfg: &PdoConfig, out: &mut Vec<Finding>) {
    let t = cfg.trans_type;
    if (241..=251).contains(&t) {
        out.push(Finding::new(Severity::Warning, "trans_reserved", None, format!("transmission type {t} is reserved (241..251)")));
    }
    if cfg.kind == PdoKind::Rpdo && (t == 252 || t == 253) {
        out.push(Finding::new(Severity::Error, "trans_rtr_rpdo", None, format!("transmission type {t} (RTR only) is for a TPDO, not for an RPDO")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canopen::pdo::{configs_from_od, MapEntry};

    fn sim_od() -> ObjectDictionary {
        crate::can::sim::builtin_dictionary()
    }

    fn tpdo(cob: u32, mapping: Vec<MapEntry>) -> PdoConfig {
        PdoConfig { kind: PdoKind::Tpdo, number: 1, cob_id: cob, valid: true, extended: false, rtr_allowed: false, trans_type: 255, inhibit_100us: 0, event_timer_ms: 0, sync_start: None, mapping }
    }

    fn me(index: u16, sub: u8, bits: u8) -> MapEntry {
        MapEntry { index, sub, bits }
    }

    fn codes(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|f| f.code).collect()
    }

    #[test]
    fn the_simulator_pdos_are_clean() {
        let od = sim_od();
        let cfgs = configs_from_od(&od, 1);
        assert!(!cfgs.is_empty());
        let others: Vec<OtherPdo> = cfgs.iter().map(|c| OtherPdo { node: 1, config: c.clone() }).collect();
        for c in &cfgs {
            let f = check(1, c, Some(&od), &others);
            assert!(!has_errors(&f), "{}: {f:?}", c.label());
        }
        // A clean configuration: no finding at all.
        assert_eq!(check(1, &tpdo(0x181, vec![me(0x6000, 1, 8), me(0x2001, 0, 16)]), Some(&od), &[]), vec![]);
    }

    #[test]
    fn more_than_64_bits() {
        let c = tpdo(0x181, vec![me(0x2002, 0, 32), me(0x2002, 0, 32), me(0x6000, 1, 8)]);
        let f = check(1, &c, Some(&sim_od()), &[]);
        assert_eq!(f[0].code, "too_long");
        assert!(f[0].message.contains("72 bits"));
    }

    #[test]
    fn more_than_8_byte_entries() {
        let c = tpdo(0x181, vec![me(0x0005, 0, 8); 9]);
        assert!(codes(&check(1, &c, None, &[])).contains(&"too_many_entries"));
        // Bit mapping: 9 entries of 1 bit are fine.
        let c = tpdo(0x181, vec![me(0x0001, 0, 1); 9]);
        assert!(!codes(&check(1, &c, None, &[])).contains(&"too_many_entries"));
    }

    #[test]
    fn zero_length_not_dummy() {
        let f = check(1, &tpdo(0x181, vec![me(0x6000, 1, 0), me(0x0005, 0, 0)]), None, &[]);
        assert_eq!(f.iter().filter(|f| f.code == "zero_length").count(), 1);
        assert_eq!(f[0].entry, Some(0));
    }

    #[test]
    fn object_not_in_the_eds() {
        let f = check(1, &tpdo(0x181, vec![me(0x6000, 9, 8), me(0x5555, 0, 8)]), Some(&sim_od()), &[]);
        assert_eq!(codes(&f), vec!["no_object", "no_object"]);
        assert_eq!(f[1].entry, Some(1));
    }

    #[test]
    fn length_differs_from_the_data_type() {
        let f = check(1, &tpdo(0x181, vec![me(0x1000, 0, 8)]), Some(&sim_od()), &[]);
        assert_eq!(f[0].code, "length");
        assert!(f[0].message.contains("UNSIGNED32") && f[0].message.contains("32 bits"), "{}", f[0].message);
    }

    #[test]
    fn pdo_mapping_0_and_access() {
        // 0x2004: UNSIGNED8, wo, PDOMapping=0 in the simulator EDS.
        let f = check(1, &tpdo(0x181, vec![me(0x2004, 0, 8)]), Some(&sim_od()), &[]);
        assert_eq!(codes(&f), vec!["not_mappable", "access"]);
        assert!(f.iter().all(|f| f.severity == Severity::Warning));
        // 0x6000 is ro: fine for a TPDO, a warning for an RPDO.
        let mut r = tpdo(0x201, vec![me(0x6000, 1, 8)]);
        r.kind = PdoKind::Rpdo;
        assert_eq!(codes(&check(1, &r, Some(&sim_od()), &[])), vec!["access"]);
    }

    #[test]
    fn without_eds_once() {
        let f = check(1, &tpdo(0x181, vec![me(0x6000, 1, 8), me(0x6000, 2, 8)]), None, &[]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].message, "no EDS: object checks skipped");
    }

    #[test]
    fn restricted_cob_ids() {
        for id in [0x000, 0x001, 0x07F, 0x101, 0x180, 0x581, 0x5FF, 0x601, 0x67F, 0x6E0, 0x6FF, 0x701, 0x77F, 0x780, 0x7FF] {
            let f = check(1, &tpdo(id, vec![]), None, &[]);
            assert!(f.iter().any(|f| f.code == "cob_restricted" && f.is_error()), "0x{id:X}: {f:?}");
        }
        for id in [0x080, 0x100, 0x181, 0x580, 0x600, 0x680, 0x6DF, 0x700] {
            let f = check(1, &tpdo(id, vec![]), None, &[]);
            assert!(!codes(&f).contains(&"cob_restricted"), "0x{id:X}: {f:?}");
        }
        // A 29-bit ID and a disabled PDO are not checked against the ranges.
        let mut c = tpdo(0x601, vec![]);
        c.extended = true;
        assert!(check(1, &c, None, &[]).is_empty());
        let mut c = tpdo(0x000, vec![]);
        c.valid = false;
        assert!(check(1, &c, None, &[]).is_empty());
    }

    #[test]
    fn duplicate_cob_id() {
        let mut t2 = tpdo(0x381, vec![]);
        t2.number = 2;
        let own = OtherPdo { node: 1, config: t2.clone() };
        let me1 = OtherPdo { node: 1, config: tpdo(0x381, vec![]) };
        let mut r = tpdo(0x381, vec![]);
        r.kind = PdoKind::Rpdo;
        let other = OtherPdo { node: 5, config: r };
        let f = check(1, &tpdo(0x381, vec![]), None, &[me1, own, other]);
        let d: Vec<&Finding> = f.iter().filter(|f| f.code == "cob_duplicate").collect();
        assert_eq!(d.len(), 2, "{f:?}");
        assert!(d[0].message.contains("TPDO2 of this node"));
        assert!(d[1].message.contains("RPDO1 of node 5") && d[1].message.contains("normal link"));
        assert!(d.iter().all(|f| f.severity == Severity::Warning));
        // A disabled other PDO does not count.
        t2.valid = false;
        assert!(check(1, &tpdo(0x381, vec![]), None, &[OtherPdo { node: 1, config: t2 }]).is_empty());
    }

    #[test]
    fn default_cob_id_of_another_node() {
        let f = check(1, &tpdo(0x182, vec![]), None, &[]);
        assert_eq!(codes(&f), vec!["cob_default_other"]);
        assert!(f[0].message.contains("TPDO1 COB-ID of node 2"));
        assert!(check(1, &tpdo(0x181, vec![]), None, &[]).is_empty());
        assert!(check(1, &tpdo(0x481, vec![]), None, &[]).is_empty());
        assert_eq!(codes(&check(1, &tpdo(0x57F, vec![]), None, &[])), vec!["cob_default_other"]);
        assert!(check(1, &tpdo(0x680, vec![]), None, &[]).is_empty());
    }

    #[test]
    fn transmission_types() {
        let mut c = tpdo(0x181, vec![]);
        for t in [241, 251] {
            c.trans_type = t;
            assert_eq!(check(1, &c, None, &[])[0].code, "trans_reserved");
        }
        c.trans_type = 252;
        assert!(check(1, &c, None, &[]).is_empty(), "252 is fine for a TPDO");
        c.kind = PdoKind::Rpdo;
        c.cob_id = 0x201;
        for t in [252, 253] {
            c.trans_type = t;
            let f = check(1, &c, None, &[]);
            assert_eq!(f[0].code, "trans_rtr_rpdo");
            assert!(f[0].is_error());
        }
        c.trans_type = 254;
        assert!(check(1, &c, None, &[]).is_empty());
    }

    #[test]
    fn json_and_text() {
        let f = check(1, &tpdo(0x5A0, vec![me(0x6000, 1, 0)]), None, &[]);
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v[0]["severity"], "error");
        assert!(f.iter().any(|f| f.text().starts_with("error: entry 1:")));
        assert!(f.iter().any(|f| f.text() == "warning: no EDS: object checks skipped"));
    }
}
