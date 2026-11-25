//! J1939 on the 29-bit frames: the decoded texts, the Messages row text,
//! user and Graph signals matched by the PGN (any source address, or one),
//! the old user signal file, and the transport protocol through the
//! tracker. All IDs and data are made up or public J1939 IDs; nothing is
//! sent.

use canoxide::can::{CanFrame, Dir};
use canoxide::canopen::decode::{decode, row_text, DecodeContext, J1939Tracker, MsgClass};
use canoxide::signals::SignalStore;
use canoxide::usersig::{MatchMode, UserSignal};
use std::sync::Arc;

fn rx(id: u32, data: &[u8], t_us: u64) -> CanFrame {
    let mut f = CanFrame::new(id, data);
    f.extended = true;
    f.dir = Dir::Rx;
    f.timestamp_us = t_us;
    f
}

#[test]
fn decoded_texts_and_row_text() {
    let ctx = DecodeContext::default();
    let f = rx(0x0CF0_0400, &[0, 0, 0, 0x40, 0x1F, 0, 0, 0], 0);
    let d = decode(&f, &ctx);
    assert_eq!((d.class, d.node, d.label.as_str()), (MsgClass::J1939, Some(0), "J1939"));
    assert!(d.text.starts_with("PGN 61444 EEC1  P3  -> all"), "{}", d.text);
    assert_eq!(row_text(&f, &ctx), "PGN 61444 EEC1  SA 0");
    // PDU1: a request to SA 0 from 33.
    let r = rx(0x18EA_0021, &[0xEB, 0xFE, 0x00], 0);
    assert_eq!(row_text(&r, &ctx), "PGN 59904 RQST  SA 33");
    // EDP and DP both set: a raw frame even with J1939 on.
    let x = rx(0x1B00_0000, &[1], 0);
    assert_eq!(decode(&x, &ctx).class, MsgClass::Other);
    // Raw frames: no J1939 text.
    let mut raw = DecodeContext::default();
    raw.set_j1939(false);
    assert_eq!(decode(&f, &raw).class, MsgClass::Other);
    assert!(!row_text(&f, &raw).contains("PGN"));
}

#[test]
fn user_signal_by_pgn_any_or_one_source() {
    // Engine coolant temperature style field: byte 0 of PGN 65262.
    let mut any = UserSignal::new("Temp any", 0x18FE_EE00, true, 0, 8);
    any.match_mode = MatchMode::J1939Pgn;
    let mut one = any.clone();
    one.name = "Temp 33".into();
    one.source = Some(33);
    let exact = UserSignal::new("Temp exact", 0x18FE_EE00, true, 0, 8);
    // Another priority and another SA have the same PGN.
    assert!(any.matches_frame(0x18FE_EE21, true, true) && any.matches_frame(0x0CFE_EE00, true, true));
    assert!(one.matches_frame(0x18FE_EE21, true, true) && !one.matches_frame(0x18FE_EE00, true, true));
    assert!(!exact.matches_frame(0x18FE_EE21, true, true));
    // J1939 off: the exact ID only.
    assert!(!any.matches_frame(0x18FE_EE21, true, false) && any.matches_frame(0x18FE_EE00, true, false));
    // Not a standard frame.
    assert!(!any.matches_frame(0x0EE, false, true));

    // Through the signal store (the Graph and Dashboard path).
    let ctx = DecodeContext::default();
    let mut st = SignalStore::default();
    let a = st.add(any.to_def());
    let o = st.add(one.to_def());
    let e = st.add(exact.to_def());
    st.on_frame(&rx(0x18FE_EE00, &[10], 0), &ctx);
    st.on_frame(&rx(0x18FE_EE21, &[33], 1000), &ctx);
    assert_eq!((st.get(a).unwrap().count, st.get(a).unwrap().last), (2, Some(33.0)));
    assert_eq!((st.get(o).unwrap().count, st.get(o).unwrap().last), (1, Some(33.0)));
    assert_eq!((st.get(e).unwrap().count, st.get(e).unwrap().last), (1, Some(10.0)));

    // The Trace text of a frame from SA 33 names both PGN signals.
    let mut ctx = DecodeContext::default();
    ctx.set_user_signals(Arc::new(vec![any.clone(), one.clone(), exact.clone()]));
    let t = row_text(&rx(0x18FE_EE21, &[5], 0), &ctx);
    assert!(t.contains("Temp any=5") && t.contains("Temp 33=5") && !t.contains("Temp exact"), "{t}");
}

#[test]
fn old_user_signal_file_is_exact() {
    let old: UserSignal = serde_json::from_str(r#"{"name":"a","cob_id":419360000,"extended":true,"start_bit":0,"bits":8,"kind":"unsigned"}"#).unwrap();
    assert_eq!((old.match_mode, old.source), (MatchMode::Exact, None));
    assert!(!old.by_pgn());
    // The new fields round-trip.
    let mut s = old.clone();
    s.match_mode = MatchMode::J1939Pgn;
    s.source = Some(7);
    let back: UserSignal = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!((back.match_mode, back.source), (MatchMode::J1939Pgn, Some(7)));
}

#[test]
fn tracker_reassembles_a_bam_dm1() {
    let ctx = DecodeContext::default();
    let mut tr = J1939Tracker::new();
    let frames = [
        rx(0x1CEC_FF00, &[0x20, 10, 0, 2, 0xFF, 0xCA, 0xFE, 0], 0),
        rx(0x1CEB_FF00, &[1, 0x04, 0xFF, 100, 0, 1, 3, 0x6E], 50_000),
        rx(0x1CEB_FF00, &[2, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF], 100_000),
    ];
    let mut texts = Vec::new();
    for f in &frames {
        let mut d = decode(f, &ctx);
        tr.annotate(f, &ctx, &mut d);
        texts.push(d.text);
    }
    assert!(texts[0].contains("BAM PGN 65226 DM1, 10 bytes, 2 packets"), "{}", texts[0]);
    assert!(texts[1].contains("TP.DT 1/2"), "{}", texts[1]);
    assert!(texts[2].contains("complete: PGN 65226 DM1, 10 bytes") && texts[2].contains("SPN 100 FMI 1 OC 3, SPN 110 FMI 0 OC 1; lamps: amber"), "{}", texts[2]);
    // J1939 off: nothing is tracked.
    let mut raw = DecodeContext::default();
    raw.set_j1939(false);
    let mut tr = J1939Tracker::new();
    let mut d = decode(&frames[0], &raw);
    tr.annotate(&frames[0], &raw, &mut d);
    assert!(!d.text.contains("BAM"), "{}", d.text);
}
