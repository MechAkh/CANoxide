//! Bus worker v0.2 commands against the simulator: tasks (PDO read, node
//! info), transmit list, signals with SDO polling, trace log, LSS, bus load,
//! listen-only refusal.

use canoxide::bus::*;
use canoxide::can::{Bitrate, CanFrame, Dir};
use canoxide::canopen::lss::{LssCommand, LssResult};
use canoxide::canopen::pdo::{PdoKind, PdoReadOptions, PdoReadTask};
use canoxide::canopen::task::TaskResult;
use canoxide::canopen::types::DisplayMode;
use canoxide::signals::{SignalDef, SignalSource};
use canoxide::txlist::{Trigger, TxEntry};
use std::sync::Arc;
use std::time::{Duration, Instant};

const NODE: u8 = 5;

fn connect(listen_only: bool) -> BusHandle {
    let bus = BusHandle::spawn(Arc::new(|| {}));
    bus.send(BusCommand::Connect { adapter: AdapterChoice::Sim { node_id: NODE, od: None }, channel: 0, bitrate: Bitrate::K250, listen_only });
    assert!(wait_for(&bus, Duration::from_secs(5), |e| matches!(e, BusEvent::Connected { .. })).is_some());
    bus
}

/// Wait for an event; returns the frames seen on the way too.
fn wait_for(bus: &BusHandle, timeout: Duration, mut f: impl FnMut(&BusEvent) -> bool) -> Option<Vec<CanFrame>> {
    let end = Instant::now() + timeout;
    let mut frames = Vec::new();
    while Instant::now() < end {
        while let Some(e) = bus.try_recv() {
            if let BusEvent::Frames(v) = &e {
                frames.extend_from_slice(v);
            }
            if f(&e) {
                return Some(frames);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

fn frames_for(bus: &BusHandle, dur: Duration) -> Vec<CanFrame> {
    let end = Instant::now() + dur;
    let mut v = Vec::new();
    while Instant::now() < end {
        while let Some(e) = bus.try_recv() {
            if let BusEvent::Frames(f) = e {
                v.extend(f);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    v
}

#[test]
fn pdo_read_task_sets_the_worker_decoder_and_node_info_fills_the_overview() {
    let bus = connect(false);
    bus.send(BusCommand::Task { tag: 7, task: Box::new(PdoReadTask::new(NODE, PdoReadOptions::default())) });
    let mut cfg = None;
    let mut progress = 0;
    wait_for(&bus, Duration::from_secs(10), |e| match e {
        BusEvent::TaskProgress { tag: 7, .. } => {
            progress += 1;
            false
        }
        BusEvent::PdoConfigRead(c) => {
            cfg = Some(c.clone());
            false
        }
        BusEvent::TaskDone { tag: 7, result: TaskResult::PdoConfig(_) } => true,
        _ => false,
    })
    .expect("task done");
    let cfg = cfg.expect("PdoConfigRead");
    assert!(progress > 0);
    assert!(cfg.complete);
    assert_eq!(cfg.get(PdoKind::Tpdo, 1).unwrap().cob_id, 0x185);
    {
        let s = bus.shared.lock().unwrap();
        assert_eq!(s.pdo_configs.get(&NODE), Some(&cfg));
        assert!(s.task.is_none());
    }
    bus.send(BusCommand::ClearNodePdos(NODE));
    bus.send(BusCommand::RefreshNodeInfo { tag: 8, nodes: Some(vec![NODE]) });
    wait_for(&bus, Duration::from_secs(10), |e| matches!(e, BusEvent::TaskDone { tag: 8, result: TaskResult::NodeInfo(_) })).expect("node info");
    let s = bus.shared.lock().unwrap();
    assert!(s.pdo_configs.is_empty());
    let n = s.network.get(NODE).expect("node in overview");
    assert_eq!(n.device_name.as_deref(), Some("CANoxide simulator"));
    assert!(n.vendor_id.is_some() && n.last_seen_us.is_some());
}

#[test]
fn cancel_a_task_ends_it_with_a_partial_result() {
    let bus = connect(false);
    let opts = PdoReadOptions { stop_after_missing: 512, ..Default::default() };
    bus.send(BusCommand::Task { tag: 1, task: Box::new(PdoReadTask::new(NODE, opts)) });
    wait_for(&bus, Duration::from_secs(5), |e| matches!(e, BusEvent::TaskProgress { tag: 1, .. })).expect("running");
    bus.send(BusCommand::CancelTask(1));
    let mut complete = None;
    wait_for(&bus, Duration::from_secs(5), |e| match e {
        BusEvent::TaskDone { tag: 1, result: TaskResult::PdoConfig(c) } => {
            complete = Some(c.complete);
            true
        }
        _ => false,
    })
    .expect("done after cancel");
    assert_eq!(complete, Some(false));
}

#[test]
fn transmit_list_periodic_on_rx_fire_and_stop_on_disconnect() {
    let bus = connect(false);
    let f = CanFrame::new(0x321, &[0xAA]);
    bus.send(BusCommand::TxUpsert(TxEntry::from_frame(1, "p", &f, Trigger::Periodic { period_ms: 20 })));
    bus.send(BusCommand::TxUpsert(TxEntry::from_frame(2, "echo", &CanFrame::new(0x322, &[1]), Trigger::OnRx { cob_id: 0x185, extended: false })));
    bus.send(BusCommand::TxUpsert(TxEntry::from_frame(3, "once", &CanFrame::new(0x323, &[2]), Trigger::Manual)));
    // Nothing is sent before a start.
    let v = frames_for(&bus, Duration::from_millis(300));
    assert!(!v.iter().any(|f| f.dir == Dir::Tx), "{v:?}");
    bus.send(BusCommand::TxStart(1));
    bus.send(BusCommand::TxStart(2));
    bus.send(BusCommand::TxFire(3));
    // The node sends TPDO1 (0x185) every 200 ms once operational.
    bus.send(BusCommand::Nmt { cmd: canoxide::canopen::nmt::NmtCommand::Start, node: NODE });
    let v = frames_for(&bus, Duration::from_millis(600));
    let n321 = v.iter().filter(|f| f.id == 0x321 && f.dir == Dir::Tx).count();
    assert!((15..=40).contains(&n321), "{n321}");
    assert_eq!(v.iter().filter(|f| f.id == 0x323).count(), 1);
    let rx185 = v.iter().filter(|f| f.id == 0x185 && f.dir == Dir::Rx).count();
    let tx322 = v.iter().filter(|f| f.id == 0x322).count();
    assert!(rx185 >= 2 && tx322 == rx185, "rx {rx185} tx {tx322}");
    assert_eq!(bus.shared.lock().unwrap().txlist.iter().filter(|e| e.enabled).count(), 2);
    bus.send(BusCommand::Disconnect);
    wait_for(&bus, Duration::from_secs(5), |e| matches!(e, BusEvent::Disconnected)).unwrap();
    assert!(bus.shared.lock().unwrap().txlist.iter().all(|e| !e.enabled));
}

#[test]
fn signals_from_pdo_and_sdo_poll_bus_load_and_log() {
    let bus = connect(false);
    let dir = std::env::temp_dir().join(format!("canoxide_bus_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("trace.csv");
    // The worker decoder needs the EDS of the node for PDO signals.
    let mut nodes = std::collections::HashMap::new();
    nodes.insert(NODE, Arc::new(canoxide::can::sim::builtin_dictionary()));
    bus.send(BusCommand::SetDecodeNodes(nodes));
    let (pdo_sig, poll_sig);
    {
        let mut s = bus.signals.lock().unwrap();
        pdo_sig = s.add(SignalDef::new("in1", SignalSource::Pdo { node: NODE, index: 0x6000, sub: 1 }));
        poll_sig = s.add(SignalDef::new("type", SignalSource::SdoPoll { node: NODE, index: 0x1000, sub: 0, period_ms: 50, data_type: None }));
    }
    bus.send(BusCommand::StartLog { path: log.clone(), mode: DisplayMode::Hex, filter: None });
    wait_for(&bus, Duration::from_secs(2), |e| matches!(e, BusEvent::LogStarted { .. })).unwrap();
    // No poll without permission.
    let v = frames_for(&bus, Duration::from_millis(200));
    assert!(!v.iter().any(|f| f.dir == Dir::Tx));
    bus.send(BusCommand::SetSignalPolling(true));
    bus.send(BusCommand::Nmt { cmd: canoxide::canopen::nmt::NmtCommand::Start, node: NODE });
    let v = frames_for(&bus, Duration::from_millis(1200));
    let polls = v.iter().filter(|f| f.id == 0x600 + NODE as u32).count();
    assert!((8..=30).contains(&polls), "{polls}");
    {
        let s = bus.signals.lock().unwrap();
        assert!(s.get(pdo_sig).unwrap().count >= 3);
        assert!(s.get(poll_sig).unwrap().count >= 5);
        let sh = bus.shared.lock().unwrap();
        assert!(sh.bus_load.total_frames > 10 && sh.bus_load.current_pct > 0.0, "{:?}", sh.bus_load);
        assert!(sh.signal_polling);
    }
    bus.send(BusCommand::SetSignalPolling(false));
    bus.send(BusCommand::StopLog);
    let mut rows = 0;
    wait_for(&bus, Duration::from_secs(2), |e| match e {
        BusEvent::LogStopped { rows: r, error: None, .. } => {
            rows = *r;
            true
        }
        _ => false,
    })
    .unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count() as u64, rows + 1);
    assert!(text.lines().any(|l| l.contains(",Tx,605,") && l.contains("SDO")), "no SDO request row");
    assert!(text.lines().any(|l| l.contains(",Rx,185,")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lss_through_the_worker() {
    let bus = connect(false);
    bus.send(BusCommand::Lss { tag: 3, cmd: LssCommand::SwitchGlobal { configuration: true } });
    wait_for(&bus, Duration::from_secs(2), |e| matches!(e, BusEvent::LssDone { tag: 3, result: Ok(LssResult::Done) })).unwrap();
    bus.send(BusCommand::Lss { tag: 4, cmd: LssCommand::InquireNodeId });
    let mut id = None;
    wait_for(&bus, Duration::from_secs(2), |e| match e {
        BusEvent::LssDone { tag: 4, result } => {
            id = Some(result.clone());
            true
        }
        _ => false,
    })
    .unwrap();
    assert_eq!(id, Some(Ok(LssResult::NodeId(NODE))));
}

#[test]
fn listen_only_refuses_the_v02_senders() {
    let bus = connect(true);
    bus.send(BusCommand::TxUpsert(TxEntry::from_frame(1, "p", &CanFrame::new(0x321, &[1]), Trigger::Periodic { period_ms: 10 })));
    bus.send(BusCommand::TxStart(1));
    bus.send(BusCommand::TxFire(1));
    bus.send(BusCommand::SetSignalPolling(true));
    bus.send(BusCommand::Lss { tag: 1, cmd: LssCommand::InquireNodeId });
    bus.send(BusCommand::Task { tag: 2, task: Box::new(PdoReadTask::new(NODE, PdoReadOptions::default())) });
    bus.send(BusCommand::RefreshNodeInfo { tag: 3, nodes: Some(vec![NODE]) });
    let mut errors = 0;
    let mut tx = 0;
    let mut done = 0;
    let end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < end {
        while let Some(e) = bus.try_recv() {
            match e {
                BusEvent::Error(_) => errors += 1,
                BusEvent::Frames(v) => tx += v.iter().filter(|f| f.dir == Dir::Tx).count(),
                BusEvent::LssDone { .. } | BusEvent::TaskDone { .. } => done += 1,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!((errors, tx, done), (6, 0, 3));
    assert!(!bus.shared.lock().unwrap().signal_polling);
}
