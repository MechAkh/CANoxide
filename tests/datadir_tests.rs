//! Data folder rule (installed / portable, migration), saved PDO
//! configurations and the v0.2 settings.

use canoxide::canopen::pdo::{MapEntry, NodePdoConfig, PdoConfig, PdoKind};
use canoxide::library::*;
use canoxide::settings::{DataView, Settings, Theme, TimestampFormat};
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("canoxide_datadir_{tag}_{}_{nanos}", std::process::id()));
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
fn marker_present_uses_appdata() {
    let t = TempDir::new("inst");
    let exe = t.0.join("exe");
    let appdata = t.0.join("Roaming");
    fs::create_dir_all(&exe).unwrap();
    fs::write(exe.join(INSTALLED_MARKER), b"").unwrap();
    // An old portable folder is not migrated in installed mode.
    fs::create_dir_all(exe.join(LEGACY_DATA_DIR_NAME)).unwrap();
    let info = resolve_data_dir(&exe, Some(appdata.as_os_str()));
    assert_eq!(info.mode, DataDirMode::Installed);
    assert_eq!(info.path, appdata.join("CANoxide"));
    assert!(info.path.is_dir(), "created when missing");
    assert_eq!(info.note, None);
    assert!(exe.join(LEGACY_DATA_DIR_NAME).is_dir());
    assert!(!exe.join(DATA_DIR_NAME).exists());
    assert!(info.text().starts_with("Data folder (installed): "));
}

#[test]
fn marker_absent_is_portable() {
    let t = TempDir::new("port");
    let appdata = t.0.join("Roaming");
    let info = resolve_data_dir(&t.0, Some(appdata.as_os_str()));
    assert_eq!(info.mode, DataDirMode::Portable);
    assert_eq!(info.path, t.0.join("CANoxide_data"));
    assert_eq!(info.note, None);
    assert!(!appdata.exists());
}

#[test]
fn marker_present_without_appdata_falls_back_to_portable() {
    let t = TempDir::new("noapp");
    fs::write(t.0.join(INSTALLED_MARKER), b"").unwrap();
    for a in [None, Some(OsStr::new(""))] {
        let info = resolve_data_dir(&t.0, a);
        assert_eq!(info.mode, DataDirMode::Portable);
        assert_eq!(info.path, t.0.join(DATA_DIR_NAME));
        assert!(info.note.as_deref().unwrap().contains("APPDATA is not set"));
    }
}

#[test]
fn portable_mode_migrates_the_old_folder() {
    let t = TempDir::new("mig");
    let old = t.0.join("CANopenTester_data");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("devices.json"), b"{}").unwrap();
    let info = resolve_data_dir(&t.0, None);
    assert_eq!(info.path, t.0.join(DATA_DIR_NAME));
    assert!(info.note.as_deref().unwrap().contains("renamed"));
    assert!(!old.exists());
    assert_eq!(fs::read(info.path.join("devices.json")).unwrap(), b"{}");
    // Second start: nothing to do.
    let again = resolve_data_dir(&t.0, None);
    assert_eq!((again.path, again.note), (t.0.join(DATA_DIR_NAME), None));
    // Both present: the new one wins, the old one stays.
    fs::create_dir_all(&old).unwrap();
    let both = resolve_data_dir(&t.0, None);
    assert_eq!((both.path, both.note), (t.0.join(DATA_DIR_NAME), None));
    assert!(old.exists());
}

#[test]
fn portable_mode_uses_the_old_folder_when_the_rename_fails() {
    let t = TempDir::new("migfail");
    let old = t.0.join(LEGACY_DATA_DIR_NAME);
    fs::create_dir_all(&old).unwrap();
    // A file open without delete sharing blocks the rename of its folder
    // on Windows.
    let held = old.join("held.txt");
    fs::write(&held, b"x").unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _f = fs::OpenOptions::new().read(true).share_mode(0x1).open(&held).unwrap();
        let info = resolve_data_dir(&t.0, None);
        assert_eq!(info.path, old);
        assert_eq!(info.mode, DataDirMode::Portable);
        assert!(info.note.as_deref().unwrap().contains("the old folder is used"), "{:?}", info.note);
    }
}

#[test]
fn pdo_config_persists_per_device() {
    let t = TempDir::new("pdo");
    let lib = Library::open(&t.0).unwrap();
    assert!(lib.load_pdo_config("io: unit/1").unwrap().is_none());
    let cfg = NodePdoConfig {
        node: 5,
        read_time: 1_700_000_000,
        pdos: vec![PdoConfig {
            kind: PdoKind::Tpdo,
            number: 1,
            cob_id: 0x185,
            valid: true,
            extended: false,
            rtr_allowed: true,
            trans_type: 255,
            inhibit_100us: 0,
            event_timer_ms: 200,
            sync_start: None,
            mapping: vec![MapEntry { index: 0x6000, sub: 1, bits: 8 }],
        }],
        complete: true,
        errors: vec![],
    };
    lib.save_pdo_config("io: unit/1", &cfg).unwrap();
    assert!(lib.pdo_config_path("io: unit/1").ends_with("pdo_configs/io_ unit_1.json"));
    let back = Library::open(&t.0).unwrap().load_pdo_config("io: unit/1").unwrap().unwrap();
    assert_eq!(back, cfg);
    assert_eq!(back.read_time_text(), "2023-11-14 22:13:20 UTC");
    lib.remove_pdo_config("io: unit/1").unwrap();
    lib.remove_pdo_config("io: unit/1").unwrap();
    assert!(lib.load_pdo_config("io: unit/1").unwrap().is_none());
    assert_eq!(safe_file_stem("..."), "_");
    assert!(lib.signals_path().ends_with("signals.json"));
    assert!(lib.txlist_path().ends_with("txlist.json"));
}

#[test]
fn v02_settings_defaults_round_trip_and_sanitize() {
    let t = TempDir::new("set");
    let s = Settings::default();
    assert_eq!((s.theme, s.ui_scale, s.sdo_block_size, s.scan_first, s.scan_last), (Theme::System, 1.0, 127, 1, 127));
    assert_eq!((s.signal_capacity, s.pdo_scan_stop, s.confirm_before_write, s.lss_timeout_ms), (20_000, 4, true, 100));
    assert_eq!((s.timestamp_format, s.default_data_view), (TimestampFormat::Relative, DataView::Chronological));
    let mut c = s.clone();
    c.theme = Theme::Dark;
    c.timestamp_format = TimestampFormat::WallClock;
    c.default_data_view = DataView::Fixed;
    c.save(&t.0).unwrap();
    assert_eq!(Settings::load(&t.0).0, c);
    // A v0.1 file without the new keys gets the defaults.
    fs::write(t.0.join(SETTINGS_FILE), br#"{"sdo_timeout_ms": 500}"#).unwrap();
    let (old, warn) = Settings::load(&t.0);
    assert!(warn.is_none());
    assert_eq!((old.sdo_timeout_ms, old.sdo_block_size, old.theme), (500, 127, Theme::System));
    fs::write(t.0.join(SETTINGS_FILE), br#"{"ui_scale": 9.0, "sdo_block_size": 0, "scan_first": 50, "scan_last": 10, "pdo_scan_stop": 0}"#).unwrap();
    let s = Settings::load(&t.0).0;
    assert_eq!((s.ui_scale, s.sdo_block_size, s.scan_first, s.scan_last, s.pdo_scan_stop), (1.0, 1, 50, 50, 1));
}
