use canoxide::can::CanFrame;
use canoxide::canopen::abort_text;
use canoxide::canopen::decode::{classify, decode, emcy_code_text, DecodeContext, MsgClass};
use canoxide::eds;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn ctx_with(node: u8, file: &str) -> DecodeContext {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join(file);
    let od = eds::load(&p).unwrap();
    let mut m = HashMap::new();
    m.insert(node, Arc::new(od));
    DecodeContext::new(m)
}

fn d(id: u32, data: &[u8]) -> canoxide::canopen::decode::Decoded {
    decode(&CanFrame::new(id, data), &DecodeContext::default())
}

#[test]
fn classes_by_cob_id() {
    let cases: [(u32, MsgClass, Option<u8>, &str); 16] = [
        (0x000, MsgClass::Nmt, None, "NMT"),
        (0x080, MsgClass::Sync, None, "SYNC"),
        (0x085, MsgClass::Emcy, Some(5), "EMCY"),
        (0x100, MsgClass::Time, None, "TIME"),
        (0x185, MsgClass::Tpdo, Some(5), "TPDO1"),
        (0x205, MsgClass::Rpdo, Some(5), "RPDO1"),
        (0x285, MsgClass::Tpdo, Some(5), "TPDO2"),
        (0x305, MsgClass::Rpdo, Some(5), "RPDO2"),
        (0x385, MsgClass::Tpdo, Some(5), "TPDO3"),
        (0x405, MsgClass::Rpdo, Some(5), "RPDO3"),
        (0x485, MsgClass::Tpdo, Some(5), "TPDO4"),
        (0x57F, MsgClass::Rpdo, Some(127), "RPDO4"),
        (0x585, MsgClass::SdoResp, Some(5), "SDO resp"),
        (0x605, MsgClass::SdoReq, Some(5), "SDO req"),
        (0x705, MsgClass::Heartbeat, Some(5), "Heartbeat"),
        (0x7E5, MsgClass::Lss, None, "LSS"),
    ];
    for (id, class, node, label) in cases {
        let (c, n, l) = classify(id, false);
        assert_eq!((c, n, l.as_str()), (class, node, label), "id 0x{id:03X}");
    }
    assert_eq!(classify(0x7E4, false).0, MsgClass::Lss);
    assert_eq!(classify(0x200, false).0, MsgClass::Other);
    assert_eq!(classify(0x123, true).0, MsgClass::Other);
    assert_eq!(classify(0x7FF, false).0, MsgClass::Other);
}

#[test]
fn nmt_sync_time_lss() {
    assert_eq!(d(0x000, &[0x01, 5]).text, "NMT Start node 5");
    assert_eq!(d(0x000, &[0x02, 0]).text, "NMT Stop all nodes");
    assert_eq!(d(0x000, &[0x80, 3]).text, "NMT Enter pre-operational node 3");
    assert_eq!(d(0x000, &[0x81, 0]).text, "NMT Reset node all nodes");
    assert_eq!(d(0x000, &[0x82, 9]).text, "NMT Reset communication node 9");
    assert_eq!(d(0x080, &[]).text, "SYNC");
    assert_eq!(d(0x080, &[7]).text, "SYNC counter 7");
    // 12:00:00.000 = 43,200,000 ms, day 1.
    let t = d(0x100, &[0x00, 0x2E, 0x93, 0x02, 0x01, 0x00]);
    assert!(t.text.starts_with("TIME 12:00:00.000 day 1"), "{}", t.text);
    assert_eq!(d(0x7E5, &[0x04, 1]).text, "LSS switch state global configuration");
    assert_eq!(d(0x7E4, &[0x5E, 7]).text, "LSS node-ID = 7");
    assert_eq!(d(0x7E5, &[0x77]).text, "LSS master request cs 0x77");
    assert_eq!(d(0x7E4, &[0x44]).text, "LSS switch state selective confirmed");
    assert_eq!(d(0x7E4, &[0x77]).text, "LSS slave response cs 0x77");
}

#[test]
fn heartbeat_states() {
    assert_eq!(d(0x705, &[0x00]).text, "Boot-up");
    assert_eq!(d(0x705, &[0x04]).text, "Heartbeat Stopped");
    assert_eq!(d(0x705, &[0x05]).text, "Heartbeat Operational");
    assert_eq!(d(0x705, &[0x7F]).text, "Heartbeat Pre-operational");
    assert_eq!(d(0x705, &[0x85]).text, "Heartbeat Operational (toggle 1)");
    let mut rtr = CanFrame::new(0x705, &[]);
    rtr.rtr = true;
    assert_eq!(decode(&rtr, &DecodeContext::default()).text, "Node guarding request");
}

#[test]
fn emcy() {
    let e = d(0x085, &[0x30, 0x81, 0x11, 1, 2, 3, 4, 5]);
    assert_eq!(e.node, Some(5));
    assert!(e.text.starts_with("EMCY 0x8130 reg 0x11 Life guard error or heartbeat error"), "{}", e.text);
    assert!(e.text.contains("[generic+communication]"), "{}", e.text);
    assert!(e.text.ends_with("mfr 01 02 03 04 05"), "{}", e.text);
    assert_eq!(emcy_code_text(0x0000), "Error reset or no error");
    assert_eq!(emcy_code_text(0x2310), "Current, device output side");
    assert_eq!(emcy_code_text(0x3210), "Voltage inside the device");
    assert_eq!(emcy_code_text(0x4210), "Device temperature");
    assert_eq!(emcy_code_text(0x5000), "Device hardware");
    assert_eq!(emcy_code_text(0x6100), "Internal software");
    assert_eq!(emcy_code_text(0x8110), "CAN overrun (objects lost)");
    assert_eq!(emcy_code_text(0x8210), "PDO not processed due to length error");
    assert_eq!(emcy_code_text(0x9000), "External error");
    assert_eq!(emcy_code_text(0xFF01), "Device specific");
    assert!(d(0x085, &[1]).text.contains("short"));
}

#[test]
fn sdo_every_command_specifier() {
    let ctx = ctx_with(5, "test.eds");
    let dd = |id: u32, data: [u8; 8]| decode(&CanFrame::new(id, &data), &ctx).text;
    // Upload request / expedited responses with 1..4 bytes.
    assert_eq!(dd(0x605, [0x40, 0x18, 0x10, 0x01, 0, 0, 0, 0]), "SDO upload req 0x1018:01 (Identity object / Vendor ID)");
    assert_eq!(dd(0x585, [0x43, 0x18, 0x10, 0x01, 0x9B, 0x06, 0, 0]), "SDO upload resp 0x1018:01 = 0x0000069B (Identity object / Vendor ID)");
    assert_eq!(dd(0x585, [0x4F, 0x01, 0x10, 0x00, 0x11, 0, 0, 0]), "SDO upload resp 0x1001:00 = 0x11 (Error register)");
    assert_eq!(dd(0x585, [0x4B, 0x17, 0x10, 0x00, 0xF4, 0x01, 0, 0]), "SDO upload resp 0x1017:00 = 0x01F4 (Producer heartbeat time)");
    assert_eq!(dd(0x585, [0x47, 0x00, 0x50, 0x00, 1, 2, 3, 0]), "SDO upload resp 0x5000:00 = 0x030201");
    // Expedited download request 1..4 bytes and its response.
    assert_eq!(dd(0x605, [0x2F, 0x01, 0x20, 0x00, 0x05, 0, 0, 0]), "SDO download req 0x2001:00 = 0x05 (1 bytes) (Mode)");
    assert_eq!(dd(0x605, [0x2B, 0x17, 0x10, 0x00, 0xE8, 0x03, 0, 0]), "SDO download req 0x1017:00 = 0x03E8 (2 bytes) (Producer heartbeat time)");
    assert_eq!(dd(0x605, [0x27, 0x00, 0x50, 0x00, 1, 2, 3, 0]), "SDO download req 0x5000:00 = 0x030201 (3 bytes)");
    assert_eq!(dd(0x605, [0x23, 0x04, 0x20, 0x00, 0x20, 0, 0, 0]), "SDO download req 0x2004:00 = 0x00000020 (4 bytes) (Mask)");
    assert_eq!(dd(0x585, [0x60, 0x04, 0x20, 0x00, 0, 0, 0, 0]), "SDO download resp 0x2004:00 OK (Mask)");
    // Segmented initiate both ways.
    assert_eq!(dd(0x585, [0x41, 0x08, 0x10, 0x00, 14, 0, 0, 0]), "SDO upload resp 0x1008:00 segmented, size 14 (Manufacturer device name)");
    assert_eq!(dd(0x605, [0x21, 0x00, 0x20, 0x00, 100, 0, 0, 0]), "SDO download req 0x2000:00 segmented, size 100 (Device label)");
    // Segments.
    assert_eq!(dd(0x605, [0x60, 0, 0, 0, 0, 0, 0, 0]), "SDO upload segment req t=0");
    assert_eq!(dd(0x605, [0x70, 0, 0, 0, 0, 0, 0, 0]), "SDO upload segment req t=1");
    assert_eq!(dd(0x585, [0x00, b'T', b'e', b's', b't', b' ', b'I', b'O']), "SDO upload segment t=0 7 bytes \"Test IO\"");
    assert_eq!(dd(0x585, [0x1B, b'!', b'!', 0, 0, 0, 0, 0]), "SDO upload segment t=1 2 bytes \"!!\" last");
    assert_eq!(dd(0x605, [0x00, 1, 2, 3, 4, 5, 6, 7]), "SDO download segment t=0 7 bytes \".......\"");
    assert_eq!(dd(0x605, [0x1D, b'x', 0, 0, 0, 0, 0, 0]), "SDO download segment t=1 1 bytes \"x\" last");
    assert_eq!(dd(0x585, [0x20, 0, 0, 0, 0, 0, 0, 0]), "SDO download segment resp t=0");
    assert_eq!(dd(0x585, [0x30, 0, 0, 0, 0, 0, 0, 0]), "SDO download segment resp t=1");
    // Aborts.
    assert_eq!(
        dd(0x585, [0x80, 0x00, 0x50, 0x00, 0x00, 0x00, 0x02, 0x06]),
        "SDO abort by server 0x5000:00 0x06020000 Object does not exist in the object dictionary"
    );
    assert_eq!(
        dd(0x605, [0x80, 0x18, 0x10, 0x01, 0x00, 0x00, 0x04, 0x05]),
        "SDO abort by client 0x1018:01 0x05040000 SDO protocol timed out (Identity object / Vendor ID)"
    );
    // Block transfer is only labelled.
    assert!(dd(0x605, [0xA0, 0, 0, 0, 0, 0, 0, 0]).contains("block"));
}

#[test]
fn string_value_in_expedited_response() {
    let ctx = ctx_with(5, "test.eds");
    let t = decode(&CanFrame::new(0x585, &[0x43, 0x09, 0x10, 0x00, b'H', b'W', b' ', b'1']), &ctx).text;
    assert_eq!(t, "SDO upload resp 0x1009:00 = \"HW 1\" (Manufacturer hardware version)");
}

#[test]
fn abort_code_texts() {
    let codes = [
        (0x0503_0000, "Toggle bit not alternated"),
        (0x0504_0001, "Client/server command specifier not valid or unknown"),
        (0x0601_0001, "Attempt to read a write only object"),
        (0x0601_0002, "Attempt to write a read only object"),
        (0x0607_0010, "Data type does not match, length of service parameter does not match"),
        (0x0609_0011, "Sub-index does not exist"),
        (0x0609_0031, "Value of parameter written too high (download only)"),
        (0x0800_0000, "General error"),
        (0x0800_0022, "Data cannot be transferred or stored to the application because of the present device state"),
        (0x1234_5678, "Unknown abort code"),
    ];
    for (c, t) in codes {
        assert_eq!(abort_text(c), t);
    }
}

#[test]
fn pdo_split_into_signals() {
    let ctx = ctx_with(5, "test.eds");
    // TPDO1: 6000:01 (8), 6000:02 (8), 6401:01 (16, signed).
    let f = CanFrame::new(0x185, &[0x81, 0x02, 0xFE, 0xFF]);
    let r = decode(&f, &ctx);
    assert_eq!(r.class, MsgClass::Tpdo);
    assert_eq!(r.label, "TPDO1");
    assert_eq!(r.node, Some(5));
    assert_eq!(
        r.text,
        "Read input 8 bit / Inputs 1-8 = 129, Read input 8 bit / Inputs 9-16 = 2, Read analog input 16 bit / Analog input 1 = -2"
    );
    // RPDO1: 6200:01, 6200:02.
    let r = decode(&CanFrame::new(0x205, &[1, 2]), &ctx);
    assert_eq!(r.class, MsgClass::Rpdo);
    assert_eq!(r.text, "Write output 8 bit / Write output 8 bit1 = 1, Write output 8 bit / Write output 8 bit2 = 2");
    // Short frame.
    let r = decode(&CanFrame::new(0x185, &[1]), &ctx);
    assert!(r.text.contains("shorter than mapping"), "{}", r.text);
    // Without EDS only the class.
    assert_eq!(d(0x185, &[1, 2]).text, "TPDO1 node 5");
}

#[test]
fn pdo_with_non_default_cob_id() {
    // TPDO2 of node 5 uses 0x305 (in the RPDO2 range by default).
    let ctx = ctx_with(5, "test.eds");
    let mut data = vec![7u8];
    data.extend_from_slice(&(-25i16).to_le_bytes());
    data.push(0x01); // Enable = 1, then 7 dummy bits
    data.extend_from_slice(&1.5f32.to_le_bytes());
    let r = decode(&CanFrame::new(0x305, &data), &ctx);
    assert_eq!(r.class, MsgClass::Tpdo);
    assert_eq!(r.label, "TPDO2");
    assert_eq!(r.text, "Mode = 7, Temperature offset = -25, Enable = true, Speed = 1.5");
    // The DCF gives TPDO1 of node 10 the COB-ID 0x1C0.
    let ctx = ctx_with(10, "test.dcf");
    let r = decode(&CanFrame::new(0x1C0, &[0x34, 0x12]), &ctx);
    assert_eq!(r.label, "TPDO1");
    assert_eq!(r.node, Some(10));
    assert_eq!(r.text, "Counter = 4660");
}
