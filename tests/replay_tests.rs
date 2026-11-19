//! Log replay (0.3): the parsers, the replay clock, and a seek that gives
//! the same state as a play to the same point. All logs are made up.

use canoxide::can::{CanAdapter, CanFrame, Dir};
use canoxide::canopen::decode::{decode, DecodeContext};
use canoxide::canopen::types::DisplayMode;
use canoxide::msgstats::MsgStats;
use canoxide::replay::*;
use canoxide::trace::{TraceStore, CSV_HEADER};
use std::time::Duration;

fn frame(id: u32, data: &[u8], us: u64) -> CanFrame {
    let mut f = CanFrame::new(id, data);
    f.dir = Dir::Rx;
    f.timestamp_us = us;
    f
}

/// A made-up bus: heartbeats, two TPDOs with a counter, a SYNC, an SDO pair.
fn made_up_log(n: usize) -> Vec<CanFrame> {
    let mut v = Vec::with_capacity(n);
    let mut t = 0u64;
    for i in 0..n {
        t += 700 + (i as u64 % 5) * 130;
        let f = match i % 6 {
            0 => frame(0x701, &[0x05], t),
            1 => frame(0x181, &[(i & 0xFF) as u8, (i >> 8) as u8, 0x10, 0x00], t),
            2 => frame(0x281, &[0x34, 0x12, (i % 3) as u8, 0, 0, 0, 0, 0], t),
            3 => frame(0x080, &[], t),
            4 => {
                let mut f = frame(0x601, &[0x40, 0x17, 0x10, 0x00, 0, 0, 0, 0], t);
                f.dir = Dir::Tx;
                f
            }
            _ => frame(0x581, &[0x4B, 0x17, 0x10, 0x00, 0xE8, 0x03, 0, 0], t),
        };
        v.push(f);
    }
    v
}

fn csv_of(frames: &[CanFrame], mode: DisplayMode) -> String {
    let ctx = DecodeContext::default();
    let mut t = TraceStore::new(frames.len().max(1));
    for f in frames {
        t.push(*f, decode(f, &ctx));
    }
    let mut out = Vec::new();
    t.write_csv(&mut out, mode).unwrap();
    String::from_utf8(out).unwrap()
}

fn same_frames(a: &[CanFrame], b: &[CanFrame]) {
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!((x.id, x.extended, x.rtr, x.dlc, x.payload(), x.dir, x.timestamp_us), (y.id, y.extended, y.rtr, y.dlc, y.payload(), y.dir, y.timestamp_us), "frame {i}");
    }
}

// ------------------------------------------------------------- parsers

#[test]
fn csv_written_by_the_app_round_trips() {
    let mut frames = made_up_log(60);
    // An extended frame and a remote frame too.
    let mut e = CanFrame::new(0x18FF_1234, &[1, 2, 3]);
    e.extended = true;
    e.timestamp_us = frames.last().unwrap().timestamp_us + 10;
    frames.push(e);
    let mut r = CanFrame::new(0x701, &[]);
    r.rtr = true;
    r.dlc = 1;
    r.timestamp_us = e.timestamp_us + 10;
    frames.push(r);
    for mode in [DisplayMode::Hex, DisplayMode::Decimal] {
        let text = csv_of(&frames, mode);
        assert!(text.starts_with(CSV_HEADER));
        let log = parse_log(&text, None).unwrap();
        assert_eq!(log.format, LogFormat::CanoxideCsv);
        assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
        // The first frame is at 0; the distances are kept.
        let base = frames[0].timestamp_us;
        let want: Vec<CanFrame> = frames.iter().map(|f| CanFrame { timestamp_us: f.timestamp_us - base, ..*f }).collect();
        same_frames(&log.frames, &want);
    }
}

#[test]
fn csv_of_version_0_2_0_loads() {
    // The header and row form of 0.2.0 (the same columns as now).
    let text = "time_s,delta_ms,dir,id,ext,rtr,dlc,data,type,node,text\n\
                12.000100,,Rx,701,0,0,1,05,HB,1,pre-operational\n\
                12.010100,10.000,Rx,181,0,0,2,12 34,TPDO1,1,\"a, quoted\nmulti-line text\"\n\
                12.020100,10.000,Tx,000,0,0,2,01 00,NMT,,start all\n";
    let log = parse_log(text, None).unwrap();
    assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
    assert_eq!(log.frames.len(), 3);
    assert_eq!(log.frames[0].timestamp_us, 0);
    assert_eq!(log.frames[1].timestamp_us, 10_000);
    assert_eq!(log.frames[1].payload(), &[0x12, 0x34]);
    assert_eq!(log.frames[2].dir, Dir::Tx);
}

#[test]
fn candump_formats() {
    let text = "(1700000000.123456) can0 123#DEADBEEF\n\
                (1700000000.223456) can0 12345678#01.02.03\n\
                (1700000000.323456) can0 701#R\n\
                (1700000000.423456) can0 702#R1\n\
                \n\
                (1700000000.523456) vcan1 080#\n";
    let log = parse_log(text, None).unwrap();
    assert_eq!(log.format, LogFormat::Candump);
    assert_eq!(log.bad_lines, 0, "{:?}", log.errors);
    let f = &log.frames;
    assert_eq!(f.len(), 5);
    assert_eq!((f[0].id, f[0].extended, f[0].payload()), (0x123, false, &[0xDE, 0xAD, 0xBE, 0xEF][..]));
    assert_eq!((f[1].id, f[1].extended, f[1].payload()), (0x1234_5678, true, &[1, 2, 3][..]));
    assert!(f[2].rtr && f[2].dlc == 0);
    assert!(f[3].rtr && f[3].dlc == 1);
    assert_eq!(f[4].dlc, 0);
    assert_eq!(f.iter().map(|x| x.timestamp_us).collect::<Vec<_>>(), vec![0, 100_000, 200_000, 300_000, 400_000]);
    // The test writer gives lines that read back the same.
    let again: String = log.frames.iter().map(|x| candump_line(x, 1_700_000_000) + "\n").collect();
    same_frames(&parse_log(&again, None).unwrap().frames, &log.frames);
}

#[test]
fn bad_lines_are_counted_with_their_line_number() {
    let text = "(1.000000) can0 123#00\n\
                (1.100000) can0 12G#00\n\
                garbage\n\
                (1.200000) can0 123#001122334455667788\n\
                (1.300000) can0 123##100\n\
                (1.400000) can0 123#01\n";
    let log = parse_log(text, None).unwrap();
    assert_eq!(log.frames.len(), 2);
    assert_eq!(log.bad_lines, 4);
    assert_eq!(log.errors.iter().map(|e| e.line).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
    assert!(log.errors[0].to_string().starts_with("line 2: "), "{}", log.errors[0]);

    let csv = format!("{CSV_HEADER}\n1.0,,Rx,181,0,0,2,12 34,,,\n1.1,,Rx,181,0,0,3,12 34,,,\n1.2,,Rx,xyz,0,0,1,00,,,\n1.3,,Rx,182,0,0,1,07,,,\n");
    let log = parse_log(&csv, None).unwrap();
    assert_eq!(log.frames.len(), 2);
    assert_eq!(log.errors.iter().map(|e| e.line).collect::<Vec<_>>(), vec![3, 4]);

    // No frame at all, or an unknown format: an error.
    assert!(parse_log("garbage\nmore garbage\n", None).is_err());
    assert!(parse_log(&format!("{CSV_HEADER}\n"), None).is_err());
    // Data saved in String mode cannot be read back.
    let s = format!("{CSV_HEADER}\n1.0,,Rx,181,0,0,4,ABCD,,,\n");
    assert!(parse_log(&s, None).unwrap_err().contains("String"));
}

#[test]
fn time_that_goes_back_is_raised() {
    let text = "(10.000000) can0 100#01\n(10.500000) can0 100#02\n(10.200000) can0 100#03\n(11.000000) can0 100#04\n";
    let log = parse_log(text, None).unwrap();
    assert_eq!(log.frames.iter().map(|f| f.timestamp_us).collect::<Vec<_>>(), vec![0, 500_000, 500_000, 1_000_000]);
}

#[test]
fn load_file_with_progress() {
    let dir = std::env::temp_dir().join(format!("canoxide_replay_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("log.csv");
    std::fs::write(&p, csv_of(&made_up_log(500), DisplayMode::Hex)).unwrap();
    let prog = std::sync::atomic::AtomicUsize::new(0);
    let log = load_log(&p, Some(&prog)).unwrap();
    assert_eq!(log.frames.len(), 500);
    assert!(prog.load(std::sync::atomic::Ordering::Relaxed) > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// --------------------------------------------------------------- player

#[test]
fn player_paces_by_the_log_time() {
    let frames: Vec<CanFrame> = (0..10).map(|i| frame(0x181, &[i as u8], i * 100_000)).collect();
    let mut p = ReplayPlayer::new(&frames);
    assert_eq!(p.duration_us(), 900_000);
    assert!(!p.play(1_000_000));
    // Frame 0 (t = 0) is due at once.
    assert_eq!(p.advance(&frames, 1_000_000), Tick::Frames(0..1));
    // 250 ms later at 1x: frames 1 and 2.
    assert_eq!(p.advance(&frames, 1_250_000), Tick::Frames(1..3));
    assert_eq!(p.pos_us(), 250_000);
    // 4x: 100 ms of wall time = 400 ms of log time.
    p.set_speed(Speed::Factor(4.0), 1_250_000);
    assert_eq!(p.advance(&frames, 1_350_000), Tick::Frames(3..7));
    assert_eq!(p.pos_us(), 650_000);
    // Pause: nothing more.
    p.pause();
    assert_eq!(p.advance(&frames, 9_000_000), Tick::Frames(7..7));
    // Step: one frame, stays paused.
    assert_eq!(p.step(&frames), Some(7));
    assert_eq!(p.pos_us(), 700_000);
    assert!(!p.playing());
    // Max: everything that is left, then it stops at the end.
    p.set_speed(Speed::Max, 0);
    p.play(0);
    assert_eq!(p.advance(&frames, 0), Tick::Frames(8..10));
    assert!(p.at_end() && !p.playing());
    assert_eq!(p.pos_us(), 900_000);
    // Play at the end starts again.
    assert!(p.play(0));
    assert_eq!(p.cursor(), 0);
    // Loop: the end gives Restart.
    p.set_looping(true);
    assert_eq!(p.advance(&frames, 0), Tick::Frames(0..10));
    assert_eq!(p.advance(&frames, 0), Tick::Restart);
    assert_eq!(p.cursor(), 0);
    // Seek: all frames with t <= pos are behind the cursor.
    assert_eq!(p.seek(&frames, 450_000, 0), 5);
    assert_eq!(p.seek(&frames, 500_000, 0), 6);
    assert_eq!(p.seek(&frames, 99_999_999, 0), 10);
}

#[test]
fn max_speed_gives_bounded_batches() {
    let frames: Vec<CanFrame> = (0..(MAX_BATCH as u64 * 2 + 5)).map(|i| frame(0x181, &[0], i)).collect();
    let mut p = ReplayPlayer::new(&frames);
    p.set_speed(Speed::Max, 0);
    p.play(0);
    let mut n = 0;
    while let Tick::Frames(r) = p.advance(&frames, 0) {
        if r.is_empty() {
            break;
        }
        assert!(r.len() <= MAX_BATCH);
        n += r.len();
    }
    assert_eq!(n, frames.len());
}

fn same_stats(a: &MsgStats, b: &MsgStats) {
    assert_eq!(a.len(), b.len());
    for e in a.entries() {
        let o = b.get(e.key).expect("same keys");
        assert_eq!((e.count, e.data, e.dlc, e.last_us, e.compared, e.bit_toggles), (o.count, o.data, o.dlc, o.last_us, o.compared, o.bit_toggles), "{:?}", e.key);
        assert_eq!(e.period_us.map(|p| p.to_bits()), o.period_us.map(|p| p.to_bits()));
    }
}

#[test]
fn seek_gives_the_same_state_as_play() {
    let frames = made_up_log(5000);
    let ctx = DecodeContext::default();
    let pos = frames[3210].timestamp_us;
    // Play at 16x from the start to `pos`, feeding frame by frame like the
    // worker does.
    let mut p = ReplayPlayer::new(&frames);
    p.set_speed(Speed::Factor(16.0), 0);
    p.play(0);
    let mut msgs = MsgStats::new();
    let mut load = canoxide::busload::BusLoad::new(250);
    let mut fed = 0;
    // The wall time at which the log reaches `pos` (frames are at least
    // 700 µs apart, so the rounding up feeds no later frame).
    let wall_end = pos.div_ceil(16);
    let mut wall = 0;
    while wall < wall_end {
        wall = (wall + 1000).min(wall_end);
        let Tick::Frames(r) = p.advance(&frames, wall) else { panic!("no loop") };
        for f in &frames[r] {
            msgs.on_frame(f, &decode(f, &ctx));
            load.add(f);
            fed += 1;
        }
    }
    load.advance(pos);
    assert_eq!(fed, 3211);
    // Seek to `pos` and rebuild in chunks.
    let mut q = ReplayPlayer::new(&frames);
    let target = q.seek(&frames, pos, 0);
    assert_eq!(target, 3211);
    let mut rb = Rebuild::new(target, pos, 100, 250);
    let mut rounds = 0;
    while !rb.step(&frames, &ctx, None, 700) {
        rounds += 1;
    }
    assert!(rounds >= 4);
    same_stats(&rb.msgs, &msgs);
    assert_eq!(format!("{:?}", rb.load.stats()), format!("{:?}", load.stats()));
    // The trace tail holds the newest frames up to the cursor.
    assert_eq!(rb.tail.len(), 100);
    assert_eq!(rb.tail.back().unwrap().0.timestamp_us, pos);
    // Node states only from received frames.
    assert_eq!(rb.nodes.nodes.get(&1).map(|e| e.last_seen_us), frames[..3211].iter().rev().find(|f| f.id == 0x701).map(|f| f.timestamp_us));
}

// ------------------------------------------------------- file adapter

#[test]
fn file_adapter_never_sends_and_ends() {
    let frames = made_up_log(30);
    let mut a = FileAdapter::new("replay test", frames.clone(), None);
    assert!(a.keeps_timestamps());
    assert!(a.send(&frames[0]).is_err());
    let mut got = Vec::new();
    while let Some(f) = a.receive(Duration::from_millis(1)).unwrap() {
        got.push(f);
    }
    assert!(a.finished());
    same_frames(&got, &frames);
    // Paced at 1000x: 30 frames over ~0.03 s of log take a few ms.
    let mut b = FileAdapter::new("replay test", frames.clone(), Some(1000.0));
    let t0 = std::time::Instant::now();
    let mut n = 0;
    while !b.finished() {
        if b.receive(Duration::from_millis(1)).unwrap().is_some() {
            n += 1;
        }
        assert!(t0.elapsed() < Duration::from_secs(5));
    }
    assert_eq!(n, 30);
}

#[test]
fn a_million_frames_load_and_seek() {
    // 1M made-up candump lines (about 30 MB of text).
    let n = 1_000_000u64;
    let mut s = String::with_capacity(32 * n as usize);
    for i in 0..n {
        let us = i * 500;
        s.push_str(&format!("({}.{:06}) can0 {:03X}#{:02X}{:02X}\n", us / 1_000_000, us % 1_000_000, 0x181 + (i % 4), i & 0xFF, (i >> 8) & 0xFF));
    }
    let prog = std::sync::atomic::AtomicUsize::new(0);
    let t0 = std::time::Instant::now();
    let log = parse_log(&s, Some(&prog)).unwrap();
    let parse = t0.elapsed();
    assert_eq!(log.frames.len(), n as usize);
    assert_eq!(log.bad_lines, 0);
    assert!(prog.load(std::sync::atomic::Ordering::Relaxed) > 0);
    // Seek to the middle and rebuild in the chunks the worker uses.
    let mut p = ReplayPlayer::new(&log.frames);
    let target = p.seek(&log.frames, 250_000_000, 0);
    assert_eq!(target, 500_001);
    let t1 = std::time::Instant::now();
    let mut rb = Rebuild::new(target, 250_000_000, 1000, 250);
    let ctx = DecodeContext::default();
    let mut chunks = 0;
    while !rb.step(&log.frames, &ctx, None, 20_000) {
        chunks += 1;
    }
    assert_eq!(chunks, 25);
    assert_eq!(rb.msgs.len(), 4);
    assert_eq!(rb.msgs.entries().map(|e| e.count).sum::<u64>(), 500_001);
    eprintln!("1M frames: parse {parse:?}, rebuild of 500k {:?}", t1.elapsed());
}