//! Trace views and the continuous CSV log (A4).

use canoxide::can::{CanFrame, Dir};
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
    t.set_filter(TraceFilter { id_min: 0x700, id_max: 0x77F, ..Default::default() });
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
    let mut log = TraceLogger::start(&p2, DisplayMode::Hex, Some(TraceFilter { id_min: 0x700, id_max: 0x7FF, ..Default::default() }), 0).unwrap();
    log.write(&e1, 0).unwrap();
    log.write(&e2, 0).unwrap();
    assert_eq!(log.stop().unwrap(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
