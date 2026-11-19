//! The per-identifier statistics of the Messages view.

use canoxide::can::CanFrame;
use canoxide::canopen::decode::{decode, DecodeContext, MsgClass};
use canoxide::msgstats::{tint_strength, Change, MsgKey, MsgStats, MAX_IDS, NOISY_MIN_FRAMES};

fn frame(id: u32, data: &[u8], t_us: u64) -> CanFrame {
    let mut f = CanFrame::new(id, data);
    f.timestamp_us = t_us;
    f
}

fn feed(s: &mut MsgStats, f: &CanFrame) {
    let d = decode(f, &DecodeContext::default());
    s.on_frame(f, &d);
}

fn key(id: u32) -> MsgKey {
    MsgKey { id, extended: false }
}

#[test]
fn one_row_per_id_with_count_data_and_type() {
    let mut s = MsgStats::new();
    feed(&mut s, &frame(0x181, &[1, 2], 1000));
    feed(&mut s, &frame(0x181, &[1, 3], 2000));
    feed(&mut s, &frame(0x701, &[0x05], 2500));
    assert_eq!(s.len(), 2);
    let e = s.get(key(0x181)).unwrap();
    assert_eq!((e.count, e.dlc, e.data[..2].to_vec(), e.last_us), (2, 2, vec![1, 3], 2000));
    assert_eq!((e.class, e.node, e.label.as_str()), (MsgClass::Tpdo, Some(1), "TPDO1"));
    let hb = s.get(key(0x701)).unwrap();
    assert_eq!((hb.class, hb.node), (MsgClass::Heartbeat, Some(1)));
    assert_eq!(s.latest_us(), 2500);
    // An extended frame with the same number is its own row.
    let mut x = frame(0x181, &[0], 3000);
    x.extended = true;
    feed(&mut s, &x);
    assert_eq!(s.len(), 3);
}

#[test]
fn smoothed_period() {
    let mut s = MsgStats::new();
    for i in 0..50u64 {
        feed(&mut s, &frame(0x181, &[0], i * 10_000));
    }
    let p = s.get(key(0x181)).unwrap().period_us.unwrap();
    assert!((p - 10_000.0).abs() < 1.0, "{p}");
    // One late frame moves the average only in part.
    feed(&mut s, &frame(0x181, &[0], 49 * 10_000 + 60_000));
    let p = s.get(key(0x181)).unwrap().period_us.unwrap();
    assert!(p > 10_000.0 && p < 60_000.0 * 0.5, "{p}");
}

#[test]
fn byte_change_direction_rate_and_bit_toggles() {
    let mut s = MsgStats::new();
    feed(&mut s, &frame(0x200, &[0x10, 0x00, 0xFF], 0));
    feed(&mut s, &frame(0x200, &[0x12, 0x00, 0xF0], 1000));
    let e = s.get(key(0x200)).unwrap();
    assert_eq!(e.bytes[0].dir, Some(Change::Up));
    assert_eq!(e.bytes[0].last_change_us, Some(1000));
    assert_eq!(e.bytes[1].dir, None, "byte 1 did not change");
    assert_eq!(e.bytes[2].dir, Some(Change::Down));
    assert_eq!(e.compared, 1);
    // 0x10 -> 0x12: bit 1 toggled; 0xFF -> 0xF0: bits 16..19.
    assert_eq!(e.bit_toggles[1], 1);
    assert_eq!(e.bit_toggles[0], 0);
    assert_eq!((16..20).map(|k| e.bit_toggles[k]).sum::<u32>(), 4);
    assert_eq!(e.toggle_rate(1), 1.0);
    assert!(e.bit(1) && e.bit(4) && !e.bit(0));
    feed(&mut s, &frame(0x200, &[0x02, 0x00, 0xF0], 2000));
    let e = s.get(key(0x200)).unwrap();
    assert_eq!(e.bytes[0].dir, Some(Change::Down));
    assert_eq!(e.change_rate(0), 1.0);
    assert_eq!(e.change_rate(2), 0.5);
    assert_eq!(e.bytes[2].last_change_us, Some(1000), "unchanged byte keeps its change time");
}

#[test]
fn noisy_byte_after_enough_frames() {
    let mut s = MsgStats::new();
    for i in 0..(NOISY_MIN_FRAMES + 1) {
        feed(&mut s, &frame(0x190, &[i as u8, 7], i * 1000));
    }
    let e = s.get(key(0x190)).unwrap();
    assert!(e.is_noisy(0), "a counter is noisy");
    assert!(!e.is_noisy(1), "a constant byte is not");
    // Too few frames: not yet judged.
    let mut s = MsgStats::new();
    for i in 0..3 {
        feed(&mut s, &frame(0x190, &[i as u8], i * 1000));
    }
    assert!(!s.get(key(0x190)).unwrap().is_noisy(0));
}

#[test]
fn dlc_change_compares_only_the_common_bytes() {
    let mut s = MsgStats::new();
    feed(&mut s, &frame(0x300, &[1, 2, 3, 4], 0));
    feed(&mut s, &frame(0x300, &[1, 9], 1000));
    let e = s.get(key(0x300)).unwrap();
    assert_eq!(e.dlc, 2);
    assert_eq!(e.data, [1, 9, 0, 0, 0, 0, 0, 0], "bytes past the DLC are zero");
    assert_eq!(e.bytes[2].changes, 0);
    assert_eq!(e.bytes[1].dir, Some(Change::Up));
}

#[test]
fn reset_row_and_all() {
    let mut s = MsgStats::new();
    feed(&mut s, &frame(0x181, &[1], 0));
    feed(&mut s, &frame(0x181, &[2], 1000));
    feed(&mut s, &frame(0x182, &[2], 1000));
    let g = s.generation();
    s.reset_id(key(0x181));
    assert!(s.generation() != g);
    let e = s.get(key(0x181)).unwrap();
    assert_eq!((e.count, e.compared, e.bytes[0].changes, e.bit_toggles.iter().sum::<u32>(), e.period_us), (0, 0, 0, 0, None));
    assert_eq!(e.data[0], 2, "the newest data stays");
    assert_eq!(s.get(key(0x182)).unwrap().count, 1, "other rows keep their count");
    // The next frame counts again, without a period from before the reset.
    feed(&mut s, &frame(0x181, &[3], 5000));
    let e = s.get(key(0x181)).unwrap();
    assert_eq!((e.count, e.compared, e.period_us), (1, 0, None));
    s.reset();
    assert!(s.is_empty());
    assert_eq!(s.latest_us(), 0);
}

#[test]
fn bounded_memory() {
    let mut s = MsgStats::new();
    for id in 0..(MAX_IDS as u32 + 10) {
        let mut f = frame(id, &[0], 0);
        f.extended = true;
        feed(&mut s, &f);
    }
    assert_eq!(s.len(), MAX_IDS);
    assert_eq!(s.ignored(), 10);
    // A known identifier still counts.
    let mut f = frame(0, &[1], 10);
    f.extended = true;
    feed(&mut s, &f);
    assert_eq!(s.get(MsgKey { id: 0, extended: true }).unwrap().count, 2);
}

#[test]
fn filter_matches_id_type_and_node() {
    let mut s = MsgStats::new();
    feed(&mut s, &frame(0x181, &[1], 0));
    let e = s.get(key(0x181)).unwrap();
    for f in ["", "181", "0x181", "tpdo", "tpdo1", "node 1"] {
        assert!(e.matches(f), "{f}");
    }
    assert!(!e.matches("sdo"));
}

#[test]
fn tint_fades_over_the_fade_time() {
    assert_eq!(tint_strength(None, 5, 1000), 0.0);
    assert_eq!(tint_strength(Some(100), 100, 1000), 1.0);
    assert!((tint_strength(Some(100), 600, 1000) - 0.5).abs() < 1e-6);
    assert_eq!(tint_strength(Some(100), 1100, 1000), 0.0);
    assert_eq!(tint_strength(Some(100), 5000, 1000), 0.0);
    assert_eq!(tint_strength(Some(100), 50, 1000), 1.0, "a clock behind the frame: full");
    assert_eq!(tint_strength(Some(100), 100, 0), 0.0);
}
