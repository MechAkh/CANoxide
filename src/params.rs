//! Store parameters (0x1010) and restore default parameters (0x1011) of
//! CiA 301: the writes with the "save" / "load" signatures and the query of
//! the sub-indexes a node supports.

use crate::canopen::task::{ArrayReadTask, ReadsReport, SdoStep};
use crate::canopen::types::le_u64;
use crate::dcf::{RESTORE_SIGNATURE, STORE_SIGNATURE};

/// The write of "save" (0x65766173, UNSIGNED32 little endian) to 0x1010:sub.
pub fn store_parameters(node: u8, sub: u8) -> SdoStep {
    SdoStep::write(node, 0x1010, sub, STORE_SIGNATURE.to_le_bytes().to_vec(), format!("store parameters (0x1010:{sub:02} = \"save\")"))
}

/// The write of "load" (0x64616F6C, UNSIGNED32 little endian) to 0x1011:sub.
pub fn restore_defaults(node: u8, sub: u8) -> SdoStep {
    SdoStep::write(node, 0x1011, sub, RESTORE_SIGNATURE.to_le_bytes().to_vec(), format!("restore default parameters (0x1011:{sub:02} = \"load\")"))
}

/// The name of a sub-index of 0x1010 / 0x1011.
pub fn sub_name(sub: u8) -> String {
    match sub {
        1 => "All parameters".to_string(),
        2 => "Communication parameters".to_string(),
        3 => "Application parameters".to_string(),
        n => format!("Manufacturer specific (sub {n})"),
    }
}

/// The task that reads 0x1010:00 and 0x1011:00 and then each sub.
pub fn query_task(node: u8) -> ArrayReadTask {
    ArrayReadTask::new("read store / restore parameters", node, vec![0x1010, 0x1011])
}

/// One sub-index of 0x1010 or 0x1011.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamSub {
    pub sub: u8,
    pub name: String,
    /// The value read, `None` when the read failed.
    pub value: Option<u32>,
    pub error: Option<String>,
}

impl ParamSub {
    /// Bit 0: saves (0x1010) / restores (0x1011) on command.
    pub fn on_command(&self) -> bool {
        self.value.is_some_and(|v| v & 1 != 0)
    }

    /// Bit 1 of 0x1010: saves autonomously.
    pub fn autonomous(&self) -> bool {
        self.value.is_some_and(|v| v & 2 != 0)
    }
}

/// What a node supports for one of the two objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParamObject {
    /// The subs 1..=n (empty when sub 0 failed).
    pub subs: Vec<ParamSub>,
    /// The error text (with the abort text) when sub 0 failed.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParamCaps {
    pub store: ParamObject,
    pub restore: ParamObject,
}

fn object_of(r: &ReadsReport, index: u16) -> ParamObject {
    let mut o = ParamObject::default();
    match r.reads.iter().find(|x| x.index == index && x.sub == 0) {
        None => o.error = Some(format!("0x{index:04X}:00 not read")),
        Some(x) if x.data.is_none() => o.error = Some(format!("0x{index:04X}:00: {}", x.error.clone().unwrap_or_default())),
        Some(_) => {
            for x in r.reads.iter().filter(|x| x.index == index && x.sub >= 1) {
                o.subs.push(ParamSub { sub: x.sub, name: sub_name(x.sub), value: x.data.as_deref().map(|d| le_u64(d) as u32), error: x.error.clone() });
            }
        }
    }
    o
}

/// The capabilities of a finished `query_task`.
pub fn caps(r: &ReadsReport) -> ParamCaps {
    ParamCaps { store: object_of(r, 0x1010), restore: object_of(r, 0x1011) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::sim::SimNode;
    use crate::canopen::task::TaskResult;
    use crate::session::{Ctl, Session};

    #[test]
    fn signatures_are_save_and_load_little_endian() {
        let s = store_parameters(5, 1);
        assert_eq!((s.index, s.sub), (0x1010, 1));
        assert_eq!(s.data, b"save".to_vec());
        let r = restore_defaults(5, 2);
        assert_eq!((r.index, r.sub), (0x1011, 2));
        assert_eq!(r.data, b"load".to_vec());
        assert_eq!(sub_name(1), "All parameters");
        assert_eq!(sub_name(3), "Application parameters");
        assert_eq!(sub_name(7), "Manufacturer specific (sub 7)");
    }

    #[test]
    fn simulator_stores_restores_and_refuses_a_wrong_signature() {
        let mut n = SimNode::builtin(1);
        assert_eq!(n.write(0x1010, 1, b"save".to_vec()), Ok(()));
        assert_eq!(n.write(0x1010, 3, b"save".to_vec()), Ok(()));
        assert_eq!(n.write(0x1011, 1, b"load".to_vec()), Ok(()));
        assert_eq!((n.store_count, n.restore_count), (2, 1));
        assert_eq!(n.write(0x1010, 1, b"load".to_vec()), Err(0x0800_0020));
        assert_eq!(n.write(0x1011, 2, b"save".to_vec()), Err(0x0800_0020));
        assert_eq!((n.store_count, n.restore_count), (2, 1));
    }

    #[test]
    fn query_of_the_simulator() {
        let mut s = Session::sim(SimNode::builtin(1), false);
        let rep = match s.run_task(Box::new(query_task(1)), &mut Ctl::default()).unwrap() {
            TaskResult::Reads(r) => r,
            _ => panic!(),
        };
        let c = caps(&rep);
        assert_eq!(c.store.subs.len(), 3);
        assert_eq!(c.restore.subs.len(), 3);
        assert!(c.store.subs.iter().all(|x| x.on_command() && !x.autonomous()));
        assert_eq!(c.restore.subs[1].name, "Communication parameters");
    }
}
