use canopen_tester::canopen::pdo::{configs_from_od, PdoKind};
use canopen_tester::canopen::types::DataType;
use canopen_tester::eds::{self, AccessType, ObjectType};
use std::path::Path;

fn data(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join(name)
}

#[test]
fn parses_test_eds_without_warnings() {
    let od = eds::load(&data("test.eds")).unwrap();
    assert!(od.warnings.is_empty(), "warnings: {:#?}", od.warnings);
    assert!(od.objects.len() >= 30, "only {} objects", od.objects.len());
    assert!(!od.is_dcf);
    assert_eq!(od.file_info_value("description"), Some("Test device"));
    assert_eq!(od.device_info_value("VENDORNAME"), Some("Test vendor"));
}

#[test]
fn var_record_array_and_compact() {
    let od = eds::load(&data("test.eds")).unwrap();
    let dt = &od.objects[&0x1000];
    assert_eq!(dt.object_type, ObjectType::Var);
    assert_eq!(dt.subs[&0].data_type, 7);
    assert_eq!(dt.subs[&0].access, AccessType::Ro);

    let id = &od.objects[&0x1018];
    assert_eq!(id.object_type, ObjectType::Record);
    assert_eq!(id.subs.len(), 5);
    assert_eq!(id.subs[&1].name, "Vendor ID");
    assert_eq!(od.name_of(0x1018, 1).unwrap(), "Identity object / Vendor ID");

    let arr = &od.objects[&0x1010];
    assert_eq!(arr.object_type, ObjectType::Array);
    assert_eq!(arr.subs.len(), 2);

    // CompactSubObj with names from [1003Name] and generated names.
    let c = &od.objects[&0x1003];
    assert_eq!(c.subs.len(), 5);
    assert_eq!(c.subs[&0].default.as_deref(), Some("4"));
    assert_eq!(c.subs[&0].access, AccessType::Ro);
    assert_eq!(c.subs[&1].name, "Newest error");
    assert_eq!(c.subs[&2].name, "Second error");
    assert_eq!(c.subs[&3].name, "Pre-defined error field3");
    assert_eq!(c.subs[&4].data_type, 7);
    let inputs = &od.objects[&0x6000];
    assert_eq!(inputs.subs[&2].name, "Inputs 9-16");
    assert!(inputs.subs[&1].pdo_mapping);
    // Mixed-case "Sub" in a section name.
    assert_eq!(od.objects[&0x6401].subs[&2].name, "Analog input 2");
}

#[test]
fn nodeid_expressions_numbers_and_strings() {
    let od = eds::load(&data("test.eds")).unwrap();
    let e = od.entry(0x1014, 0).unwrap();
    assert_eq!(e.effective_bytes(5).unwrap(), vec![0x85, 0, 0, 0]);
    assert_eq!(od.entry(0x1200, 2).unwrap().effective_bytes(5).unwrap(), vec![0x85, 0x05, 0, 0]);
    assert_eq!(eds::eval_int("$NODEID+0x180", 3), Some(0x183));
    assert_eq!(eds::eval_int("$nodeid + 0x300", 5), Some(0x305));
    assert_eq!(eds::eval_int("0x600+$NODEID", 1), Some(0x601));
    assert_eq!(eds::eval_int("010", 1), Some(8));

    let s = od.entry(0x2000, 0).unwrap();
    assert_eq!(s.dtype(), Some(DataType::VisibleString));
    assert_eq!(s.effective_bytes(1).unwrap(), b"Bench unit 1".to_vec());
    assert_eq!(od.entry(0x2008, 0).unwrap().effective_bytes(1).unwrap(), vec![0x80]);
    assert_eq!(od.entry(0x200B, 0).unwrap().effective_bytes(1).unwrap(), vec![8, 0]);
    assert_eq!(od.entry(0x200A, 0).unwrap().effective_bytes(1).unwrap(), vec![1, 2, 3, 4, 5]);
    assert_eq!(od.entry(0x2007, 0).unwrap().effective_bytes(1).unwrap(), 0x0123_4567_89AB_CDEFu64.to_le_bytes().to_vec());
    assert_eq!(od.entry(0x2005, 0).unwrap().effective_bytes(1).unwrap(), 1.5f32.to_le_bytes().to_vec());
}

#[test]
fn lower_case_keys_and_limits() {
    let od = eds::load(&data("test.eds")).unwrap();
    let e = od.entry(0x2002, 0).unwrap();
    assert_eq!(od.objects[&0x2002].name, "Temperature offset");
    assert_eq!(e.dtype(), Some(DataType::Integer16));
    assert_eq!(e.access, AccessType::Rw);
    assert_eq!(e.low.as_deref(), Some("-500"));
    assert_eq!(e.high.as_deref(), Some("500"));
    assert!(e.pdo_mapping);
    let m = od.entry(0x2004, 0).unwrap();
    assert_eq!(m.low.as_deref(), Some("0x00000010"));
    assert_eq!(od.entry(0x2003, 0).unwrap().access, AccessType::Rww);
    assert_eq!(od.entry(0x1008, 0).unwrap().access, AccessType::Const);
}

#[test]
fn pdo_configs_from_eds() {
    let od = eds::load(&data("test.eds")).unwrap();
    let cfgs = configs_from_od(&od, 5);
    let t1 = cfgs.iter().find(|c| c.kind == PdoKind::Tpdo && c.number == 1).unwrap();
    assert_eq!(t1.cob_id, 0x185);
    assert!(t1.valid);
    assert_eq!(t1.event_timer_ms, 100);
    assert_eq!(t1.mapping.len(), 3);
    assert_eq!(t1.mapping[2].index, 0x6401);
    assert_eq!(t1.mapping[2].bits, 16);
    let t2 = cfgs.iter().find(|c| c.kind == PdoKind::Tpdo && c.number == 2).unwrap();
    assert_eq!(t2.cob_id, 0x305);
    assert_eq!(t2.trans_type, 1);
    assert_eq!(t2.total_bits(), 64);
    assert!(t2.mapping[3].is_dummy());
    let r1 = cfgs.iter().find(|c| c.kind == PdoKind::Rpdo && c.number == 1).unwrap();
    assert_eq!(r1.cob_id, 0x205);
    assert_eq!(r1.mapping.len(), 2);
}

#[test]
fn parses_dcf() {
    let od = eds::load(&data("test.dcf")).unwrap();
    assert!(od.warnings.is_empty(), "warnings: {:#?}", od.warnings);
    assert!(od.is_dcf);
    assert_eq!(od.node_id, Some(10));
    assert_eq!(od.baudrate_kbit, Some(500));
    let e = od.entry(0x1800, 1).unwrap();
    assert_eq!(e.param_value.as_deref(), Some("0x000001C0"));
    assert_eq!(e.effective_bytes(10).unwrap(), vec![0xC0, 0x01, 0, 0]);
    assert_eq!(od.entry(0x1016, 1).unwrap().param_value.as_deref(), Some("0x000501F4"));
    assert_eq!(od.entry(0x1016, 2).unwrap().param_value, None);
    assert_eq!(od.entry(0x2001, 0).unwrap().effective_bytes(10).unwrap(), vec![7, 0]);
    let cfgs = configs_from_od(&od, 10);
    let t1 = cfgs.iter().find(|c| c.kind == PdoKind::Tpdo && c.number == 1).unwrap();
    assert_eq!(t1.cob_id, 0x1C0);
    assert_eq!(t1.trans_type, 254);
    assert_eq!(t1.mapping.len(), 1);
}

#[test]
fn malformed_file_gives_warnings_not_panic() {
    let od = eds::load(&data("malformed.eds")).unwrap();
    let w = od.warnings.join("\n");
    assert!(od.warnings.len() >= 8, "{w}");
    assert!(w.contains("without ']'"), "{w}");
    assert!(w.contains("outside a section"), "{w}");
    assert!(w.contains("no '='"), "{w}");
    assert!(w.contains("0x5555 listed but has no section"), "{w}");
    assert!(w.contains("bad object index 'nonsense'"), "{w}");
    assert!(w.contains("bad DataType"), "{w}");
    assert!(w.contains("unknown AccessType"), "{w}");
    assert!(w.contains("unknown key 'VendorSpecialKey'"), "{w}");
    assert!(w.contains("no section [1002]"), "{w}");
    assert!(w.contains("appears again"), "{w}");
    // The broken objects still exist.
    assert!(od.objects.contains_key(&0x1000));
    assert_eq!(od.objects[&0x1001].object_type, ObjectType::Var);
}

#[test]
fn random_bytes_never_panic() {
    let mut x: u64 = 0x1234_5678_9ABC_DEF0;
    let alphabet = b"[]=;$+-0x1Fsub \r\nNODEIDCompactSubObj=ParameterValue7";
    for len in [0usize, 1, 7, 50, 300, 2000] {
        for _ in 0..20 {
            let mut v = Vec::with_capacity(len);
            for _ in 0..len {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v.push(if x.is_multiple_of(3) { (x >> 8) as u8 } else { alphabet[(x as usize >> 3) % alphabet.len()] });
            }
            let od = eds::parse_bytes(&v);
            let _ = canopen_tester::canopen::pdo::configs_from_od(&od, 1);
        }
    }
}

#[test]
fn windows_1252_and_bom() {
    let mut bytes = b"[1000]\r\nParameterName=Temp \xB0C\r\nDataType=0x0007\r\nAccessType=ro\r\n".to_vec();
    let od = eds::parse_bytes(&bytes);
    assert_eq!(od.objects[&0x1000].name, "Temp \u{00B0}C");
    bytes = [&[0xEF, 0xBB, 0xBF][..], "[1000]\nParameterName=Grad °C\nDataType=7\nAccessType=ro\n".as_bytes()].concat();
    let od = eds::parse_bytes(&bytes);
    assert_eq!(od.objects[&0x1000].name, "Grad °C");
    assert!(od.warnings.is_empty(), "{:?}", od.warnings);
}
