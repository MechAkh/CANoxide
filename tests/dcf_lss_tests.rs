//! DCF save / compare / restore (A7) and the LSS master (A6) against the
//! simulator.

use canopen_tester::can::sim::{builtin_dictionary, SimNode, BUILTIN_EDS};
use canopen_tester::canopen::lss::{LssCommand, LssError, LssIdentity, LssResult, NODE_ID_UNCONFIGURED};
use canopen_tester::canopen::nmt::{nmt_frame, NmtCommand};
use canopen_tester::canopen::task::TaskResult;
use canopen_tester::dcf::*;
use canopen_tester::eds;
use canopen_tester::session::{Ctl, Session};
use std::sync::Arc;
use std::time::Duration;

const NODE: u8 = 5;

fn comm() -> DcfCommissioning {
    DcfCommissioning { node_id: NODE, baudrate_kbit: 250, node_name: "bench node".to_string(), net_number: 1, network_name: "test net".to_string() }
}

fn save(s: &mut Session) -> DcfSaveReport {
    let t = DcfSaveTask::new(NODE, Arc::new(builtin_dictionary()), BUILTIN_EDS.to_string(), comm(), "node5.dcf".to_string());
    match s.run_task(Box::new(t), &mut Ctl::default()).unwrap() {
        TaskResult::DcfSave(r) => r,
        r => panic!("unexpected {r:?}"),
    }
}

fn compare(s: &mut Session, dcf: &Arc<eds::ObjectDictionary>) -> DcfCompareReport {
    match s.run_task(Box::new(DcfCompareTask::new(NODE, dcf.clone(), false)), &mut Ctl::default()).unwrap() {
        TaskResult::DcfCompare(r) => r,
        r => panic!("unexpected {r:?}"),
    }
}

fn restore(s: &mut Session, dcf: &Arc<eds::ObjectDictionary>, opts: RestoreOptions) -> DcfRestoreReport {
    match s.run_task(Box::new(DcfRestoreTask::new(NODE, dcf.clone(), opts)), &mut Ctl::default()).unwrap() {
        TaskResult::DcfRestore(r) => r,
        r => panic!("unexpected {r:?}"),
    }
}

fn write(s: &mut Session, index: u16, sub: u8, data: Vec<u8>) {
    use canopen_tester::canopen::sdo::{SdoRequest, StartOptions};
    s.sdo(NODE, SdoRequest::Download { index, sub, data }, StartOptions::default()).unwrap().result.unwrap();
}

#[test]
fn dcf_save_then_parse() {
    let mut s = Session::sim(SimNode::builtin(NODE), false);
    let r = save(&mut s);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(r.values > 30, "{}", r.values);
    assert!(r.text.contains("\r\n"));
    assert!(!r.text.contains("\n\n"), "CRLF only");
    assert!(r.text.contains("[DeviceComissioning]"));
    assert!(r.text.contains("FileName=node5.dcf"));
    let dcf = eds::parse_str(&r.text);
    assert!(dcf.is_dcf);
    assert_eq!(dcf.node_id, Some(NODE));
    assert_eq!(dcf.baudrate_kbit, Some(250));
    // The values are the node's.
    let e = dcf.entry(0x2002, 0).unwrap();
    assert_eq!(eds::value_bytes(e.dtype().unwrap(), e.param_value.as_deref().unwrap(), NODE).unwrap(), (-100i32).to_le_bytes());
    let e = dcf.entry(0x1800, 1).unwrap();
    assert_eq!(e.param_value.as_deref(), Some("0x00000185"));
    let e = dcf.entry(0x1008, 0).unwrap();
    assert!(e.param_value.as_deref().unwrap().ends_with("simulator"));
    // Nothing was written by the save.
    assert_eq!(s.sim_node().unwrap().sdo_writes, 0);
}

#[test]
fn dcf_compare_shows_exactly_the_changed_values_and_restore_brings_them_back() {
    let mut s = Session::sim(SimNode::builtin(NODE), false);
    let dcf = Arc::new(eds::parse_str(&save(&mut s).text));
    assert!(compare(&mut s, &dcf).differences.is_empty(), "fresh compare");
    write(&mut s, 0x2002, 0, 77i32.to_le_bytes().to_vec());
    write(&mut s, 0x1017, 0, 300u16.to_le_bytes().to_vec());
    let c = compare(&mut s, &dcf);
    let keys: Vec<(u16, u8)> = c.differences.iter().map(|d| (d.index, d.sub)).collect();
    assert_eq!(keys, vec![(0x1017, 0), (0x2002, 0)]);
    let d = &c.differences[1];
    assert_eq!(d.dcf_value, "0xFFFFFF9C");
    assert_eq!(d.node_value.as_deref(), Some("0x0000004D"));
    assert!(c.compared > 20);

    // Dry run: nothing written, no download frame.
    let writes = s.sim_node().unwrap().sdo_writes;
    let _ = s.take_frames();
    let r = restore(&mut s, &dcf, RestoreOptions { dry_run: true, ..Default::default() });
    assert!(r.ok && r.dry_run);
    assert_eq!(r.count(|a| *a == ObjectAction::WouldWrite), 2);
    assert_eq!(s.sim_node().unwrap().sdo_writes, writes);
    assert!(s.take_frames().iter().all(|f| f.id != 0x605 || f.data[0] >> 5 != 1), "no SDO download in a dry run");
    assert_eq!(s.sim_node().unwrap().value(0x2002, 0).unwrap(), 77i32.to_le_bytes());

    // Restore.
    let r = restore(&mut s, &dcf, RestoreOptions::default());
    assert!(r.ok, "{:?}", r.steps.first_error());
    assert_eq!(r.count(|a| *a == ObjectAction::Written), 2);
    assert_eq!(r.steps.done.len(), 2);
    // Read-only and const objects are skipped.
    let ro = r.objects.iter().find(|o| (o.index, o.sub) == (0x2001, 0)).unwrap();
    assert_eq!(ro.action, ObjectAction::Skipped { reason: "read-only".to_string() });
    let c = r.objects.iter().find(|o| (o.index, o.sub) == (0x1008, 0)).unwrap();
    assert_eq!(c.action, ObjectAction::Skipped { reason: "const".to_string() });
    assert_eq!(s.sim_node().unwrap().value(0x2002, 0).unwrap(), (-100i32).to_le_bytes());
    assert!(compare(&mut s, &dcf).differences.is_empty());
    // Nothing to do the second time.
    let r = restore(&mut s, &dcf, RestoreOptions::default());
    assert!(r.steps.done.is_empty());
}

#[test]
fn dcf_restore_of_a_pdo_uses_the_sequence_and_changes_the_traffic() {
    let mut s = Session::sim(SimNode::builtin(NODE), false);
    let dcf = Arc::new(eds::parse_str(&save(&mut s).text));
    // TPDO1 event timer 50 ms (CiA sequence: invalid, timer, valid).
    write(&mut s, 0x1800, 1, (0x185u32 | 0x8000_0000).to_le_bytes().to_vec());
    write(&mut s, 0x1800, 5, 50u16.to_le_bytes().to_vec());
    write(&mut s, 0x1800, 1, 0x185u32.to_le_bytes().to_vec());
    s.send(nmt_frame(NmtCommand::Start, NODE)).unwrap();
    let count = |s: &mut Session, ms: u64| {
        let mut n = 0;
        s.listen(Duration::from_millis(ms), |f| {
            n += (f.id == 0x185) as usize;
            true
        })
        .unwrap();
        n
    };
    assert!(count(&mut s, 500) >= 7);
    let r = restore(&mut s, &dcf, RestoreOptions { from: Some(0x1800), to: Some(0x1800), ..Default::default() });
    assert!(r.ok, "{:?}", r.steps.first_error());
    let subs: Vec<(u16, u8)> = r.pdo_steps.iter().map(|s| (s.index, s.sub)).collect();
    assert_eq!(subs, vec![(0x1800, 1), (0x1800, 5), (0x1800, 1)]);
    assert_eq!(s.sim_node().unwrap().value(0x1800, 5).unwrap(), 200u16.to_le_bytes());
    let _ = count(&mut s, 20);
    let n = count(&mut s, 650);
    assert!((2..=4).contains(&n), "event timer back to 200 ms: {n}");
    // Range filter: only 0x1800 objects in the report.
    assert!(r.objects.iter().all(|o| o.index == 0x1800));
}

/// Set the ParameterValue line of one section of a DCF text.
fn set_param(text: &str, section: &str, value: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.split("\r\n") {
        if line.starts_with('[') {
            inside = line.eq_ignore_ascii_case(section);
        }
        if inside && line.to_ascii_lowercase().starts_with("parametervalue=") {
            out.push(format!("ParameterValue={value}"));
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\r\n")
}

#[test]
fn dcf_restore_stops_at_an_error_unless_continue_and_stores() {
    let mut s = Session::sim(SimNode::builtin(NODE), false);
    let text = save(&mut s).text;
    // A domain larger than the node takes (64 KiB) fails; 0x2100 comes later.
    let text = set_param(&text, "[2003]", &"00".repeat(70_000));
    let dcf = Arc::new(eds::parse_str(&text));
    let large = s.sim_node().unwrap().value(0x2100, 0).unwrap().to_vec();
    s.sim_node().unwrap().set_value(0x2100, 0, vec![1, 2, 3]);
    let r = restore(&mut s, &dcf, RestoreOptions::default());
    assert!(!r.ok);
    let f = r.objects.iter().find(|o| o.index == 0x2003).unwrap();
    assert!(matches!(&f.action, ObjectAction::Failed { abort_code: Some(0x0607_0012), .. }), "{:?}", f.action);
    let later = r.objects.iter().find(|o| (o.index, o.sub) == (0x2100, 0)).unwrap();
    assert_eq!(later.action, ObjectAction::NotRun);
    assert_eq!(s.sim_node().unwrap().value(0x2100, 0).unwrap(), [1, 2, 3]);
    let r = restore(&mut s, &dcf, RestoreOptions { continue_on_error: true, store: true, ..Default::default() });
    assert!(!r.ok);
    let later = r.objects.iter().find(|o| (o.index, o.sub) == (0x2100, 0)).unwrap();
    assert_eq!(later.action, ObjectAction::Written);
    assert_eq!(s.sim_node().unwrap().value(0x2100, 0).unwrap(), large.as_slice());
    assert_eq!(r.stored, Some(true));
    assert_eq!(r.steps.done.last().unwrap().step.index, 0x1010);
    assert_eq!(s.sim_node().unwrap().store_count, 1);
}

#[test]
fn lss_fastscan_configure_and_store_an_unconfigured_node() {
    let mut s = Session::sim(SimNode::builtin(NODE).unconfigured(), false);
    s.lss.timeout_ms = 20;
    // The unconfigured node says nothing.
    let mut seen = 0;
    s.listen(Duration::from_millis(100), |_| {
        seen += 1;
        true
    })
    .unwrap();
    assert_eq!(seen, 0);
    let id = s.sim_node().unwrap().identity();
    assert_eq!(s.lss(LssCommand::IdentifyNonConfigured, &mut Ctl::default()).unwrap(), Ok(LssResult::NonConfigured(1)));
    let mut steps = 0;
    let mut p = |_d: usize, _t: Option<usize>| steps += 1;
    let r = s.lss(LssCommand::Fastscan, &mut Ctl { cancel: None, progress: Some(&mut p) }).unwrap();
    assert_eq!(r, Ok(LssResult::Found(Some(id))));
    assert!(steps > 100);
    assert_eq!(s.lss(LssCommand::InquireNodeId, &mut Ctl::default()).unwrap(), Ok(LssResult::NodeId(NODE_ID_UNCONFIGURED)));
    assert_eq!(s.lss(LssCommand::ConfigureNodeId { node_id: 10 }, &mut Ctl::default()).unwrap(), Ok(LssResult::Done));
    assert_eq!(s.lss(LssCommand::ConfigureBitTiming { table: 0, index: 3 }, &mut Ctl::default()).unwrap(), Ok(LssResult::Done));
    assert_eq!(s.lss(LssCommand::StoreConfiguration, &mut Ctl::default()).unwrap(), Ok(LssResult::Done));
    assert_eq!(s.sim_node().unwrap().lss_store_count, 1);
    // Out of configuration: the node starts with ID 10.
    let _ = s.take_frames();
    assert_eq!(s.lss(LssCommand::SwitchGlobal { configuration: false }, &mut Ctl::default()).unwrap(), Ok(LssResult::Done));
    let mut boot = false;
    s.listen(Duration::from_millis(100), |f| {
        boot |= f.id == 0x70A && f.data[0] == 0;
        true
    })
    .unwrap();
    assert!(boot, "boot-up of node 10");
    assert_eq!(s.sim_node().unwrap().node_id, 10);
    assert!(s.sim_node().unwrap().is_configured());
    // A second fastscan finds nobody.
    assert_eq!(s.lss(LssCommand::Fastscan, &mut Ctl::default()).unwrap(), Ok(LssResult::Found(None)));
}

#[test]
fn lss_selective_inquire_and_errors() {
    let mut s = Session::sim(SimNode::builtin(NODE), false);
    s.lss.timeout_ms = 20;
    let id = s.sim_node().unwrap().identity();
    let wrong = LssIdentity { serial: id.serial ^ 1, ..id };
    assert_eq!(s.lss(LssCommand::SwitchSelective { identity: wrong }, &mut Ctl::default()).unwrap(), Err(LssError::Timeout));
    assert_eq!(s.lss(LssCommand::SwitchSelective { identity: id }, &mut Ctl::default()).unwrap(), Ok(LssResult::Done));
    assert_eq!(s.lss(LssCommand::InquireIdentity, &mut Ctl::default()).unwrap(), Ok(LssResult::Identity(id)));
    assert_eq!(s.lss(LssCommand::InquireNodeId, &mut Ctl::default()).unwrap(), Ok(LssResult::NodeId(NODE)));
    // Node-ID 0 is refused before anything is sent.
    let _ = s.take_frames();
    assert!(matches!(s.lss(LssCommand::ConfigureNodeId { node_id: 0 }, &mut Ctl::default()).unwrap(), Err(LssError::InvalidArgument(_))));
    assert!(s.take_frames().is_empty());
    // An unknown bit timing index is refused by the slave.
    match s.lss(LssCommand::ConfigureBitTiming { table: 0, index: 12 }, &mut Ctl::default()).unwrap() {
        Err(LssError::Refused { code, .. }) => assert_eq!(code, 1),
        r => panic!("{r:?}"),
    }
    assert_eq!(s.lss(LssCommand::ConfigureBitTiming { table: 0, index: 3 }, &mut Ctl::default()).unwrap(), Ok(LssResult::Done));
    assert_eq!(s.lss(LssCommand::ActivateBitTiming { delay_ms: 10 }, &mut Ctl::default()).unwrap(), Ok(LssResult::Done));
    assert_eq!(s.sim_node().unwrap().lss_bit_timing_requests.last().copied(), Some((3, 10)));
    // Listen-only refuses LSS before sending.
    let mut l = Session::sim(SimNode::builtin(NODE), true);
    assert!(l.lss(LssCommand::InquireNodeId, &mut Ctl::default()).is_err());
    assert!(l.take_frames().is_empty());
}
