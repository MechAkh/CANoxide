use canoxide::can::sim::SimNode;
use canoxide::can::CanFrame;
use canoxide::canopen::nmt::{nmt_frame, sync_frame, NmtCommand, NodeState};
use canoxide::canopen::sdo::*;
use canoxide::eds;
use std::path::Path;
use std::sync::Arc;

const NODE: u8 = 5;

/// Run one transfer between the client and the simulated node.
fn transfer(client: &mut SdoClient, node: &mut SimNode, req: SdoRequest) -> SdoOutcome {
    let mut now = 0u64;
    let mut frame = client.start(NODE, req, now).unwrap();
    for _ in 0..10_000 {
        let mut resp = Vec::new();
        node.handle_frame(&frame, &mut resp);
        assert!(resp.len() <= 1, "one response per request");
        let Some(r) = resp.first() else { panic!("no answer from the node") };
        now += 1;
        let out = client.on_frame(r, now);
        if let Some(done) = out.done {
            return done;
        }
        frame = out.tx.expect("next request");
    }
    panic!("transfer did not end");
}

fn upload(c: &mut SdoClient, n: &mut SimNode, index: u16, sub: u8) -> Result<Vec<u8>, SdoError> {
    transfer(c, n, SdoRequest::Upload { index, sub }).result
}

fn download(c: &mut SdoClient, n: &mut SimNode, index: u16, sub: u8, data: Vec<u8>) -> Result<Vec<u8>, SdoError> {
    transfer(c, n, SdoRequest::Download { index, sub, data }).result
}

#[test]
fn expedited_upload_and_download() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    assert_eq!(upload(&mut c, &mut n, 0x1018, 1).unwrap(), vec![0x9B, 0x06, 0, 0]);
    assert_eq!(upload(&mut c, &mut n, 0x1001, 0).unwrap(), vec![0]);
    assert_eq!(upload(&mut c, &mut n, 0x1017, 0).unwrap(), 1000u16.to_le_bytes().to_vec());
    download(&mut c, &mut n, 0x2002, 0, 500i32.to_le_bytes().to_vec()).unwrap();
    assert_eq!(n.value(0x2002, 0).unwrap(), 500i32.to_le_bytes());
    download(&mut c, &mut n, 0x1017, 0, 250u16.to_le_bytes().to_vec()).unwrap();
    assert_eq!(upload(&mut c, &mut n, 0x1017, 0).unwrap(), 250u16.to_le_bytes().to_vec());
    download(&mut c, &mut n, 0x6200, 2, vec![0xA5]).unwrap();
    assert_eq!(upload(&mut c, &mut n, 0x6200, 2).unwrap(), vec![0xA5]);
    assert!(!c.is_busy());
}

#[test]
fn segmented_upload_and_download_of_100_byte_string() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    let s: Vec<u8> = (0..100).map(|i| b'A' + (i % 26) as u8).collect();
    download(&mut c, &mut n, 0x2000, 0, s.clone()).unwrap();
    assert_eq!(n.value(0x2000, 0).unwrap(), s.as_slice());
    assert_eq!(upload(&mut c, &mut n, 0x2000, 0).unwrap(), s);
    // Default string and an exact multiple of 7.
    let mut n2 = SimNode::builtin(NODE);
    assert_eq!(upload(&mut c, &mut n2, 0x1008, 0).unwrap(), b"CANoxide simulator".to_vec());
    let s14 = b"abcdefghijklmn".to_vec();
    download(&mut c, &mut n2, 0x2003, 0, s14.clone()).unwrap();
    assert_eq!(upload(&mut c, &mut n2, 0x2003, 0).unwrap(), s14);
    // Empty domain.
    download(&mut c, &mut n2, 0x2003, 0, Vec::new()).unwrap();
    assert_eq!(upload(&mut c, &mut n2, 0x2003, 0).unwrap(), Vec::<u8>::new());
}

#[test]
fn server_aborts() {
    let mut c = SdoClient::default();
    let mut n = SimNode::builtin(NODE);
    assert_eq!(upload(&mut c, &mut n, 0x5000, 0), Err(SdoError::ServerAbort(0x0602_0000)));
    assert_eq!(upload(&mut c, &mut n, 0x1018, 9), Err(SdoError::ServerAbort(0x0609_0011)));
    assert_eq!(download(&mut c, &mut n, 0x1000, 0, vec![1, 2, 3, 4]), Err(SdoError::ServerAbort(0x0601_0002)));
    assert_eq!(download(&mut c, &mut n, 0x1008, 0, b"a long new name".to_vec()), Err(SdoError::ServerAbort(0x0601_0002)));
    assert_eq!(upload(&mut c, &mut n, 0x2004, 0), Err(SdoError::ServerAbort(0x0601_0001)));
    assert_eq!(download(&mut c, &mut n, 0x2002, 0, 1001i32.to_le_bytes().to_vec()), Err(SdoError::ServerAbort(0x0609_0031)));
    assert_eq!(download(&mut c, &mut n, 0x2002, 0, (-1001i32).to_le_bytes().to_vec()), Err(SdoError::ServerAbort(0x0609_0032)));
    assert_eq!(download(&mut c, &mut n, 0x2002, 0, vec![1, 2]), Err(SdoError::ServerAbort(0x0607_0013)));
    let e = SdoError::ServerAbort(0x0602_0000).to_string();
    assert!(e.contains("Object does not exist"), "{e}");
    assert!(!c.is_busy());
}

#[test]
fn timeout_sends_abort() {
    let mut c = SdoClient::new(1000);
    let f = c.start(NODE, SdoRequest::Upload { index: 0x1000, sub: 0 }, 50).unwrap();
    assert_eq!(f.id, 0x605);
    assert_eq!(f.data[0], 0x40);
    assert_eq!(c.poll(1049), SdoOutput::default());
    let out = c.poll(1050);
    let done = out.done.unwrap();
    assert_eq!(done.result, Err(SdoError::Timeout));
    let abort = out.tx.unwrap();
    assert_eq!(abort.data, [0x80, 0x00, 0x10, 0x00, 0x00, 0x00, 0x04, 0x05]);
    // The scan variant sends no abort.
    c.start_with(NODE, SdoRequest::Upload { index: 0x1000, sub: 0 }, 0, 100, false).unwrap();
    let out = c.poll(100);
    assert!(out.tx.is_none());
    assert_eq!(out.done.unwrap().result, Err(SdoError::Timeout));
}

#[test]
fn busy_and_invalid_node() {
    let mut c = SdoClient::default();
    c.start(NODE, SdoRequest::Upload { index: 0x1000, sub: 0 }, 0).unwrap();
    assert_eq!(c.start(NODE, SdoRequest::Upload { index: 0x1000, sub: 0 }, 0), Err(SdoError::Busy));
    let mut c2 = SdoClient::default();
    assert_eq!(c2.start(0, SdoRequest::Upload { index: 0x1000, sub: 0 }, 0), Err(SdoError::InvalidNode));
    assert_eq!(c2.start(128, SdoRequest::Upload { index: 0x1000, sub: 0 }, 0), Err(SdoError::InvalidNode));
}

#[test]
fn toggle_error_in_upload_aborts() {
    let mut c = SdoClient::default();
    c.start(NODE, SdoRequest::Upload { index: 0x2000, sub: 0 }, 0).unwrap();
    let init = CanFrame::new(0x585, &[0x41, 0x00, 0x20, 0x00, 20, 0, 0, 0]);
    let out = c.on_frame(&init, 1);
    assert_eq!(out.tx.unwrap().data[0], 0x60);
    // First segment must have t=0; the server sends t=1.
    let bad = CanFrame::new(0x585, &[0x10, 1, 2, 3, 4, 5, 6, 7]);
    let out = c.on_frame(&bad, 2);
    assert_eq!(out.done.unwrap().result, Err(SdoError::ClientAbort(ABORT_TOGGLE)));
    assert_eq!(out.tx.unwrap().data, [0x80, 0x00, 0x20, 0x00, 0x00, 0x00, 0x03, 0x05]);
    assert!(!c.is_busy());
}

#[test]
fn toggle_error_in_download_aborts() {
    let mut c = SdoClient::default();
    c.start(NODE, SdoRequest::Download { index: 0x2000, sub: 0, data: vec![b'x'; 20] }, 0).unwrap();
    let out = c.on_frame(&CanFrame::new(0x585, &[0x60, 0x00, 0x20, 0x00, 0, 0, 0, 0]), 1);
    assert_eq!(out.tx.unwrap().data[0] & 0x10, 0);
    let out = c.on_frame(&CanFrame::new(0x585, &[0x30, 0, 0, 0, 0, 0, 0, 0]), 2);
    assert_eq!(out.done.unwrap().result, Err(SdoError::ClientAbort(ABORT_TOGGLE)));
}

#[test]
fn size_mismatch_and_bad_command_abort() {
    let mut c = SdoClient::default();
    c.start(NODE, SdoRequest::Upload { index: 0x2000, sub: 0 }, 0).unwrap();
    c.on_frame(&CanFrame::new(0x585, &[0x41, 0x00, 0x20, 0x00, 10, 0, 0, 0]), 1);
    // Last segment after 7 bytes, size said 10.
    let out = c.on_frame(&CanFrame::new(0x585, &[0x01, 1, 2, 3, 4, 5, 6, 7]), 2);
    assert_eq!(out.done.unwrap().result, Err(SdoError::ClientAbort(ABORT_LENGTH)));
    // Download response to an upload request.
    c.start(NODE, SdoRequest::Upload { index: 0x2000, sub: 0 }, 0).unwrap();
    let out = c.on_frame(&CanFrame::new(0x585, &[0x60, 0x00, 0x20, 0x00, 0, 0, 0, 0]), 1);
    assert_eq!(out.done.unwrap().result, Err(SdoError::ClientAbort(ABORT_COMMAND)));
    // Frames of other nodes are ignored.
    c.start(NODE, SdoRequest::Upload { index: 0x1000, sub: 0 }, 0).unwrap();
    assert_eq!(c.on_frame(&CanFrame::new(0x586, &[0x80, 0, 0x10, 0, 0, 0, 0, 0]), 1), SdoOutput::default());
    assert!(c.is_busy());
    let out = c.cancel();
    assert_eq!(out.done.unwrap().result, Err(SdoError::Cancelled));
    assert_eq!(out.tx.unwrap().data[0], 0x80);
}

#[test]
fn sim_bootup_heartbeat_and_nmt() {
    let mut n = SimNode::builtin(NODE);
    let mut out = Vec::new();
    n.tick(0, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].id, out[0].payload()), (0x705, &[0u8][..]));
    out.clear();
    n.tick(999, &mut out);
    assert!(out.is_empty());
    n.tick(1000, &mut out);
    assert_eq!((out[0].id, out[0].payload()), (0x705, &[0x7Fu8][..]));
    out.clear();
    // Start: TPDO1 (event timer 200 ms) and TPDO2 (1000 ms).
    n.handle_frame(&nmt_frame(NmtCommand::Start, 0), &mut out);
    assert_eq!(n.state(), NodeState::Operational);
    let mut frames = Vec::new();
    for t in (1001..=3000).step_by(10) {
        n.tick(t, &mut frames);
    }
    let tpdo1 = frames.iter().filter(|f| f.id == 0x185).count();
    let tpdo2 = frames.iter().filter(|f| f.id == 0x285).count();
    let hb = frames.iter().filter(|f| f.id == 0x705 && f.data[0] == 5).count();
    assert!((9..=11).contains(&tpdo1), "tpdo1 {tpdo1}");
    assert!((1..=3).contains(&tpdo2), "tpdo2 {tpdo2}");
    assert_eq!(hb, 1); // at 2001 ms; the next one is due at 3001 ms
    // Stop: no SDO answer.
    n.handle_frame(&nmt_frame(NmtCommand::Stop, NODE), &mut out);
    assert_eq!(n.state(), NodeState::Stopped);
    out.clear();
    n.handle_frame(&CanFrame::new(0x605, &[0x40, 0, 0x10, 0, 0, 0, 0, 0]), &mut out);
    assert!(out.is_empty());
    // NMT for another node is ignored.
    n.handle_frame(&nmt_frame(NmtCommand::Start, 9), &mut out);
    assert_eq!(n.state(), NodeState::Stopped);
    // Reset node: boot-up and defaults again.
    n.set_value(0x2002, 0, 7i32.to_le_bytes().to_vec());
    n.handle_frame(&nmt_frame(NmtCommand::ResetNode, NODE), &mut out);
    assert_eq!(out.last().unwrap().id, 0x705);
    assert_eq!(out.last().unwrap().data[0], 0);
    assert_eq!(n.state(), NodeState::PreOperational);
    assert_eq!(n.value(0x2002, 0).unwrap(), (-100i32).to_le_bytes());
}

#[test]
fn sim_sync_pdo_rpdo_and_emcy_with_test_eds() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join("test.eds");
    let od = Arc::new(eds::load(&p).unwrap());
    let mut n = SimNode::new(NODE, od);
    let mut out = Vec::new();
    n.tick(0, &mut out);
    n.handle_frame(&nmt_frame(NmtCommand::Start, 0), &mut out);
    out.clear();
    // TPDO2 has transmission type 1: one frame per SYNC on COB-ID 0x305.
    n.handle_frame(&sync_frame(), &mut out);
    let t2: Vec<&CanFrame> = out.iter().filter(|f| f.id == 0x305).collect();
    assert_eq!(t2.len(), 1);
    assert_eq!(t2[0].dlc, 8);
    assert_eq!(t2[0].data[0], 3); // Mode default 3
    assert_eq!(i16::from_le_bytes([t2[0].data[1], t2[0].data[2]]), -25);
    assert_eq!(t2[0].data[3] & 1, 1); // Enable
    assert_eq!(f32::from_le_bytes([t2[0].data[4], t2[0].data[5], t2[0].data[6], t2[0].data[7]]), 1.5);
    // RPDO1 writes 6200:01 and 6200:02.
    n.handle_frame(&CanFrame::new(0x205, &[0x11, 0x22]), &mut out);
    assert_eq!(n.value(0x6200, 1).unwrap(), &[0x11]);
    assert_eq!(n.value(0x6200, 2).unwrap(), &[0x22]);
    out.clear();
    n.emcy(0x8130, 0x11, &mut out);
    assert_eq!(out[0].id, 0x085);
    assert_eq!(&out[0].data[..3], &[0x30, 0x81, 0x11]);
    assert_eq!(n.value(0x1001, 0).unwrap(), &[0x11]);
}
