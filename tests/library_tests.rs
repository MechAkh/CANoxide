use canoxide::library::{ensure_writable, Device, Library};
use canoxide::settings::{AdapterKind, Settings};
use std::fs;
use std::path::PathBuf;

/// A fresh folder under the system temp folder; removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("canoxide_{tag}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn eds_add_with_name_clash_and_remove() {
    let t = TempDir::new("eds");
    let src_dir = t.0.join("src");
    fs::create_dir_all(src_dir.join("other")).unwrap();
    let a = src_dir.join("io.eds");
    let b = src_dir.join("other").join("io.eds");
    fs::write(&a, b"[1000]\nParameterName=A\n").unwrap();
    fs::write(&b, b"[1000]\nParameterName=B\n").unwrap();
    let mut lib = Library::open(&t.0.join("data")).unwrap();
    assert_eq!(lib.add_eds(&a).unwrap(), "io.eds");
    // Same name, same content: reused.
    assert_eq!(lib.add_eds(&a).unwrap(), "io.eds");
    // Same name, other content: numbered.
    assert_eq!(lib.add_eds(&b).unwrap(), "io_2.eds");
    assert_eq!(lib.add_eds(&b).unwrap(), "io_2.eds");
    assert_eq!(lib.list_eds(), vec!["io.eds".to_string(), "io_2.eds".to_string()]);
    assert_eq!(fs::read(lib.eds_path("io_2.eds")).unwrap(), fs::read(&b).unwrap());

    lib.add_device(Device { name: "Left".into(), node_id: 5, eds_file: Some("io_2.eds".into()), ..Default::default() }).unwrap();
    lib.remove_eds("io_2.eds").unwrap();
    assert_eq!(lib.list_eds(), vec!["io.eds".to_string()]);
    assert_eq!(lib.devices[0].eds_file, None);
    assert!(lib.remove_eds("..\\x.eds").is_err());
    assert!(lib.remove_eds("missing.eds").is_err());
}

#[test]
fn devices_add_rename_remove_and_json_round_trip() {
    let t = TempDir::new("dev");
    let mut lib = Library::open(&t.0).unwrap();
    let d1 = Device { name: "PMU".into(), node_id: 125, eds_file: Some("pmu.eds".into()), bitrate_kbit: 250, notes: "rack \"A\"".into() };
    let d2 = Device { name: "HMI".into(), node_id: 126, ..Default::default() };
    assert_eq!(lib.add_device(d1.clone()).unwrap(), 0);
    assert_eq!(lib.add_device(d2.clone()).unwrap(), 1);
    assert!(lib.add_device(d2.clone()).is_err(), "duplicate name");
    assert!(lib.add_device(Device { name: "X".into(), node_id: 0, ..Default::default() }).is_err());
    assert!(lib.add_device(Device { name: " ".into(), ..Default::default() }).is_err());
    lib.rename_device(1, "HMI 1").unwrap();
    assert!(lib.rename_device(1, "PMU").is_err());
    assert_eq!(lib.find_device("HMI 1"), Some(1));

    let again = Library::open(&t.0).unwrap();
    assert_eq!(again.devices.len(), 2);
    assert_eq!(again.devices[0], d1);
    assert_eq!(again.devices[1].name, "HMI 1");
    let json = fs::read_to_string(t.0.join("devices.json")).unwrap();
    assert!(json.contains("\n"), "human readable");
    assert!(json.contains("\"node_id\": 125"));

    let mut lib = again;
    let removed = lib.remove_device(0).unwrap();
    assert_eq!(removed.name, "PMU");
    assert!(lib.remove_device(5).is_err());
    let again = Library::open(&t.0).unwrap();
    assert_eq!(again.devices.len(), 1);
}

#[test]
fn broken_devices_json_is_kept() {
    let t = TempDir::new("bad");
    fs::write(t.0.join("devices.json"), "{ not json").unwrap();
    let lib = Library::open(&t.0).unwrap();
    assert!(lib.devices.is_empty());
    assert!(lib.load_warning.is_some());
    assert!(t.0.join("devices.json.bad").exists());
}

#[test]
fn settings_round_trip_and_defaults() {
    let t = TempDir::new("set");
    let (s, w) = Settings::load(&t.0);
    assert!(w.is_none());
    assert_eq!(s, Settings::default());
    let mut s2 = s.clone();
    s2.sdo_timeout_ms = 500;
    s2.adapter = AdapterKind::Peak;
    s2.listen_only = true;
    s2.active_device = Some("PMU".into());
    s2.save(&t.0).unwrap();
    let (s3, w) = Settings::load(&t.0);
    assert!(w.is_none());
    assert_eq!(s3, s2);
    fs::write(t.0.join("settings.json"), "{\"sdo_timeout_ms\": 3}").unwrap();
    let (s4, _) = Settings::load(&t.0);
    assert_eq!(s4.sdo_timeout_ms, 10, "clamped");
    assert_eq!(s4.trace_capacity, 100_000, "missing keys get defaults");
    // A settings file of v0.3.3 (no CANfox fields) loads; the CANfox kind
    // round-trips with its own net number.
    fs::write(t.0.join("settings.json"), "{\"adapter\": \"Peak\", \"channel\": 82}").unwrap();
    let (s5, w) = Settings::load(&t.0);
    assert!(w.is_none());
    assert_eq!((s5.adapter, s5.channel, s5.canfox_net), (AdapterKind::Peak, 82, 105));
    let mut s6 = s5.clone();
    s6.adapter = AdapterKind::Canfox;
    s6.canfox_net = 106;
    s6.save(&t.0).unwrap();
    let (s7, _) = Settings::load(&t.0);
    assert_eq!((s7.adapter, s7.channel, s7.canfox_net), (AdapterKind::Canfox, 82, 106));
}

#[test]
fn writable_check() {
    let t = TempDir::new("w");
    ensure_writable(&t.0.join("sub")).unwrap();
    assert!(t.0.join("sub").join("eds").is_dir());
    // A file where the folder should be: not writable.
    let f = t.0.join("file");
    fs::write(&f, b"x").unwrap();
    assert!(ensure_writable(&f).is_err());
    assert!(Library::open(&f).is_err());
}
