//! Trace views and the continuous CSV log (A4).

use canoxide::can::{CanFrame, CanId, Dir};
use canoxide::canopen::decode::{decode, DecodeContext};
use canoxide::canopen::types::DisplayMode;
use canoxide::trace::*;

fn push(t: &mut TraceStore, id: u32, data: &[u8], us: u64) -> TraceEntry {
    let mut f = CanFrame::new(id, data);
    f.dir = Dir::Rx;
    f.timestamp_us = us;
    let d = decode(&f, &DecodeContext::default());
    t.push(f, d.clone());
    TraceEntry::new(t.total() - 1, f, d)
}

#[test]
fn several_views_share_one_buffer() {
    let mut t = TraceStore::new(100);
    let v1 = t.add_view();
    let v2 = t.add_view();
    assert_eq!(t.view_ids(), vec![0, v1, v2]);
    t.set_view_filter(v1, TraceFilter { node: Some(5), ..Default::default() });
    push(&mut t, 0x185, &[1], 0);
    push(&mut t, 0x186, &[1], 1);
    push(&mut t, 0x705, &[5], 2);
    assert_eq!((t.view_len_of(0), t.view_len_of(v1), t.view_len_of(v2)), (3, 2, 3));
    assert_eq!(t.view_row_of(v1, 1).unwrap().frame.id, 0x705);
    // Pause v2: it keeps 3 rows while the others grow.
    t.set_view_paused(v2, true);
    assert!(t.view_paused(v2));
    push(&mut t, 0x185, &[2], 3);
    assert_eq!((t.view_len(), t.view_len_of(v1), t.view_len_of(v2)), (4, 3, 3));
    // Resume catches up.
    t.set_view_paused(v2, false);
    assert_eq!(t.view_len_of(v2), 4);
    // Fixed rows per view filter.
    assert_eq!(t.fixed_rows_of(v1).len(), 2);
    assert_eq!(t.fixed_rows().len(), 3);
    // A filter on view 0 does not change the others.
    t.set_filter(TraceFilter { id_min: CanId::std(0x700), id_max: CanId::std(0x77F), ..Default::default() });
    assert_eq!((t.view_len(), t.view_len_of(v1)), (1, 3));
    assert_eq!(t.view_filter(v1).node, Some(5));
    // Ring trim drops rows from every view.
    t.set_capacity(2);
    assert_eq!((t.view_len(), t.view_len_of(v1), t.view_len_of(v2)), (1, 2, 2));
    let mut csv = Vec::new();
    t.write_csv_of(v1, &mut csv, DisplayMode::Hex).unwrap();
    assert_eq!(String::from_utf8(csv).unwrap().lines().count(), 3);
    // Remove: id 0 is fixed, a freed id is reused.
    assert!(!t.remove_view(0));
    assert!(t.remove_view(v1));
    assert!(!t.remove_view(v1));
    assert_eq!(t.view_len_of(v1), 0);
    assert_eq!(t.add_view(), v1);
}

#[test]
fn continuous_log_file_content() {
    let dir = std::env::temp_dir().join(format!("canoxide_trace_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("log.csv");
    let mut t = TraceStore::new(2);
    let mut log = TraceLogger::start(&path, DisplayMode::Hex, None, 0).unwrap();
    // Header is on disk at once.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{CSV_HEADER}\n"));
    let e1 = push(&mut t, 0x185, &[0x12, 0x34], 1_000_000);
    log.write(&e1, 100).unwrap();
    // Not flushed yet (100 ms < 500 ms).
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    let e2 = push(&mut t, 0x701, &[0x05], 1_002_500);
    log.write(&e2, 600).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1], canoxide::trace::csv_row(&e1, None, DisplayMode::Hex));
    assert!(lines[1].starts_with("1.000000,,Rx,185,0,0,2,12 34,TPDO1,5,"), "{}", lines[1]);
    assert!(lines[2].starts_with("1.002500,2.500,Rx,701,0,0,1,05,"), "{}", lines[2]);
    // More frames than the ring holds: the log keeps all of them.
    for i in 0..10u64 {
        let e = push(&mut t, 0x185, &[i as u8], 2_000_000 + i);
        log.write(&e, 700).unwrap();
    }
    assert_eq!(t.len(), 2);
    assert_eq!(log.rows(), 12);
    assert_eq!(log.stop().unwrap(), 12);
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 13);
    // A filtered log.
    let p2 = dir.join("log2.csv");
    let mut log = TraceLogger::start(&p2, DisplayMode::Hex, Some(TraceFilter { id_min: CanId::std(0x700), id_max: CanId::std(0x7FF), ..Default::default() }), 0).unwrap();
    log.write(&e1, 0).unwrap();
    log.write(&e2, 0).unwrap();
    assert_eq!(log.stop().unwrap(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

fn push_ext(t: &mut TraceStore, id: u32, data: &[u8], us: u64) {
    let mut f = CanFrame::new(id, data);
    f.extended = true;
    f.dir = Dir::Rx;
    f.timestamp_us = us;
    // Settings > Extended IDs are: Raw frames.
    let mut ctx = DecodeContext::default();
    ctx.set_j1939(false);
    let d = decode(&f, &ctx);
    t.push(f, d);
}

/// J1939 (the default): the PGN and source address filters take J1939
/// frames only; the node filter does not take them.
#[test]
fn j1939_pgn_and_source_address_filters() {
    let mut t = TraceStore::new(100);
    push(&mut t, 0x701, &[5], 0);
    for (i, id) in [0x0CF0_0400u32, 0x18FE_EE00, 0x18FE_EE21, 0x18EA_0021].into_iter().enumerate() {
        let mut f = CanFrame::new(id, &[1, 2, 3]);
        f.extended = true;
        f.dir = Dir::Rx;
        f.timestamp_us = 10 + i as u64;
        let d = decode(&f, &DecodeContext::default());
        t.push(f, d);
    }
    let ids = |t: &TraceStore| (0..t.view_len()).map(|i| id_text(&t.view_row(i).unwrap().frame)).collect::<Vec<_>>();
    t.set_filter(TraceFilter { pgn: Some(65262), ..Default::default() });
    assert_eq!(ids(&t), vec!["18FEEE00", "18FEEE21"]);
    t.set_filter(TraceFilter { pgn: Some(65262), sa: Some(0x21), ..Default::default() });
    assert_eq!(ids(&t), vec!["18FEEE21"]);
    t.set_filter(TraceFilter { sa: Some(0x21), ..Default::default() });
    assert_eq!(ids(&t), vec!["18FEEE21", "18EA0021"]);
    // PDU1: the PGN has no destination byte.
    t.set_filter(TraceFilter { pgn: Some(59904), ..Default::default() });
    assert_eq!(ids(&t), vec!["18EA0021"]);
    // Node 0 is not the J1939 source address 0.
    t.set_filter(TraceFilter { node: Some(0), ..Default::default() });
    assert_eq!(t.view_len(), 0);
}

#[test]
fn standard_and_extended_with_same_number_stay_apart() {
    let mut t = TraceStore::new(100);
    push(&mut t, 0x123, &[1], 0);
    push_ext(&mut t, 0x123, &[2], 1);
    push(&mut t, 0x123, &[3], 2);
    push_ext(&mut t, 0x18FE_F100, &[4], 3);
    push_ext(&mut t, 0x18FE_F1FF, &[5], 4);
    push_ext(&mut t, 0x18FE_F200, &[6], 5);
    // Fixed mode: one row per pair, standard rows first.
    let rows = t.fixed_rows();
    let ids: Vec<String> = rows.iter().map(|r| id_text(&r.last.frame)).collect();
    assert_eq!(ids, vec!["123", "00000123", "18FEF100", "18FEF1FF", "18FEF200"]);
    assert_eq!(rows[0].count, 2);
    assert_eq!(rows[1].count, 1);
    // The Type column of an extended frame is the neutral class.
    assert_eq!(rows[1].last.decoded.label, "Ext");
    // Format filter.
    t.set_filter(TraceFilter { format: FrameFormat::Standard, ..Default::default() });
    assert_eq!(t.view_len(), 2);
    assert!((0..2).all(|i| !t.view_row(i).unwrap().frame.extended));
    t.set_filter(TraceFilter { format: FrameFormat::Extended, ..Default::default() });
    assert_eq!(t.view_len(), 4);
    assert_eq!(t.fixed_rows().len(), 4);
    // A range of extended IDs.
    t.set_filter(TraceFilter { id_min: CanId::ext(0x18FE_F100), id_max: CanId::ext(0x18FE_F1FF), ..Default::default() });
    assert_eq!(t.view_len(), 2);
    // A standard range does not hold the extended 123.
    t.set_filter(TraceFilter { id_min: CanId::std(0x123), id_max: CanId::std(0x123), ..Default::default() });
    assert_eq!(t.view_len(), 2);
    t.set_filter(TraceFilter { id_min: CanId::ext(0x123), id_max: CanId::ext(0x123), ..Default::default() });
    assert_eq!(t.view_len(), 1);
    assert_eq!(t.view_row(0).unwrap().frame.data[0], 2);
    // The text filter finds the 8-digit form.
    t.set_filter(TraceFilter { text: "00000123".into(), ..Default::default() });
    assert_eq!(t.view_len(), 1);
    assert!(TraceFilter::default().is_empty());
}

#[test]
fn view_index_of_seq_follows_dropped_rows() {
    let mut t = TraceStore::new(5);
    for i in 0..5u64 {
        push(&mut t, 0x181, &[i as u8], i);
    }
    // Row 2 has sequence number 2.
    assert_eq!(t.view_index_of_seq(0, 2), 2);
    // Two more frames drop the two oldest: the same frame is now row 0.
    push(&mut t, 0x181, &[5], 5);
    push(&mut t, 0x181, &[6], 6);
    assert_eq!(t.view_index_of_seq(0, 2), 0);
    assert_eq!(t.view_row_of(0, 0).unwrap().seq, 2);
    // A dropped sequence number gives the oldest row; a future one the end.
    assert_eq!(t.view_index_of_seq(0, 0), 0);
    assert_eq!(t.view_index_of_seq(0, 99), t.view_len());
}
