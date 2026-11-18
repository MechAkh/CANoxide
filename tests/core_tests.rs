//! Signal store (A3), bus load and transmit list (A4).

use canopen_tester::busload::{frame_bits, BusLoad};
use canopen_tester::can::sim::builtin_dictionary;
use canopen_tester::can::{CanFrame, Dir};
use canopen_tester::canopen::decode::DecodeContext;
use canopen_tester::signals::*;
use canopen_tester::txlist::{Trigger, TxEntry, TxList};
use std::collections::HashMap;
use std::sync::Arc;

const NODE: u8 = 5;

fn ctx() -> DecodeContext {
    let mut nodes = HashMap::new();
    nodes.insert(NODE, Arc::new(builtin_dictionary()));
    DecodeContext::new(nodes)
}

fn rx(id: u32, data: &[u8], t_us: u64) -> CanFrame {
    let mut f = CanFrame::new(id, data);
    f.dir = Dir::Rx;
    f.timestamp_us = t_us;
    f
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("cot_core_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn signal_from_a_pdo() {
    let ctx = ctx();
    let mut st = SignalStore::default();
    assert_eq!(st.capacity(), DEFAULT_SIGNAL_CAPACITY);
    let mut def = SignalDef::new("input 2", SignalSource::Pdo { node: NODE, index: 0x6000, sub: 2 });
    def.scale = 0.5;
    def.offset = 1.0;
    def.unit = "V".to_string();
    let id = st.add(def);
    let other = st.add(SignalDef::new("input 1", SignalSource::Pdo { node: NODE, index: 0x6000, sub: 1 }));
    st.on_frame(&rx(0x185, &[0x12, 0x34], 1_000_000), &ctx);
    st.on_frame(&rx(0x185, &[0x12, 0x10], 2_000_000), &ctx);
    // A frame of another COB-ID does not count.
    st.on_frame(&rx(0x186, &[0xFF, 0xFF], 3_000_000), &ctx);
    let s = st.get(id).unwrap();
    assert_eq!(s.points().iter().copied().collect::<Vec<_>>(), vec![(1.0, 27.0), (2.0, 9.0)]);
    assert_eq!((s.last, s.min, s.max, s.count), (Some(9.0), Some(9.0), Some(27.0), 2));
    assert_eq!(st.get(other).unwrap().last, Some(18.0));
    assert_eq!(st.find("input 1").unwrap().id, other);
}

#[test]
fn signal_from_a_pdo_of_a_read_config_with_signed_type() {
    use canopen_tester::canopen::pdo::*;
    let mut ctx = ctx();
    let cfg = PdoConfig {
        kind: PdoKind::Tpdo,
        number: 2,
        cob_id: 0x290,
        valid: true,
        extended: false,
        rtr_allowed: true,
        trans_type: 255,
        inhibit_100us: 0,
        event_timer_ms: 100,
        sync_start: None,
        mapping: vec![MapEntry { index: 0x2001, sub: 0, bits: 16 }, MapEntry { index: 0x2002, sub: 0, bits: 32 }],
    };
    ctx.set_node_pdos(NODE, vec![cfg]);
    let mut st = SignalStore::new(100);
    let id = st.add(SignalDef::new("set point", SignalSource::Pdo { node: NODE, index: 0x2002, sub: 0 }));
    let mut d = vec![7, 0];
    d.extend_from_slice(&(-250i32).to_le_bytes());
    st.on_frame(&rx(0x290, &d, 500_000), &ctx);
    assert_eq!(st.get(id).unwrap().last, Some(-250.0));
}

#[test]
fn signal_from_an_sdo_poll() {
    let ctx = ctx();
    let mut st = SignalStore::default();
    assert!(!st.has_polls());
    let id = st.add(SignalDef::new("set point", SignalSource::SdoPoll { node: NODE, index: 0x2002, sub: 0, period_ms: 10, data_type: None }));
    assert!(st.has_polls());
    // The period is raised to 50 ms.
    let SignalSource::SdoPoll { period_ms, .. } = st.get(id).unwrap().def.source else { panic!() };
    assert_eq!(period_ms, MIN_POLL_MS);
    assert_eq!(st.polls_due(0), vec![PollRequest { id, node: NODE, index: 0x2002, sub: 0 }]);
    assert!(st.polls_due(49).is_empty());
    assert_eq!(st.polls_due(50).len(), 1);
    st.on_poll_value(id, 0.05, &(-100i32).to_le_bytes(), &ctx);
    // Type from the definition when given.
    let u = st.add(SignalDef::new("raw", SignalSource::SdoPoll { node: 9, index: 0x2002, sub: 0, period_ms: 100, data_type: Some(0x0007) }));
    st.on_poll_value(u, 0.1, &(-100i32).to_le_bytes(), &ctx);
    assert_eq!(st.get(id).unwrap().last, Some(-100.0));
    assert_eq!(st.get(u).unwrap().last, Some(4294967196.0));
    // PDO frames do not touch poll signals.
    st.on_frame(&rx(0x185, &[1, 2], 0), &ctx);
    assert_eq!(st.get(id).unwrap().count, 1);
}

#[test]
fn signal_from_raw_bits() {
    let ctx = DecodeContext::default();
    let mut st = SignalStore::default();
    let raw = |start_bit, bits, raw_kind, big_endian| SignalSource::Raw { cob_id: 0x300, extended: false, start_bit, bits, raw_kind, big_endian };
    let a = st.add(SignalDef::new("a", raw(4, 12, RawKind::Unsigned, false)));
    let b = st.add(SignalDef::new("b", raw(8, 8, RawKind::Signed, false)));
    let c = st.add(SignalDef::new("c", raw(16, 32, RawKind::Float, false)));
    // Motorola: start bit 7 (MSB of byte 0), 16 bits = bytes 0..1 big endian.
    let d = st.add(SignalDef::new("d", raw(7, 16, RawKind::Unsigned, true)));
    let e = st.add(SignalDef::new("e", SignalSource::Raw { cob_id: 0x300, extended: true, start_bit: 0, bits: 8, raw_kind: RawKind::Unsigned, big_endian: false }));
    let mut data = vec![0x5A, 0xFE];
    data.extend_from_slice(&1.5f32.to_le_bytes());
    st.on_frame(&rx(0x300, &data, 0), &ctx);
    assert_eq!(st.get(a).unwrap().last, Some(((0xFE5A >> 4) & 0xFFF) as f64));
    assert_eq!(st.get(b).unwrap().last, Some(-2.0));
    assert_eq!(st.get(c).unwrap().last, Some(1.5));
    assert_eq!(st.get(d).unwrap().last, Some(0x5AFE as f64));
    assert_eq!(st.get(e).unwrap().last, None, "extended flag must match");
    assert_eq!(raw_bits(&[0x12, 0x34], 7, 16, true), Some(0x1234));
    assert_eq!(raw_bits(&[0x12], 7, 16, true), None);
}

#[test]
fn ring_buffer_decimation_and_csv() {
    let mut st = SignalStore::new(1000);
    let id = st.add(SignalDef::new("wave, \"quoted\"", SignalSource::Raw { cob_id: 1, extended: false, start_bit: 0, bits: 8, raw_kind: RawKind::Unsigned, big_endian: false }));
    for i in 0..1500 {
        st.push_raw(id, i as f64 * 0.01, (i % 100) as f64);
    }
    let s = st.get(id).unwrap();
    assert_eq!(s.points().len(), 1000);
    assert_eq!(s.count, 1500);
    assert_eq!(s.points().front().unwrap().0, 5.0);
    // 5 s window (500 points) to 5 buckets: each holds 0..99.
    let v = st.slice(id, 5.0, 9.99, 5);
    assert_eq!(v.len(), 5);
    for p in &v {
        assert_eq!((p.min, p.max), (0.0, 99.0));
    }
    // Few points: no decimation.
    let v = st.slice(id, 5.0, 5.05, 100);
    assert_eq!(v.len(), 6);
    assert_eq!(v[1].min, v[1].max);
    let mut csv = Vec::new();
    st.write_csv(&mut csv).unwrap();
    let csv = String::from_utf8(csv).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "signal,unit,time_s,value");
    assert_eq!(lines.len(), 1001);
    assert_eq!(lines[1], "\"wave, \"\"quoted\"\"\",,5.000000,0");
    st.set_capacity(10);
    assert_eq!(st.get(id).unwrap().points().len(), 10);
    st.clear();
    assert_eq!(st.get(id).unwrap().count, 0);
    assert!(st.remove(id));
    assert!(st.list().is_empty());
}

#[test]
fn signal_definitions_persist() {
    let dir = tmp("signals");
    let path = dir.join(SIGNALS_FILE);
    let mut st = SignalStore::default();
    let mut d = SignalDef::new("t", SignalSource::SdoPoll { node: 3, index: 0x2002, sub: 0, period_ms: 200, data_type: Some(4) });
    d.unit = "degC".to_string();
    st.add(d.clone());
    st.add(SignalDef::new("r", SignalSource::Raw { cob_id: 0x123, extended: true, start_bit: 3, bits: 5, raw_kind: RawKind::Signed, big_endian: true }));
    st.save_defs(&path).unwrap();
    let mut back = SignalStore::default();
    assert_eq!(back.load_defs(&path).unwrap(), 2);
    assert_eq!(back.defs(), st.defs());
    assert_eq!(back.defs()[0], d);
    let mut none = SignalStore::default();
    assert_eq!(none.load_defs(&dir.join("missing.json")).unwrap(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bus_load_of_a_known_stream() {
    let std8 = CanFrame::new(0x123, &[0; 8]);
    assert_eq!(frame_bits(&std8), 135);
    assert_eq!(frame_bits(&CanFrame::new(0x123, &[])), 55);
    let mut ext8 = std8;
    ext8.extended = true;
    assert_eq!(frame_bits(&ext8), 160);
    let mut rtr = CanFrame::new(0x123, &[]);
    rtr.rtr = true;
    rtr.dlc = 8;
    assert_eq!(frame_bits(&rtr), 55);
    // 1000 frames of 135 bits per second at 250 kbit/s = 54 %.
    let mut bl = BusLoad::new(250);
    for i in 0..1000u64 {
        let mut f = std8;
        f.timestamp_us = i * 1000;
        bl.add(&f);
    }
    bl.advance(1_000_000);
    let s = bl.stats();
    assert!((s.current_pct - 54.0).abs() < 1e-9, "{}", s.current_pct);
    assert_eq!(s.frames_per_s, 1000.0);
    assert_eq!(s.total_frames, 1000);
    // A quiet second: current 0, peak stays, average half.
    bl.advance(2_000_000);
    let s = bl.stats();
    assert_eq!(s.current_pct, 0.0);
    assert!((s.peak_pct - 54.0).abs() < 1e-9);
    assert!((s.average_pct - 27.0).abs() < 1e-9, "{}", s.average_pct);
    bl.reset();
    assert_eq!(bl.stats().total_frames, 0);
}

#[test]
fn transmit_list_triggers() {
    let mut l = TxList::default();
    let k1 = l.free_key();
    l.upsert(TxEntry::from_frame(k1, "periodic", &CanFrame::new(0x100, &[1]), Trigger::Periodic { period_ms: 100 }));
    let k2 = l.free_key();
    l.upsert(TxEntry::from_frame(k2, "answer", &CanFrame::new(0x200, &[2]), Trigger::OnRx { cob_id: 0x185, extended: false }));
    let k3 = l.free_key();
    l.upsert(TxEntry::from_frame(k3, "manual", &CanFrame::new(0x300, &[3, 4]), Trigger::Manual));
    assert_eq!((k1, k2, k3), (1, 2, 3));
    // Nothing runs until started.
    assert!(l.due(0).is_empty());
    assert!(l.on_rx(&rx(0x185, &[], 0)).is_empty());
    assert!(!l.any_running());
    // Manual fire.
    assert_eq!(l.fire(k3).unwrap().payload(), [3, 4]);
    assert!(l.fire(99).is_none());
    // Periodic: at once, then every 100 ms.
    l.start(k1, 1000);
    assert!(l.any_running());
    let sent: Vec<u64> = (1000..1500).filter(|t| !l.due(*t).is_empty()).collect();
    assert_eq!(sent, vec![1000, 1100, 1200, 1300, 1400]);
    // On receive: only received frames of the right identifier.
    l.start(k2, 0);
    assert_eq!(l.on_rx(&rx(0x185, &[9], 0)).len(), 1);
    assert!(l.on_rx(&rx(0x186, &[9], 0)).is_empty());
    let mut tx = rx(0x185, &[9], 0);
    tx.dir = Dir::Tx;
    assert!(l.on_rx(&tx).is_empty(), "sent frames do not trigger");
    let mut ext = rx(0x185, &[9], 0);
    ext.extended = true;
    assert!(l.on_rx(&ext).is_empty());
    l.stop(k1);
    assert!(l.due(5000).is_empty());
    // Persist: entries come back stopped.
    let dir = tmp("txlist");
    let p = dir.join(canopen_tester::txlist::TXLIST_FILE);
    l.save(&p).unwrap();
    let back = TxList::load(&p).unwrap();
    assert_eq!(back.entries.len(), 3);
    assert!(back.entries.iter().all(|e| !e.enabled));
    assert_eq!(back.get(k2).unwrap().trigger, Trigger::OnRx { cob_id: 0x185, extended: false });
    assert!(TxList::load(&dir.join("none.json")).unwrap().entries.is_empty());
    l.stop_all();
    assert!(!l.any_running());
    l.remove(k2);
    assert!(l.get(k2).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
