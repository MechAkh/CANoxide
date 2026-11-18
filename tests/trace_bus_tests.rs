use canopen_tester::bus::{AdapterChoice, BusCommand, BusEvent, BusHandle};
use canopen_tester::can::{Bitrate, CanFrame};
use canopen_tester::canopen::decode::{decode, DecodeContext, MsgClass};
use canopen_tester::canopen::nmt::NodeState;
use canopen_tester::canopen::sdo::{SdoError, SdoRequest};
use canopen_tester::canopen::types::DisplayMode;
use canopen_tester::trace::{TraceFilter, TraceStore};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn push(t: &mut TraceStore, id: u32, data: &[u8], ts: u64) {
    let mut f = CanFrame::new(id, data);
    f.timestamp_us = ts;
    let d = decode(&f, &DecodeContext::default());
    t.push(f, d);
}

#[test]
fn ring_buffer_filter_and_fixed_view() {
    let mut t = TraceStore::new(5);
    for i in 0..8u64 {
        push(&mut t, 0x705, &[0x05], i * 1000);
    }
    push(&mut t, 0x185, &[1, 2], 9000);
    assert_eq!(t.len(), 5);
    assert_eq!(t.total(), 9);
    assert_eq!(t.view_len(), 5);
    assert_eq!(t.view_row(0).unwrap().seq, 4);
    assert_eq!(t.view_row(4).unwrap().frame.id, 0x185);
    let prev = t.previous(t.view_row(1).unwrap()).unwrap();
    assert_eq!(prev.seq, 4);

    let mut f = TraceFilter::default();
    f.set_class(MsgClass::Heartbeat, false);
    t.set_filter(f.clone());
    assert_eq!(t.view_len(), 1);
    f = TraceFilter { text: "OPERATIONAL".into(), ..Default::default() };
    t.set_filter(f);
    assert_eq!(t.view_len(), 4);
    t.set_filter(TraceFilter { node: Some(5), id_min: 0x180, id_max: 0x1FF, ..Default::default() });
    assert_eq!(t.view_len(), 1);
    t.set_filter(TraceFilter::default());

    let rows = t.fixed_rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].last.frame.id, 0x185);
    assert_eq!(rows[1].count, 8);
    assert_eq!(rows[1].cycle_us, Some(1000));

    let mut csv = Vec::new();
    t.write_csv(&mut csv, DisplayMode::Hex).unwrap();
    let s = String::from_utf8(csv).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 6);
    assert!(lines[0].starts_with("time_s,"));
    assert!(lines[5].contains("185"), "{}", lines[5]);
    assert!(lines[5].contains("01 02"), "{}", lines[5]);
    t.clear();
    assert!(t.is_empty());
    assert_eq!(t.view_len(), 0);
}

#[test]
fn csv_quotes_fields() {
    assert_eq!(canopen_tester::trace::csv_field("a,b"), "\"a,b\"");
    assert_eq!(canopen_tester::trace::csv_field("say \"x\""), "\"say \"\"x\"\"\"");
    assert_eq!(canopen_tester::trace::csv_field("plain"), "plain");
}

fn wait_for(bus: &BusHandle, timeout: Duration, mut f: impl FnMut(&BusEvent) -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        while let Some(e) = bus.try_recv() {
            if f(&e) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn bus_worker_with_simulator() {
    let bus = BusHandle::spawn(Arc::new(|| {}));
    bus.send(BusCommand::Connect { adapter: AdapterChoice::Sim { node_id: 7, od: None }, channel: 0, bitrate: Bitrate::K250, listen_only: false });
    assert!(wait_for(&bus, Duration::from_secs(5), |e| matches!(e, BusEvent::Connected { .. })));
    // Boot-up of the simulated node arrives without any send.
    assert!(wait_for(&bus, Duration::from_secs(5), |e| matches!(e, BusEvent::Frames(v) if v.iter().any(|f| f.id == 0x707))));
    bus.send(BusCommand::Sdo { tag: 42, node: 7, req: SdoRequest::Upload { index: 0x1008, sub: 0 } });
    let mut got = None;
    assert!(wait_for(&bus, Duration::from_secs(5), |e| {
        if let BusEvent::SdoDone { tag: 42, outcome } = e {
            got = Some(outcome.result.clone());
            true
        } else {
            false
        }
    }));
    assert_eq!(got.unwrap().unwrap(), b"CANopenTester simulator".to_vec());
    {
        let s = bus.shared.lock().unwrap();
        assert!(s.connected);
        assert_eq!(s.nodes.nodes.get(&7).map(|n| n.state), Some(NodeState::BootUp));
    }
    bus.send(BusCommand::Disconnect);
    assert!(wait_for(&bus, Duration::from_secs(5), |e| matches!(e, BusEvent::Disconnected)));
}

#[test]
fn listen_only_sends_nothing() {
    let bus = BusHandle::spawn(Arc::new(|| {}));
    bus.send(BusCommand::Connect { adapter: AdapterChoice::Sim { node_id: 3, od: None }, channel: 0, bitrate: Bitrate::K500, listen_only: true });
    assert!(wait_for(&bus, Duration::from_secs(5), |e| matches!(e, BusEvent::Connected { listen_only: true, .. })));
    bus.send(BusCommand::Send(CanFrame::new(0x123, &[1])));
    bus.send(BusCommand::Sdo { tag: 1, node: 3, req: SdoRequest::Upload { index: 0x1000, sub: 0 } });
    bus.send(BusCommand::Sync);
    let mut errors = 0;
    let mut tx = 0;
    let mut sdo_cancelled = false;
    let end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < end {
        while let Some(e) = bus.try_recv() {
            match e {
                BusEvent::Error(_) => errors += 1,
                BusEvent::Frames(v) => tx += v.iter().filter(|f| f.dir == canopen_tester::can::Dir::Tx).count(),
                BusEvent::SdoDone { outcome, .. } => sdo_cancelled = outcome.result == Err(SdoError::Cancelled),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(errors, 3);
    assert_eq!(tx, 0);
    assert!(sdo_cancelled);
}
