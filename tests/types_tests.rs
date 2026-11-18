use canopen_tester::canopen::types::*;

fn sample(dt: DataType) -> Vec<Value> {
    use DataType::*;
    match dt {
        Boolean => vec![Value::Bool(false), Value::Bool(true)],
        Real32 => vec![Value::F32(0.0), Value::F32(-12.5), Value::F32(3.25e10)],
        Real64 => vec![Value::F64(0.0), Value::F64(-1.0e-300), Value::F64(123456.789)],
        VisibleString => vec![Value::Str(String::new()), Value::Str("Hello CANopen".into())],
        UnicodeString => vec![Value::Str("Grüße €".into())],
        OctetString | Domain => vec![Value::Bytes(vec![]), Value::Bytes(vec![0, 1, 0xFE, 0xFF])],
        TimeOfDay | TimeDifference => vec![Value::Bytes(vec![1, 2, 3, 4, 5, 6])],
        _ => {
            let (lo, hi) = dt.int_range().unwrap();
            if dt.is_signed() {
                vec![Value::Int(lo as i64), Value::Int(-1), Value::Int(0), Value::Int(hi as i64)]
            } else {
                vec![Value::UInt(0), Value::UInt(1), Value::UInt(hi as u64)]
            }
        }
    }
}

#[test]
fn round_trip_every_type() {
    assert_eq!(DataType::ALL.len(), 25);
    for dt in DataType::ALL {
        assert_eq!(DataType::from_code(dt.code()), Some(dt));
        for v in sample(dt) {
            let b = encode(dt, &v).unwrap_or_else(|e| panic!("{dt}: {e}"));
            if let Some(n) = dt.size() {
                assert_eq!(b.len(), n, "{dt}");
            }
            let back = decode(dt, &b).unwrap();
            assert_eq!(back, v, "{dt}");
        }
    }
}

#[test]
fn text_to_bytes_little_endian() {
    use DataType::*;
    assert_eq!(parse_text(Unsigned16, "0x1234", false).unwrap(), vec![0x34, 0x12]);
    assert_eq!(parse_text(Unsigned32, "4660", false).unwrap(), vec![0x34, 0x12, 0, 0]);
    assert_eq!(parse_text(Integer16, "-2", false).unwrap(), vec![0xFE, 0xFF]);
    assert_eq!(parse_text(Integer8, "0xFF", false).unwrap(), vec![0xFF]);
    assert_eq!(parse_text(Integer24, "-1", false).unwrap(), vec![0xFF, 0xFF, 0xFF]);
    assert_eq!(parse_text(Unsigned24, "0xABCDEF", false).unwrap(), vec![0xEF, 0xCD, 0xAB]);
    assert_eq!(parse_text(Unsigned40, "0x0102030405", false).unwrap(), vec![5, 4, 3, 2, 1]);
    assert_eq!(parse_text(Integer48, "-3", false).unwrap().len(), 6);
    assert_eq!(parse_text(Integer56, "100", false).unwrap(), vec![100, 0, 0, 0, 0, 0, 0]);
    assert_eq!(parse_text(Unsigned64, "18446744073709551615", false).unwrap(), vec![0xFF; 8]);
    assert_eq!(parse_text(Integer64, "-9223372036854775808", false).unwrap(), (i64::MIN).to_le_bytes().to_vec());
    assert_eq!(parse_text(Boolean, "true", false).unwrap(), vec![1]);
    assert_eq!(parse_text(Boolean, "0", false).unwrap(), vec![0]);
    assert_eq!(parse_text(Real32, "1.5", false).unwrap(), 1.5f32.to_le_bytes().to_vec());
    assert_eq!(parse_text(Real32, "0x3FC00000", false).unwrap(), 1.5f32.to_le_bytes().to_vec());
    assert_eq!(parse_text(Real64, "-2.5", false).unwrap(), (-2.5f64).to_le_bytes().to_vec());
    assert_eq!(parse_text(VisibleString, "abc", false).unwrap(), b"abc".to_vec());
    assert_eq!(parse_text(UnicodeString, "A", false).unwrap(), vec![0x41, 0]);
    assert_eq!(parse_text(OctetString, "01 02 ff", false).unwrap(), vec![1, 2, 0xFF]);
    assert_eq!(parse_text(Domain, "0x0A0B", false).unwrap(), vec![0x0A, 0x0B]);
    assert_eq!(parse_text(TimeOfDay, "01 02 03 04 05 06", false).unwrap().len(), 6);
    assert_eq!(parse_text(Unsigned8, "010", true).unwrap(), vec![8]);
    assert_eq!(parse_text(Unsigned8, "010", false).unwrap(), vec![10]);
}

#[test]
fn text_errors() {
    use DataType::*;
    assert!(parse_text(Unsigned8, "256", false).is_err());
    assert!(parse_text(Unsigned8, "-1", false).is_err());
    assert!(parse_text(Integer8, "128", false).is_err());
    assert!(parse_text(Unsigned16, "abc", false).is_err());
    assert!(parse_text(Boolean, "2", false).is_err());
    assert!(parse_text(OctetString, "123", false).is_err());
    assert!(parse_text(TimeOfDay, "01 02", false).is_err());
    assert!(parse_text(Real32, "x1", false).is_err());
    assert!(decode(Unsigned32, &[1, 2]).is_err());
}

#[test]
fn display_modes() {
    use DataType::*;
    let b = [0x34, 0x12];
    assert_eq!(format_value(Some(Unsigned16), &b, DisplayMode::Hex), "0x1234");
    assert_eq!(format_value(Some(Unsigned16), &b, DisplayMode::Decimal), "4660");
    assert_eq!(format_value(Some(Unsigned16), &b, DisplayMode::String), "4.");
    assert_eq!(format_value(Some(Integer8), &[0xFF], DisplayMode::Decimal), "-1");
    assert_eq!(format_value(Some(Integer8), &[0xFF], DisplayMode::Hex), "0xFF");
    assert_eq!(format_value(Some(Unsigned32), &[0x9B, 0x06, 0, 0], DisplayMode::Hex), "0x0000069B");
    assert_eq!(format_value(Some(Real32), &1.5f32.to_le_bytes(), DisplayMode::Decimal), "1.5");
    assert_eq!(format_value(Some(VisibleString), b"Hi!", DisplayMode::String), "Hi!");
    assert_eq!(format_value(Some(VisibleString), b"Hi!", DisplayMode::Hex), "48 69 21");
    assert_eq!(format_value(Some(VisibleString), b"Hi!", DisplayMode::Decimal), "72 105 33");
    assert_eq!(format_value(Some(Boolean), &[1], DisplayMode::Decimal), "1");
    assert_eq!(format_value(None, &[0x41, 0x00], DisplayMode::String), "A.");
    assert_eq!(format_bytes(&[0x41, 0x0A, 0xFF], DisplayMode::Hex), "41 0A FF");
    assert_eq!(format_bytes(&[0x41, 0x0A, 0xFF], DisplayMode::Decimal), "65 10 255");
    assert_eq!(format_bytes(&[0x41, 0x0A, 0xFF], DisplayMode::String), "A..");
}

#[test]
fn limits() {
    use DataType::*;
    assert!(check_limits(Unsigned8, &[5], Some("1"), Some("10")).is_ok());
    assert!(check_limits(Unsigned8, &[11], Some("1"), Some("10")).is_err());
    assert!(check_limits(Unsigned8, &[0], Some("1"), Some("10")).is_err());
    assert!(check_limits(Integer16, &(-501i16).to_le_bytes(), Some("-500"), Some("500")).is_err());
    assert!(check_limits(Integer16, &(-500i16).to_le_bytes(), Some("-500"), Some("500")).is_ok());
    // Hex limits of a signed object in two's complement.
    assert!(check_limits(Integer8, &[0xF0], Some("0xF0"), Some("0x10")).is_ok());
    assert!(check_limits(Integer8, &[0xEF], Some("0xF0"), Some("0x10")).is_err());
    assert!(check_limits(Unsigned32, &0x20u32.to_le_bytes(), Some("0x00000010"), Some("0x00FFFFFF")).is_ok());
    assert!(check_limits(Real32, &2.5f32.to_le_bytes(), Some("0.0"), Some("2.0")).is_err());
    // Unparsable limits are ignored; strings have no limits.
    assert!(check_limits(Unsigned8, &[200], Some("x"), None).is_ok());
    assert!(check_limits(VisibleString, b"zz", Some("1"), Some("2")).is_ok());
    assert_eq!(Integer24.int_range(), Some((-(1 << 23), (1 << 23) - 1)));
    assert_eq!(Unsigned56.int_range(), Some((0, (1i128 << 56) - 1)));
}
