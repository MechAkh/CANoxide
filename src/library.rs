//! Device library (`devices.json`) and the managed `eds\` folder in the
//! portable data folder.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const DATA_DIR_NAME: &str = "CANopenTester_data";
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

/// `CANopenTester_data` next to the running exe.
pub fn default_data_dir() -> Result<PathBuf, LibError> {
    let exe = std::env::current_exe().map_err(|e| LibError(format!("exe path unknown: {e}")))?;
    let dir = exe.parent().ok_or_else(|| LibError("exe has no folder".to_string()))?;
    Ok(dir.join(DATA_DIR_NAME))
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
