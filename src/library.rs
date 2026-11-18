//! Device library (`devices.json`), the managed `eds\` folder, the saved
//! PDO configurations, and the rule that finds the data folder.

use crate::canopen::pdo::NodePdoConfig;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Portable data folder next to the exe.
pub const DATA_DIR_NAME: &str = "CANoxide_data";
/// Portable data folder of the versions before the rename (migrated).
pub const LEGACY_DATA_DIR_NAME: &str = "CANopenTester_data";
/// Marker file next to the exe of an installed copy.
pub const INSTALLED_MARKER: &str = "CANoxide.installed";
/// Folder name inside `%APPDATA%` for an installed copy.
pub const APPDATA_DIR_NAME: &str = "CANoxide";
/// Folder of the last read PDO configuration of each device.
pub const PDO_CONFIG_DIR: &str = "pdo_configs";
pub const DEVICES_FILE: &str = "devices.json";
pub const SETTINGS_FILE: &str = "settings.json";
pub const EDS_DIR: &str = "eds";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    pub name: String,
    pub node_id: u8,
    /// File name inside `eds\`.
    pub eds_file: Option<String>,
    pub bitrate_kbit: u32,
    pub notes: String,
}

impl Default for Device {
    fn default() -> Self {
        Device { name: "New device".to_string(), node_id: 1, eds_file: None, bitrate_kbit: 250, notes: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DevicesFile {
    version: u32,
    devices: Vec<Device>,
}

#[derive(Debug)]
pub struct LibError(pub String);

impl fmt::Display for LibError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LibError {}

fn io_err(what: &str, p: &Path, e: std::io::Error) -> LibError {
    LibError(format!("{what} {}: {e}", p.display()))
}

/// How the data folder was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataDirMode {
    /// `CANoxide.installed` next to the exe: `%APPDATA%\CANoxide`.
    Installed,
    /// `CANoxide_data` next to the exe.
    Portable,
}

impl DataDirMode {
    pub fn label(self) -> &'static str {
        match self {
            DataDirMode::Installed => "installed",
            DataDirMode::Portable => "portable",
        }
    }
}

/// The data folder of the GUI and how it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDirInfo {
    pub path: PathBuf,
    pub mode: DataDirMode,
    /// Something the status line should say (a migration, a fallback).
    pub note: Option<String>,
}

impl DataDirInfo {
    /// "Data folder (portable): D:\x\CANoxide_data".
    pub fn text(&self) -> String {
        format!("Data folder ({}): {}", self.mode.label(), self.path.display())
    }
}

/// Resolve the data folder of the GUI. `exe_dir` is the folder of the exe,
/// `appdata` the value of the `APPDATA` environment variable.
///
/// 1. `CANoxide.installed` next to the exe: `<APPDATA>\CANoxide` (created
///    when missing). Without `APPDATA`, rule 2 applies, with a note.
/// 2. Else portable: `CANoxide_data` next to the exe. When it is missing and
///    `CANopenTester_data` exists, the old folder is renamed; when the
///    rename fails, the old folder is used in place, with a note. The
///    migration runs only in portable mode.
///
/// The portable folder is not created here (`ensure_writable` does that).
pub fn resolve_data_dir(exe_dir: &Path, appdata: Option<&OsStr>) -> DataDirInfo {
    let mut note = None;
    if exe_dir.join(INSTALLED_MARKER).is_file() {
        match appdata.filter(|a| !a.is_empty()) {
            Some(a) => {
                let path = Path::new(a).join(APPDATA_DIR_NAME);
                if let Err(e) = fs::create_dir_all(&path) {
                    note = Some(format!("cannot create {}: {e}", path.display()));
                }
                return DataDirInfo { path, mode: DataDirMode::Installed, note };
            }
            None => note = Some(format!("{INSTALLED_MARKER} found but APPDATA is not set; the portable data folder is used")),
        }
    }
    let path = exe_dir.join(DATA_DIR_NAME);
    let old = exe_dir.join(LEGACY_DATA_DIR_NAME);
    if !path.exists() && old.is_dir() {
        match fs::rename(&old, &path) {
            Ok(()) => note = join_note(note, format!("data folder {LEGACY_DATA_DIR_NAME} renamed to {DATA_DIR_NAME}")),
            Err(e) => {
                let n = format!("{LEGACY_DATA_DIR_NAME} could not be renamed to {DATA_DIR_NAME} ({e}); the old folder is used");
                return DataDirInfo { path: old, mode: DataDirMode::Portable, note: join_note(note, n) };
            }
        }
    }
    DataDirInfo { path, mode: DataDirMode::Portable, note }
}

fn join_note(a: Option<String>, b: String) -> Option<String> {
    Some(match a {
        Some(a) => format!("{a}; {b}"),
        None => b,
    })
}

/// The data folder of the running exe (`resolve_data_dir` with the exe
/// folder and `%APPDATA%`).
pub fn default_data_dir() -> Result<DataDirInfo, LibError> {
    let exe = std::env::current_exe().map_err(|e| LibError(format!("exe path unknown: {e}")))?;
    let dir = exe.parent().ok_or_else(|| LibError("exe has no folder".to_string()))?;
    Ok(resolve_data_dir(dir, std::env::var_os("APPDATA").as_deref()))
}

/// Create the folder (and `eds\`) and prove that it is writable.
pub fn ensure_writable(dir: &Path) -> Result<(), LibError> {
    fs::create_dir_all(dir.join(EDS_DIR)).map_err(|e| io_err("cannot create", dir, e))?;
    let probe = dir.join(".write_test");
    fs::write(&probe, b"ok").map_err(|e| io_err("folder not writable:", dir, e))?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Write a file through a temporary file and a rename.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), LibError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data).map_err(|e| io_err("cannot write", &tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| io_err("cannot replace", path, e))
}

pub struct Library {
    root: PathBuf,
    pub devices: Vec<Device>,
    /// Problem found while loading `devices.json` (the file is kept).
    pub load_warning: Option<String>,
}

impl Library {
    /// Open the library in `root` (created when missing).
    pub fn open(root: &Path) -> Result<Library, LibError> {
        ensure_writable(root)?;
        let mut lib = Library { root: root.to_path_buf(), devices: Vec::new(), load_warning: None };
        let p = lib.devices_path();
        if p.exists() {
            let text = fs::read_to_string(&p).map_err(|e| io_err("cannot read", &p, e))?;
            match serde_json::from_str::<DevicesFile>(&text) {
                Ok(f) => lib.devices = f.devices,
                Err(e) => {
                    // Keep the broken file for the user; do not overwrite it silently.
                    let backup = p.with_extension("json.bad");
                    let _ = fs::copy(&p, &backup);
                    lib.load_warning = Some(format!("{} could not be read ({e}); a copy is in {}", p.display(), backup.display()));
                }
            }
        }
        Ok(lib)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn devices_path(&self) -> PathBuf {
        self.root.join(DEVICES_FILE)
    }

    pub fn eds_dir(&self) -> PathBuf {
        self.root.join(EDS_DIR)
    }

    pub fn eds_path(&self, name: &str) -> PathBuf {
        self.eds_dir().join(name)
    }

    pub fn save(&self) -> Result<(), LibError> {
        let f = DevicesFile { version: 1, devices: self.devices.clone() };
        let text = serde_json::to_string_pretty(&f).map_err(|e| LibError(e.to_string()))?;
        write_atomic(&self.devices_path(), text.as_bytes())
    }

    /// EDS / DCF files in `eds\`, sorted by name.
    pub fn list_eds(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.eds_dir())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                    .filter(|n| {
                        let l = n.to_ascii_lowercase();
                        l.ends_with(".eds") || l.ends_with(".dcf")
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|s| s.to_ascii_lowercase());
        v
    }

    /// Copy a file into `eds\`. The same name with the same content is
    /// reused; the same name with other content gets "name_2.eds", ...
    /// Returns the file name in the library.
    pub fn add_eds(&mut self, src: &Path) -> Result<String, LibError> {
        let data = fs::read(src).map_err(|e| io_err("cannot read", src, e))?;
        let file = src.file_name().and_then(|n| n.to_str()).ok_or_else(|| LibError("invalid file name".to_string()))?;
        let (stem, ext) = match file.rfind('.') {
            Some(p) if p > 0 => (&file[..p], &file[p..]),
            _ => (file, ""),
        };
        let mut n = 1;
        loop {
            let name = if n == 1 { file.to_string() } else { format!("{stem}_{n}{ext}") };
            let target = self.eds_path(&name);
            if !target.exists() {
                fs::write(&target, &data).map_err(|e| io_err("cannot write", &target, e))?;
                return Ok(name);
            }
            if fs::read(&target).map(|d| d == data).unwrap_or(false) {
                return Ok(name);
            }
            n += 1;
            if n > 999 {
                return Err(LibError("too many files with this name".to_string()));
            }
        }
    }

    /// Delete a file from `eds\` (the UI asks first). Devices that use it
    /// lose their EDS assignment. Saves `devices.json` when it changed.
    pub fn remove_eds(&mut self, name: &str) -> Result<(), LibError> {
        if name.contains(['/', '\\']) || name.contains("..") {
            return Err(LibError("invalid file name".to_string()));
        }
        let p = self.eds_path(name);
        fs::remove_file(&p).map_err(|e| io_err("cannot delete", &p, e))?;
        let mut changed = false;
        for d in &mut self.devices {
            if d.eds_file.as_deref() == Some(name) {
                d.eds_file = None;
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(())
    }

    /// `signals.json` (signal definitions).
    pub fn signals_path(&self) -> PathBuf {
        self.root.join(crate::signals::SIGNALS_FILE)
    }

    /// `txlist.json` (transmit list).
    pub fn txlist_path(&self) -> PathBuf {
        self.root.join(crate::txlist::TXLIST_FILE)
    }

    /// File of the saved PDO configuration of a device.
    pub fn pdo_config_path(&self, device: &str) -> PathBuf {
        self.root.join(PDO_CONFIG_DIR).join(format!("{}.json", safe_file_stem(device)))
    }

    /// Save the last read PDO configuration of a device (its read time is
    /// inside).
    pub fn save_pdo_config(&self, device: &str, cfg: &NodePdoConfig) -> Result<(), LibError> {
        let p = self.pdo_config_path(device);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).map_err(|e| io_err("cannot create", d, e))?;
        }
        let text = serde_json::to_string_pretty(cfg).map_err(|e| LibError(e.to_string()))?;
        write_atomic(&p, text.as_bytes())
    }

    /// The saved PDO configuration of a device; `Ok(None)` when none.
    pub fn load_pdo_config(&self, device: &str) -> Result<Option<NodePdoConfig>, LibError> {
        let p = self.pdo_config_path(device);
        match fs::read_to_string(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err("cannot read", &p, e)),
            Ok(t) => serde_json::from_str(&t).map(Some).map_err(|e| LibError(format!("{}: {e}", p.display()))),
        }
    }

    /// Delete the saved PDO configuration of a device (no error when none).
    pub fn remove_pdo_config(&self, device: &str) -> Result<(), LibError> {
        let p = self.pdo_config_path(device);
        match fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_err("cannot delete", &p, e)),
            _ => Ok(()),
        }
    }

    pub fn find_device(&self, name: &str) -> Option<usize> {
        self.devices.iter().position(|d| d.name == name)
    }

    fn check_name(&self, name: &str, except: Option<usize>) -> Result<(), LibError> {
        if name.trim().is_empty() {
            return Err(LibError("device name is empty".to_string()));
        }
        if self.devices.iter().enumerate().any(|(i, d)| Some(i) != except && d.name == name) {
            return Err(LibError(format!("a device named '{name}' exists already")));
        }
        Ok(())
    }

    fn check_device(&self, d: &Device, except: Option<usize>) -> Result<(), LibError> {
        self.check_name(&d.name, except)?;
        if !(1..=127).contains(&d.node_id) {
            return Err(LibError("node ID must be 1..127".to_string()));
        }
        Ok(())
    }

    /// Add a device; returns its index. Saves.
    pub fn add_device(&mut self, d: Device) -> Result<usize, LibError> {
        self.check_device(&d, None)?;
        self.devices.push(d);
        self.save()?;
        Ok(self.devices.len() - 1)
    }

    /// Replace a device (edit). Saves.
    pub fn update_device(&mut self, idx: usize, d: Device) -> Result<(), LibError> {
        if idx >= self.devices.len() {
            return Err(LibError("no such device".to_string()));
        }
        self.check_device(&d, Some(idx))?;
        self.devices[idx] = d;
        self.save()
    }

    pub fn rename_device(&mut self, idx: usize, name: &str) -> Result<(), LibError> {
        let mut d = self.devices.get(idx).cloned().ok_or_else(|| LibError("no such device".to_string()))?;
        d.name = name.to_string();
        self.update_device(idx, d)
    }

    pub fn remove_device(&mut self, idx: usize) -> Result<Device, LibError> {
        if idx >= self.devices.len() {
            return Err(LibError("no such device".to_string()));
        }
        let d = self.devices.remove(idx);
        self.save()?;
        Ok(d)
    }
}

/// A device name as a file name stem: characters that Windows does not
/// allow in a file name become `_`.
pub fn safe_file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let s = s.trim_end_matches(['.', ' ']);
    if s.is_empty() {
        "_".to_string()
    } else {
        s.to_string()
    }
}
