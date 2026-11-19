//! PDO configuration read / apply (A2) and network overview (A4) against
//! the simulator.

use canoxide::can::sim::{builtin_dictionary, SimNode};
use canoxide::can::CanFrame;
use canoxide::canopen::decode::{decode, DecodeContext, MsgClass};
use canoxide::canopen::nmt::{nmt_frame, NmtCommand};
use canoxide::canopen::pdo::*;
use canoxide::canopen::task::{StepsReport, StepsTask, TaskResult};
use canoxide::netinfo::{heartbeat_steps, NetworkOverview, NodeInfoTask};
use canoxide::session::{Ctl, Session};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const NODE: u8 = 5;

fn session() -> Session {
    Session::sim(SimNode::builtin(NODE), false)
}

fn read_pdos(s: &mut Session) -> NodePdoConfig {
    match s.run_task(Box::new(PdoReadTask::new(NODE, PdoReadOptions::default())), &mut Ctl::default()).unwrap() {
        TaskResult::PdoConfig(c) => c,
        r => panic!("unexpected {r:?}"),
    }
}

fn run_steps(s: &mut Session, steps: Vec<canoxide::canopen::task::SdoStep>) -> StepsReport {
    match s.run_task(Box::new(StepsTask::new("test", steps, true)), &mut Ctl::default()).unwrap() {
        TaskResult::Steps(r) => r,
        r => panic!("unexpected {r:?}"),
    }
}

/// Frames with this identifier received in `ms`.
fn count_id(s: &mut Session, id: u32, ms: u64) -> Vec<CanFrame> {
    let mut v = Vec::new();
    s.listen(Duration::from_millis(ms), |f| {
        if f.id == id && !f.extended {
            v.push(*f);
        }
        true
    })
    .unwrap();
    v
}

fn start(s: &mut Session) {
    s.send(nmt_frame(NmtCommand::Start, NODE)).unwrap();
}

#[test]
fn pdo_read_equals_the_dictionary() {
    let mut s = session();
    let c = read_pdos(&mut s);
    assert!(c.complete, "{:?}", c.errors);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(c.node, NODE);
    assert!(c.read_time > 1_600_000_000);
    assert!(c.read_time_text().ends_with(" UTC"));
    let sim = s.sim_node().unwrap().pdo_configs();
    assert_eq!(c.pdos, sim);
    let eds = configs_from_od(&builtin_dictionary(), NODE);
    assert_eq!(c.pdos, eds);
    assert_eq!(c.pdos.len(), 3);
    let t1 = c.get(PdoKind::Tpdo, 1).unwrap();
    assert_eq!((t1.cob_id, t1.valid, t1.trans_type, t1.event_timer_ms), (0x185, true, 255, 200));
    assert_eq!(t1.mapping, vec![MapEntry { index: 0x6000, sub: 1, bits: 8 }, MapEntry { index: 0x6000, sub: 2, bits: 8 }]);
    assert_eq!(t1.sync_start, None);
    let t2 = c.get(PdoKind::Tpdo, 2).unwrap();
    assert_eq!(t2.sync_start, Some(0));
    assert_eq!(t2.event_timer_ms, 1000);
    let r1 = c.get(PdoKind::Rpdo, 1).unwrap();
    assert_eq!(r1.cob_id, 0x205);
    assert_eq!(r1.mapping.len(), 2);
}

#[test]
fn pdo_read_of_one_pdo_and_stop_count() {
    let mut s = session();
    let opts = PdoReadOptions { only: Some((PdoKind::Tpdo, 2)), ..Default::default() };
    let TaskResult::PdoConfig(c) = s.run_task(Box::new(PdoReadTask::new(NODE, opts)), &mut Ctl::default()).unwrap() else { panic!() };
    assert_eq!(c.pdos.len(), 1);
    assert_eq!(c.pdos[0].label(), "TPDO2");
    // Stop after 1 missing: the scan of each direction ends at the first gap.
    let mut s = session();
    let _ = s.take_frames();
    let opts = PdoReadOptions { stop_after_missing: 1, ..Default::default() };
    let TaskResult::PdoConfig(c) = s.run_task(Box::new(PdoReadTask::new(NODE, opts)), &mut Ctl::default()).unwrap() else { panic!() };
    assert_eq!(c.pdos.len(), 3);
    // RPDO: 0x1400 found, 0x1401 missing → stop; no request for 0x1402.
    let reqs: Vec<u16> = s.take_frames().iter().filter(|f| f.id == 0x605 && f.data[0] == 0x40).map(|f| u16::from_le_bytes([f.data[1], f.data[2]])).collect();
    assert!(reqs.contains(&0x1401));
    assert!(!reqs.contains(&0x1402));
    assert!(reqs.contains(&0x1802));
    assert!(!reqs.contains(&0x1803));
}

#[test]
fn changed_cob_id_decodes_by_the_read_config() {
    let mut s = session();
    let c = read_pdos(&mut s);
    let t1 = c.get(PdoKind::Tpdo, 1).unwrap().clone();
    let rep = run_steps(&mut s, plan_change(NODE, &t1, &PdoChange { cob_id: Some(0x3A0), ..Default::default() }));
    assert!(rep.ok, "{:?}", rep.first_error());
    let c = read_pdos(&mut s);
    assert_eq!(c.get(PdoKind::Tpdo, 1).unwrap().cob_id, 0x3A0);
    start(&mut s);
    let frames = count_id(&mut s, 0x3A0, 500);
    assert!(!frames.is_empty(), "TPDO1 on its new COB-ID");
    assert!(count_id(&mut s, 0x185, 300).is_empty(), "nothing on the old COB-ID");

    let mut nodes = HashMap::new();
    nodes.insert(NODE, Arc::new(builtin_dictionary()));
    let eds_ctx = DecodeContext::new(nodes);
    let mut read_ctx = eds_ctx.clone();
    read_ctx.set_node_pdos(NODE, c.pdos.clone());
    let f = frames[0];
    let by_eds = decode(&f, &eds_ctx);
    assert_eq!(by_eds.node, Some(0x20), "the EDS default does not know 0x3A0");
    let by_read = decode(&f, &read_ctx);
    assert_eq!(by_read.class, MsgClass::Tpdo);
    assert_eq!(by_read.node, Some(NODE));
    assert_eq!(by_read.label, "TPDO1");
    assert!(by_read.text.contains("Read input 8 bit"), "{}", by_read.text);
    // The old COB-ID is no longer decoded with the TPDO1 mapping.
    let old = CanFrame::new(0x185, &[1, 2]);
    assert!(!decode(&old, &read_ctx).text.contains("Read input"));
    assert!(decode(&old, &eds_ctx).text.contains("Read input"));
    // Without an EDS, names are 0xIIII:SS.
    let mut bare = DecodeContext::default();
    bare.set_node_pdos(NODE, c.pdos.clone());
    let d = decode(&f, &bare);
    assert_eq!(d.label, "TPDO1");
    assert!(d.text.contains("0x6000:01"), "{}", d.text);
    assert_eq!(bare.node_pdos(NODE).unwrap().len(), 3);
    assert_eq!(bare.nodes_with_read_pdos(), vec![NODE]);
    bare.clear_node_pdos(NODE);
    assert!(!decode(&f, &bare).text.contains("0x6000:01"));
}

#[test]
fn enable_disable_and_event_timer_change_the_traffic() {
    let mut s = session();
    let c = read_pdos(&mut s);
    start(&mut s);
    let n = count_id(&mut s, 0x185, 650).len();
    assert!((2..=4).contains(&n), "event timer 200 ms: {n}");
    let t1 = c.get(PdoKind::Tpdo, 1).unwrap().clone();
    let rep = run_steps(&mut s, plan_enable(NODE, &t1, false));
    assert!(rep.ok);
    assert_eq!(rep.done.len(), 1);
    let _ = count_id(&mut s, 0x185, 20);
    assert_eq!(count_id(&mut s, 0x185, 450).len(), 0, "disabled");
    let off = read_pdos(&mut s);
    assert!(!off.get(PdoKind::Tpdo, 1).unwrap().valid);
    let t1_off = off.get(PdoKind::Tpdo, 1).unwrap().clone();
    let rep = run_steps(&mut s, plan_change(NODE, &t1_off, &PdoChange { valid: Some(true), event_timer_ms: Some(50), ..Default::default() }));
    assert!(rep.ok, "{:?}", rep.first_error());
    let n = count_id(&mut s, 0x185, 520).len();
    assert!(n >= 7, "event timer 50 ms: {n}");
    // Disable all TPDOs: nothing from either.
    let all = read_pdos(&mut s);
    let rep = run_steps(&mut s, plan_enable_all(NODE, &all.pdos, Some(PdoKind::Tpdo), false));
    assert!(rep.ok);
    assert_eq!(rep.done.len(), 2);
    let _ = count_id(&mut s, 0x185, 20);
    s.listen(Duration::from_millis(400), |f| {
        assert!(f.id != 0x185 && f.id != 0x285, "TPDO after disable all");
        true
    })
    .unwrap();
}

#[test]
fn mapping_change_uses_the_cia301_sequence_and_changes_the_frame() {
    let mut s = session();
    let c = read_pdos(&mut s);
    let t1 = c.get(PdoKind::Tpdo, 1).unwrap().clone();
    let steps = plan_map(NODE, &t1, vec![parse_map_entry("2002:0:32").unwrap()]);
    let what: Vec<(u16, u8)> = steps.iter().map(|s| (s.index, s.sub)).collect();
    assert_eq!(what, vec![(0x1800, 1), (0x1A00, 0), (0x1A00, 1), (0x1A00, 0), (0x1800, 1)]);
    assert_eq!(steps[0].data, (0x185u32 | 0x8000_0000).to_le_bytes());
    assert_eq!(steps[1].data, vec![0]);
    assert_eq!(steps[2].data, 0x2002_0020u32.to_le_bytes());
    assert_eq!(steps[3].data, vec![1]);
    assert_eq!(steps[4].data, 0x185u32.to_le_bytes());
    let rep = run_steps(&mut s, steps);
    assert!(rep.ok);
    start(&mut s);
    let f = count_id(&mut s, 0x185, 450);
    assert!(!f.is_empty());
    assert_eq!(f[0].dlc, 4);
    assert_eq!(f[0].payload(), (-100i32).to_le_bytes());
}

#[test]
fn a_failed_step_stops_the_sequence_with_the_abort() {
    let mut s = session();
    let c = read_pdos(&mut s);
    let t1 = c.get(PdoKind::Tpdo, 1).unwrap().clone();
    // 0x2000 (string) is not mappable.
    let rep = run_steps(&mut s, plan_map(NODE, &t1, vec![MapEntry { index: 0x2000, sub: 0, bits: 8 }]));
    assert!(!rep.ok);
    let e = rep.first_error().unwrap();
    assert_eq!(e.abort_code, Some(0x0604_0041));
    assert!(e.error.as_deref().unwrap().contains("cannot be mapped"), "{:?}", e.error);
    assert_eq!((e.step.index, e.step.sub), (0x1A00, 0));
    assert_eq!(rep.not_run.len(), 1, "the final COB-ID step did not run");
    // A COB-ID change while the PDO is valid is refused by the node.
    let mut s = session();
    let raw = 0x3A0u32.to_le_bytes().to_vec();
    let rep = run_steps(&mut s, vec![canoxide::canopen::task::SdoStep::write(NODE, 0x1800, 1, raw, "direct")]);
    assert_eq!(rep.first_error().unwrap().abort_code, Some(0x0609_0030));
}

#[test]
fn parse_helpers() {
    assert_eq!(parse_pdo_name("tpdo1"), Some((PdoKind::Tpdo, 1)));
    assert_eq!(parse_pdo_name("RPDO12"), Some((PdoKind::Rpdo, 12)));
    assert_eq!(parse_pdo_name("tpdo0"), None);
    assert_eq!(parse_pdo_name("tpdo513"), None);
    assert_eq!(parse_pdo_name("pdo1"), None);
    assert_eq!(parse_map_entry("0x6000:01:8"), Some(MapEntry { index: 0x6000, sub: 1, bits: 8 }));
    assert_eq!(parse_map_entry("6000:1:65"), None);
    assert_eq!(parse_map_entry("6000:1"), None);
    assert_eq!(comm_index(PdoKind::Rpdo, 1), 0x1400);
    assert_eq!(comm_index(PdoKind::Tpdo, 512), 0x19FF);
    assert_eq!(pdo_of_index(0x1A01), Some((PdoKind::Tpdo, 2, true)));
    assert_eq!(pdo_of_index(0x1000), None);
    assert_eq!(unix_time_text(0), "1970-01-01 00:00:00 UTC");
    assert_eq!(unix_time_text(1_700_000_000), "2023-11-14 22:13:20 UTC");
}

#[test]
fn node_info_and_heartbeat_of_all_nodes() {
    let mut s = session();
    s.sdo.timeout_ms = 100;
    let TaskResult::NodeInfo(infos) = s.run_task(Box::new(NodeInfoTask::new(&[NODE, 9])), &mut Ctl::default()).unwrap() else { panic!() };
    assert_eq!(infos.len(), 2);
    let n5 = &infos[0];
    assert_eq!(n5.node, NODE);
    assert_eq!(n5.vendor_id, Some(0x069B));
    assert!(n5.device_type.is_some());
    assert_eq!(n5.error_register, Some(0));
    assert!(n5.device_name.as_deref().unwrap().ends_with("simulator"), "{:?}", n5.device_name);
    assert!(n5.read_errors.is_empty(), "{:?}", n5.read_errors);
    let n9 = &infos[1];
    assert_eq!(n9.device_type, None);
    assert_eq!(n9.read_errors.len(), 1, "node 9 skipped after the first timeout");
    let mut ov = NetworkOverview::new();
    for i in &infos {
        ov.merge(i);
    }
    assert_eq!(ov.known_nodes(), vec![NODE, 9]);
    // Heartbeat 250 ms on all known nodes (node 9 fails, the rest goes on).
    let steps = heartbeat_steps(&[NODE], 250);
    let rep = run_steps(&mut s, steps);
    assert!(rep.ok);
    assert_eq!(s.sim_node().unwrap().value(0x1017, 0).unwrap(), 250u16.to_le_bytes());
    let hb = count_id(&mut s, 0x705, 800).len();
    assert!((2..=4).contains(&hb), "heartbeat every 250 ms: {hb}");
}

#[test]
fn network_overview_from_frames() {
    let mut ov = NetworkOverview::new();
    let mut hb = CanFrame::new(0x705, &[0x05]);
    hb.timestamp_us = 1000;
    hb.dir = canoxide::can::Dir::Rx;
    ov.on_frame(&hb);
    let mut em = CanFrame::new(0x085, &[0x10, 0x81, 0x11, 1, 2, 3, 4, 5]);
    em.timestamp_us = 2000;
    em.dir = canoxide::can::Dir::Rx;
    ov.on_frame(&em);
    let n = ov.get(NODE).unwrap();
    assert_eq!(n.state.as_deref(), Some("Operational"));
    assert_eq!(n.last_seen_us, Some(2000));
    let e = n.last_emcy.as_ref().unwrap();
    assert_eq!((e.code, e.register), (0x8110, 0x11));
    assert_eq!(e.data, vec![1, 2, 3, 4, 5]);
    assert!(e.text.contains("CAN overrun"));
    // Sent frames do not count.
    let mut tx = CanFrame::new(0x706, &[0x05]);
    tx.dir = canoxide::can::Dir::Tx;
    ov.on_frame(&tx);
    assert!(ov.get(6).is_none());
}
