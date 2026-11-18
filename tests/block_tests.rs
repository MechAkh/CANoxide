//! SDO block transfer (A1) against the simulator.

use canopen_tester::can::sim::{large_object_pattern, SimNode, LARGE_OBJECT, LARGE_OBJECT_SIZE};
use canopen_tester::can::CanFrame;
use canopen_tester::canopen::decode::{DecodeContext, SdoBlockTracker};
use canopen_tester::canopen::sdo::*;
use canopen_tester::session::{download_from_file, upload_to_file, Ctl, Session};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;

const NODE: u8 = 5;

/// Run one transfer; `filter(frame, to_server)` may change a frame or drop
/// it (false). Time moves 1 ms per frame and 10 ms when nothing is in flight.
fn run(
    c: &mut SdoClient,
    n: &mut SimNode,
    req: SdoRequest,
    mode: TransferMode,
    mut filter: impl FnMut(&mut CanFrame, bool) -> bool,
    log: &mut Vec<CanFrame>,
) -> SdoOutcome {
    let mut now = 0u64;
    let first = c.start_opts(NODE, req, now, StartOptions::mode(mode)).expect("start");
    let mut to_server: VecDeque<CanFrame> = VecDeque::from([first]);
    let mut to_client: VecDeque<CanFrame> = VecDeque::new();
    for _ in 0..200_000 {
        if let Some(mut f) = to_server.pop_front() {
            if filter(&mut f, true) {
                log.push(f);
                let mut out = Vec::new();
                n.handle_frame(&f, &mut out);
                to_client.extend(out);
            }
            continue;
        }
        now += 1;
        let out = if let Some(mut f) = to_client.pop_front() {
            if !filter(&mut f, false) {
                continue;
            }
            log.push(f);
            c.on_frame(&f, now)
        } else {
            now += 10;
            c.poll(now)
        };
        to_server.extend(out.frames().copied());
        if let Some(d) = out.done {
            // Deliver what the client still sent (abort, end response).
            while let Some(f) = to_server.pop_front() {
                log.push(f);
                let mut o = Vec::new();
                n.handle_frame(&f, &mut o);
            }
            return d;
        }
    }
    panic!("transfer did not end");
}

fn keep(_: &mut CanFrame, _: bool) -> bool {
    true
}

fn up(c: &mut SdoClient, n: &mut SimNode, index: u16, sub: u8, mode: TransferMode, log: &mut Vec<CanFrame>) -> Result<Vec<u8>, SdoError> {
    run(c, n, SdoRequest::Upload { index, sub }, mode, keep, log).result
}

fn down(c: &mut SdoClient, n: &mut SimNode, index: u16, sub: u8, data: Vec<u8>, mode: TransferMode, log: &mut Vec<CanFrame>) -> Result<Vec<u8>, SdoError> {
    run(c, n, SdoRequest::Download { index, sub, data }, mode, keep, log).result
}

fn new_data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + 3) as u8).collect()
}

#[test]
fn crc16_ccitt_known_value() {
    // CRC-16/XMODEM check value of "123456789".
    assert_eq!(crc16_ccitt(b"123456789"), 0x31C3);
    assert_eq!(crc16_ccitt(&[]), 0);
}

#[test]
fn block_upload_large_object_with_and_without_crc() {
    for (client_crc, server_crc) in [(true, true), (false, true), (true, false), (false, false)] {
        let mut c = SdoClient::default();
        c.block_crc = client_crc;
        let mut n = SimNode::builtin(NODE);
        n.block_crc = server_crc;
        let mut log = Vec::new();
        let data = up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Block, &mut log).unwrap();
        assert_eq!(data.len(), LARGE_OBJECT_SIZE);
        assert_eq!(data, large_object_pattern(), "crc {client_crc}/{server_crc}");
        // Block frames were used: initiate with ccs 5.
        assert_eq!(log[0].data[0] >> 5, 5);
        let crc_flag = log[1].data[0] & 0x04 != 0;
        assert_eq!(crc_flag, client_crc && server_crc);
        assert!(!c.is_busy());
    }
}

#[test]
fn block_download_large_object_with_and_without_crc() {
    for (client_crc, server_crc) in [(true, true), (false, true), (true, false)] {
        let mut c = SdoClient::default();
        c.block_crc = client_crc;
        let mut n = SimNode::builtin(NODE);
        n.block_crc = server_crc;
        let data = new_data(LARGE_OBJECT_SIZE + 3);
        let mut log = Vec::new();
        down(&mut c, &mut n, LARGE_OBJECT, 0, data.clone(), TransferMode::Block, &mut log).unwrap();
        assert_eq!(n.value(LARGE_OBJECT, 0).unwrap(), data.as_slice());
        assert_eq!(log[0].data[0] >> 5, 6, "block download initiate");
        let mut log = Vec::new();
        assert_eq!(up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Block, &mut log).unwrap(), data);
    }
}

#[test]
fn block_download_small_sizes_and_n_unused() {
    // 1..15 bytes cover every "n bytes unused" value and a single segment.
    for len in 1..=15usize {
        let mut c = SdoClient::default();
        let mut n = SimNode::builtin(NODE);
        let data = new_data(len);
        let mut log = Vec::new();
        down(&mut c, &mut n, LARGE_OBJECT, 0, data.clone(), TransferMode::Block, &mut log).unwrap();
        assert_eq!(n.value(LARGE_OBJECT, 0).unwrap(), data.as_slice(), "len {len}");
        let mut log = Vec::new();
        assert_eq!(up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Block, &mut log).unwrap(), data, "len {len}");
    }
}

#[test]
fn block_size_negotiation() {
    let mut c = SdoClient::default();
    c.block_size = 10;
    let mut n = SimNode::builtin(NODE);
    n.block_size = 4;
    // Upload: the server sends sub-blocks of the client's size (10).
    let mut log = Vec::new();
    assert_eq!(up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Block, &mut log).unwrap(), large_object_pattern());
    let max_up_seq = log.iter().filter(|f| f.id == 0x580 + NODE as u32 && log_is_segment(f)).map(|f| f.data[0] & 0x7F).max();
    assert_eq!(max_up_seq, Some(10));
    // Download: the client sends sub-blocks of the server's size (4).
    let data = new_data(200);
    let mut log = Vec::new();
    down(&mut c, &mut n, LARGE_OBJECT, 0, data.clone(), TransferMode::Block, &mut log).unwrap();
    assert_eq!(n.value(LARGE_OBJECT, 0).unwrap(), data.as_slice());
    let acks = log.iter().filter(|f| f.id == 0x580 + NODE as u32 && f.data[0] == 0xA2).count();
    assert_eq!(acks, 200usize.div_ceil(7).div_ceil(4));
}

/// A server frame of a block upload that is a segment (not init/end/abort).
fn log_is_segment(f: &CanFrame) -> bool {
    let cs = f.data[0] >> 5;
    !(cs == 6 && f.data[0] & 0x60 == 0x40 || f.data[0] == 0x80)
}

#[test]
fn block_download_missing_segment_is_sent_again() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let data = new_data(1000);
    let mut dropped = false;
    let mut seq3 = 0;
    let mut log = Vec::new();
    let o = run(
        &mut c,
        &mut n,
        SdoRequest::Download { index: LARGE_OBJECT, sub: 0, data: data.clone() },
        TransferMode::Block,
        |f, to_server| {
            // After the initiate response, drop segment 3 of the first sub-block once.
            if to_server && f.id == 0x600 + NODE as u32 && f.data[0] & 0x7F == 3 && f.data[0] >> 5 != 6 {
                seq3 += 1;
                if !dropped {
                    dropped = true;
                    return false;
                }
            }
            true
        },
        &mut log,
    );
    o.result.unwrap();
    assert!(dropped);
    assert_eq!(n.value(LARGE_OBJECT, 0).unwrap(), data.as_slice());
    // The ack after the lost segment confirmed seq 2.
    assert!(log.iter().any(|f| f.data[0] == 0xA2 && f.data[1] == 2));
    assert!(seq3 >= 2);
}

#[test]
fn block_upload_missing_segment_is_sent_again() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let mut dropped = false;
    let mut log = Vec::new();
    let o = run(
        &mut c,
        &mut n,
        SdoRequest::Upload { index: LARGE_OBJECT, sub: 0 },
        TransferMode::Block,
        |f, to_server| {
            if !to_server && !dropped && f.data[0] == 5 {
                dropped = true;
                return false;
            }
            true
        },
        &mut log,
    );
    assert_eq!(o.result.unwrap(), large_object_pattern());
    assert!(dropped);
    // The client acked seq 4, then the server sent from there again.
    assert!(log.iter().any(|f| f.id == 0x600 + NODE as u32 && f.data[0] == 0xA2 && f.data[1] == 4));
}

#[test]
fn block_upload_corrupted_segment_fails_the_crc() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let mut done = false;
    let mut log = Vec::new();
    let o = run(
        &mut c,
        &mut n,
        SdoRequest::Upload { index: LARGE_OBJECT, sub: 0 },
        TransferMode::Block,
        |f, to_server| {
            if !to_server && !done && f.data[0] == 7 {
                done = true;
                f.data[3] ^= 0xFF;
            }
            true
        },
        &mut log,
    );
    assert_eq!(o.result, Err(SdoError::ClientAbort(ABORT_CRC)));
    let last = log.last().unwrap();
    assert_eq!(last.data[0], 0x80, "client abort sent");
    assert_eq!(u32::from_le_bytes([last.data[4], last.data[5], last.data[6], last.data[7]]), 0x0504_0004);
}

#[test]
fn block_download_corrupted_segment_fails_the_crc_on_the_server() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let before = n.value(LARGE_OBJECT, 0).unwrap().to_vec();
    let mut done = false;
    let mut log = Vec::new();
    let o = run(
        &mut c,
        &mut n,
        SdoRequest::Download { index: LARGE_OBJECT, sub: 0, data: new_data(500) },
        TransferMode::Block,
        |f, to_server| {
            if to_server && !done && f.data[0] == 9 {
                done = true;
                f.data[2] ^= 0x55;
            }
            true
        },
        &mut log,
    );
    assert_eq!(o.result, Err(SdoError::ServerAbort(0x0504_0004)));
    assert_eq!(n.value(LARGE_OBJECT, 0).unwrap(), before.as_slice(), "nothing written");
}

#[test]
fn block_upload_protocol_switch_to_expedited_and_segmented() {
    let mut c = SdoClient::default();
    c.pst = 50;
    let mut n = SimNode::builtin(NODE);
    // 0x1018:1 is 4 bytes: the server answers expedited.
    let mut log = Vec::new();
    assert_eq!(up(&mut c, &mut n, 0x1018, 1, TransferMode::Block, &mut log).unwrap(), vec![0x9B, 0x06, 0, 0]);
    assert_eq!(log[1].data[0] & 0xE0, 0x40, "expedited upload response");
    // A 20-byte object: segmented answer.
    let s = b"twenty bytes string!".to_vec();
    n.set_value(0x2000, 0, s.clone());
    let mut log = Vec::new();
    assert_eq!(up(&mut c, &mut n, 0x2000, 0, TransferMode::Block, &mut log).unwrap(), s);
    // The large object is bigger than pst: block.
    let mut log = Vec::new();
    assert_eq!(up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Block, &mut log).unwrap(), large_object_pattern());
    assert_eq!(log[1].data[0] & 0xE3, 0xC2);
}

#[test]
fn server_without_block_support_aborts() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    n.block_enabled = false;
    let mut log = Vec::new();
    assert_eq!(up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Block, &mut log), Err(SdoError::ServerAbort(ABORT_COMMAND)));
    let mut log = Vec::new();
    assert_eq!(down(&mut c, &mut n, LARGE_OBJECT, 0, vec![1, 2, 3], TransferMode::Block, &mut log), Err(SdoError::ServerAbort(ABORT_COMMAND)));
    // Segmented still works on that server.
    let mut log = Vec::new();
    assert_eq!(up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Segmented, &mut log).unwrap(), large_object_pattern());
}

#[test]
fn auto_never_uses_block() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let mut log = Vec::new();
    up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Auto, &mut log).unwrap();
    assert!(log.iter().all(|f| f.id != 0x600 + NODE as u32 || f.data[0] >> 5 != 5));
    let mut log = Vec::new();
    down(&mut c, &mut n, LARGE_OBJECT, 0, new_data(100), TransferMode::Auto, &mut log).unwrap();
    assert_eq!(log[0].data[0] >> 5, 1);
}

#[test]
fn segmented_mode_for_a_small_download() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let mut log = Vec::new();
    down(&mut c, &mut n, 0x1017, 0, 500u16.to_le_bytes().to_vec(), TransferMode::Segmented, &mut log).unwrap();
    assert_eq!(log[0].data[0] & 0x02, 0, "not expedited");
    assert_eq!(n.value(0x1017, 0).unwrap(), 500u16.to_le_bytes());
}

#[test]
fn block_transfer_progress_and_cancel() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let first = c.start_opts(NODE, SdoRequest::Upload { index: LARGE_OBJECT, sub: 0 }, 0, StartOptions::mode(TransferMode::Block)).unwrap();
    let mut resp = Vec::new();
    n.handle_frame(&first, &mut resp);
    let out = c.on_frame(&resp[0], 1);
    assert_eq!(c.progress(), Some(SdoProgress { done: 0, total: Some(LARGE_OBJECT_SIZE) }));
    // Start: the first sub-block arrives.
    let mut seg = Vec::new();
    n.handle_frame(out.tx.as_ref().unwrap(), &mut seg);
    for f in seg.iter().take(10) {
        let o = c.on_frame(f, 2);
        assert!(o.done.is_none());
    }
    assert_eq!(c.progress().unwrap().done, 70);
    let out = c.cancel();
    let abort = out.tx.unwrap();
    assert_eq!(abort.data[0], 0x80);
    assert_eq!(out.done.unwrap().result, Err(SdoError::Cancelled));
    assert!(!c.is_busy());
}

#[test]
fn block_timeout_aborts() {
    let mut c = SdoClient::new(100);
    let _ = c.start_opts(NODE, SdoRequest::Upload { index: LARGE_OBJECT, sub: 0 }, 0, StartOptions::mode(TransferMode::Block)).unwrap();
    assert!(c.poll(50).done.is_none());
    let out = c.poll(150);
    assert_eq!(out.done.unwrap().result, Err(SdoError::Timeout));
    assert_eq!(out.tx.unwrap().data[4..8], 0x0504_0000u32.to_le_bytes());
}

#[test]
fn session_block_transfer_with_progress_and_file() {
    let mut s = Session::sim(SimNode::builtin(NODE), false);
    let dir = std::env::temp_dir().join(format!("cot_block_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("large.bin");
    let mut steps = 0usize;
    let mut last = (0usize, None);
    let mut p = |d: usize, t: Option<usize>| {
        steps += 1;
        last = (d, t);
    };
    let n = upload_to_file(&mut s, NODE, LARGE_OBJECT, 0, StartOptions::mode(TransferMode::Block), &path, &mut Ctl { cancel: None, progress: Some(&mut p) }).unwrap();
    assert_eq!(n, LARGE_OBJECT_SIZE);
    assert!(steps > 10);
    assert_eq!(last.1, Some(LARGE_OBJECT_SIZE));
    assert_eq!(std::fs::read(&path).unwrap(), large_object_pattern());
    // Download a changed file back.
    let data = new_data(3000);
    std::fs::write(&path, &data).unwrap();
    download_from_file(&mut s, NODE, LARGE_OBJECT, 0, StartOptions::mode(TransferMode::Block), &path, &mut Ctl::default()).unwrap();
    assert_eq!(s.sim_node().unwrap().value(LARGE_OBJECT, 0).unwrap(), data.as_slice());
    // Cancel at once.
    let flag = AtomicBool::new(true);
    let o = s.sdo_ctl(NODE, SdoRequest::Upload { index: LARGE_OBJECT, sub: 0 }, StartOptions::mode(TransferMode::Block), &mut Ctl { cancel: Some(&flag), progress: None }).unwrap();
    assert_eq!(o.result, Err(SdoError::Cancelled));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn session_listen_only_refuses_sdo() {
    let mut s = Session::sim(SimNode::builtin(NODE), true);
    assert!(s.sdo(NODE, SdoRequest::Upload { index: 0x1000, sub: 0 }, StartOptions::default()).is_err());
    assert!(s.take_frames().is_empty(), "nothing sent");
}

#[test]
fn decoder_labels_block_frames() {
    let ctx = DecodeContext::default();
    let mut c = SdoClient::default();
    c.block_size = 5;
    let mut n = SimNode::builtin(NODE);
    let mut log = Vec::new();
    up(&mut c, &mut n, LARGE_OBJECT, 0, TransferMode::Block, &mut log).unwrap();
    let mut t = SdoBlockTracker::new();
    let texts: Vec<String> = log.iter().map(|f| t.decode(f, &ctx).text).collect();
    assert!(texts[0].starts_with("SDO block upload init 0x2100:00 blksize 5"), "{}", texts[0]);
    assert!(texts[1].starts_with("SDO block upload init resp 0x2100:00 size 4096 CRC"), "{}", texts[1]);
    assert_eq!(texts[2], "SDO block upload start");
    assert!(texts[3].starts_with("SDO block upload segment seq 1 "), "{}", texts[3]);
    assert!(texts[7].starts_with("SDO block upload segment seq 5 "), "{}", texts[7]);
    assert_eq!(texts[8], "SDO block upload ack seq 5 blksize 5");
    assert!(texts.iter().any(|s| s.contains("segment seq") && s.contains(" last ")));
    let end = &texts[texts.len() - 2];
    assert!(end.starts_with("SDO block upload end, "), "{end}");
    assert_eq!(texts[texts.len() - 1], "SDO block upload end resp");

    let mut log = Vec::new();
    down(&mut c, &mut n, LARGE_OBJECT, 0, new_data(30), TransferMode::Block, &mut log).unwrap();
    let mut t = SdoBlockTracker::new();
    let texts: Vec<String> = log.iter().map(|f| t.decode(f, &ctx).text).collect();
    assert!(texts[0].starts_with("SDO block download init 0x2100:00 size 30 CRC"), "{}", texts[0]);
    assert!(texts[1].starts_with("SDO block download init resp 0x2100:00 blksize 127 CRC"), "{}", texts[1]);
    assert!(texts[2].starts_with("SDO block download segment seq 1 "), "{}", texts[2]);
    assert!(texts[6].starts_with("SDO block download segment seq 5 last "), "{}", texts[6]);
    assert_eq!(texts[7], "SDO block download ack seq 5 blksize 127");
    assert!(texts[8].starts_with("SDO block download end, 5 bytes unused"), "{}", texts[8]);
    assert_eq!(texts[9], "SDO block download end resp OK");
}
