//! Extended (29-bit) IDs: a standard and an extended frame with the same
//! number are two different messages everywhere (statistics, user signals,
//! raw Graph signals, the transmit list, Find signal, the simulator demo).
//! All IDs and data are made up or public J1939 IDs.

use canoxide::analysis::{FindSettings, FindSignal, FindStep};
use canoxide::busload::frame_bits;
use canoxide::can::sim::{SimAdapter, SimNode, EXT_DEMO_FRAMES};
use canoxide::can::{Bitrate, CanAdapter, CanFrame, CanId, Dir};
use canoxide::canopen::decode::{decode, DecodeContext, MsgClass};
use canoxide::msgstats::{MsgKey, MsgStats};
use canoxide::signals::{RawKind, SignalDef, SignalSource, SignalStore};
use canoxide::txlist::{Trigger, TxEntry, TxList};
use canoxide::usersig::UserSignal;
use std::time::{Duration, Instant};

fn rx(id: u32, extended: bool, data: &[u8], t_us: u64) -> CanFrame {
    let mut f = CanFrame::new(id, data);
    f.extended = extended;
    f.dir = Dir::Rx;
    f.timestamp_us = t_us;
    f
}

#[test]
fn messages_rows_and_statistics_stay_apart() {
    // Settings > Extended IDs are: Raw frames.
    let mut ctx = DecodeContext::default();
    ctx.set_j1939(false);
    let mut s = MsgStats::new();
    for (i, (ext, b)) in [(false, 1u8), (true, 2), (false, 3), (false, 4)].into_iter().enumerate() {
        let f = rx(0x181, ext, &[b], i as u64 * 1000);
        s.on_frame(&f, &decode(&f, &ctx));
    }
    assert_eq!(s.len(), 2);
    let st = s.get(MsgKey::std(0x181)).unwrap();
    let ex = s.get(MsgKey::ext(0x181)).unwrap();
    assert_eq!((st.count, st.data[0]), (3, 4));
    assert_eq!((ex.count, ex.data[0]), (1, 2));
    // No CANopen meaning for the extended frame.
    assert_eq!(st.label, "TPDO1");
    assert_eq!((ex.class, ex.node, ex.label.as_str()), (MsgClass::Other, None, "Ext"));
    // The text filter finds each by its own text form.
    assert!(ex.matches("00000181") && !st.matches("00000181"));
    // Rows sort standard first.
    let mut keys: Vec<MsgKey> = s.entries().map(|e| e.key).collect();
    keys.sort();
    assert_eq!(keys, vec![CanId::std(0x181), CanId::ext(0x181)]);
}

#[test]
fn user_signal_decodes_only_its_format() {
    let us = UserSignal::new("x", 0x123, true, 0, 8);
    assert!(us.matches(0x123, true) && !us.matches(0x123, false));
    // An old file without "extended" is a standard signal.
    let old: UserSignal = serde_json::from_str(r#"{"name":"a","cob_id":291,"start_bit":0,"bits":8,"kind":"unsigned"}"#).unwrap();
    assert!(!old.extended && old.matches(0x123, false));
    // Through the signal store (the dashboard / graph path).
    let ctx = DecodeContext::default();
    let mut st = SignalStore::default();
    let e = st.add(us.to_def());
    let s = st.add(old.to_def());
    st.on_frame(&rx(0x123, true, &[7], 0), &ctx);
    st.on_frame(&rx(0x123, false, &[9], 1000), &ctx);
    assert_eq!(st.get(e).unwrap().last, Some(7.0));
    assert_eq!(st.get(s).unwrap().last, Some(9.0));
    assert_eq!((st.get(e).unwrap().count, st.get(s).unwrap().count), (1, 1));
}

#[test]
fn raw_graph_source_reads_only_its_format() {
    let ctx = DecodeContext::default();
    let mut st = SignalStore::default();
    let raw = |extended| SignalSource::Raw { cob_id: 0x18FE_F100 & 0x7FF, extended, start_bit: 0, bits: 8, raw_kind: RawKind::Unsigned, big_endian: false };
    let a = st.add(SignalDef::new("std", raw(false)));
    let b = st.add(SignalDef::new("ext", raw(true)));
    let c = st.add(SignalDef::new("j1939", SignalSource::Raw { cob_id: 0x18FE_F100, extended: true, start_bit: 8, bits: 8, raw_kind: RawKind::Unsigned, big_endian: false }));
    st.on_frame(&rx(0x100, false, &[1, 0], 0), &ctx);
    st.on_frame(&rx(0x100, true, &[2, 0], 1000), &ctx);
    st.on_frame(&rx(0x18FE_F100, true, &[0, 0x55], 2000), &ctx);
    assert_eq!(st.get(a).unwrap().last, Some(1.0));
    assert_eq!(st.get(b).unwrap().last, Some(2.0));
    assert_eq!(st.get(c).unwrap().last, Some(85.0));
    assert_eq!((st.get(a).unwrap().count, st.get(b).unwrap().count, st.get(c).unwrap().count), (1, 1, 1));
}

#[test]
fn transmit_on_receive_matches_the_pair() {
    let mut l = TxList::default();
    let mut ext = CanFrame::new(0x18FE_F100, &[1, 2]);
    ext.extended = true;
    let k1 = l.free_key();
    l.upsert(TxEntry::from_frame(k1, "on ext 123", &ext, Trigger::OnRx { cob_id: 0x123, extended: true }));
    let k2 = l.free_key();
    l.upsert(TxEntry::from_frame(k2, "on std 123", &CanFrame::new(0x321, &[3]), Trigger::OnRx { cob_id: 0x123, extended: false }));
    l.start(k1, 0);
    l.start(k2, 0);
    let out = l.on_rx(&rx(0x123, true, &[], 0));
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].id, out[0].extended), (0x18FE_F100, true));
    let out = l.on_rx(&rx(0x123, false, &[], 0));
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].id, out[0].extended), (0x321, false));
    // Saved and loaded as extended.
    let dir = std::env::temp_dir().join(format!("canoxide_ext_tx_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(canoxide::txlist::TXLIST_FILE);
    l.save(&p).unwrap();
    let back = TxList::load(&p).unwrap();
    let e = back.get(k1).unwrap();
    assert_eq!((e.id, e.extended, e.trigger.clone()), (0x18FE_F100, true, Trigger::OnRx { cob_id: 0x123, extended: true }));
    assert!(e.frame().extended);
    // An old file without the "extended" fields: standard.
    std::fs::write(&p, r#"{"entries":[{"key":1,"name":"old","id":385,"rtr":false,"dlc":1,"data":[1,0,0,0,0,0,0,0],"trigger":{"kind":"on_rx","cob_id":385}}]}"#).unwrap();
    let old = TxList::load(&p).unwrap();
    assert_eq!(old.entries.len(), 1);
    assert!(!old.entries[0].extended);
    assert_eq!(old.entries[0].trigger, Trigger::OnRx { cob_id: 0x181, extended: false });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_signal_keeps_same_number_apart() {
    let std_d = [10u8];
    let ext_d = [20u8];
    // Only the extended 123.
    let mut fs = FindSignal::new(FindSettings { sizes: vec![8], ids: vec![CanId::ext(0x123)], ..FindSettings::default() });
    fs.reset([(CanId::std(0x123), &std_d[..]), (CanId::ext(0x123), &ext_d[..])]);
    assert_eq!(fs.candidates().len(), 1);
    assert_eq!((fs.candidates()[0].key, fs.candidates()[0].value), (CanId::ext(0x123), 20.0));
    // Both: two candidates; a step on the extended data keeps only that one.
    let mut fs = FindSignal::new(FindSettings { sizes: vec![8], ..FindSettings::default() });
    fs.reset([(CanId::std(0x123), &std_d[..]), (CanId::ext(0x123), &ext_d[..])]);
    assert_eq!(fs.candidates().len(), 2);
    fs.step(FindStep::Changed, |k| Some(if k.extended { vec![21] } else { vec![10] }));
    assert_eq!(fs.candidates().len(), 1);
    assert_eq!(fs.candidates()[0].key, CanId::ext(0x123));
}

#[test]
fn bus_load_uses_the_extended_length() {
    let s = CanFrame::new(0x123, &[0; 4]);
    let mut e = s;
    e.extended = true;
    // 20 more bits of header (SRR, IDE, 18 ID bits) plus their stuffing.
    assert!(frame_bits(&e) > frame_bits(&s) + 18, "{} vs {}", frame_bits(&e), frame_bits(&s));
}

/// Frames of the simulator over `ms` milliseconds.
fn sim_frames(demo: bool, ms: u64) -> Vec<CanFrame> {
    let mut a = SimAdapter::new(SimNode::builtin(1)).with_extended_demo(demo);
    a.open(0, Bitrate::K250, false).unwrap();
    let mut v = Vec::new();
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if let Some(f) = a.receive(Duration::from_millis(1)).unwrap() {
            v.push(f);
        }
    }
    // An extended frame sent to the simulator is accepted and ignored.
    let mut x = CanFrame::new(0x601, &[0x40, 0x00, 0x10, 0x00, 0, 0, 0, 0]);
    x.extended = true;
    a.send(&x).unwrap();
    let mut answers = 0;
    for _ in 0..5 {
        if let Some(f) = a.receive(Duration::from_millis(1)).unwrap() {
            if !f.extended && f.id == 0x581 {
                answers += 1;
            }
        }
    }
    assert_eq!(answers, 0, "an extended frame got an SDO answer");
    a.close();
    v
}

#[test]
fn simulator_extended_demo_frames() {
    // Off (the default): no extended frame at all.
    assert!(sim_frames(false, 120).iter().all(|f| !f.extended));
    let v = sim_frames(true, 330);
    let of = |id: u32| v.iter().filter(|f| f.extended && f.id == id).collect::<Vec<_>>();
    let eec1 = of(0x0CF0_0400);
    let ccvs = of(0x18FE_F100);
    let et1 = of(0x18FE_EE00);
    assert!(eec1.len() >= 5, "0CF00400 every 50 ms: {}", eec1.len());
    assert!((2..=5).contains(&ccvs.len()), "18FEF100 every 100 ms: {}", ccvs.len());
    assert_eq!(et1.len(), 1, "18FEEE00 every 1000 ms");
    assert_eq!(EXT_DEMO_FRAMES.len(), 4);
    // Changing data, Rx, 8 bytes, and no CANopen meaning.
    assert_ne!(eec1[0].data, eec1[1].data);
    assert!(eec1.iter().all(|f| f.dir == Dir::Rx && f.dlc == 8));
    let mut raw = DecodeContext::default();
    raw.set_j1939(false);
    let d = decode(eec1[0], &raw);
    assert_eq!((d.class, d.node), (MsgClass::Other, None));
    // J1939 (the default): the source address is the node.
    let d = decode(eec1[0], &DecodeContext::default());
    assert_eq!((d.class, d.node, d.label.as_str()), (MsgClass::J1939, Some(0), "J1939"));
    // The CANopen node still runs (boot-up 701).
    assert!(v.iter().any(|f| !f.extended && f.id == 0x701));
}
