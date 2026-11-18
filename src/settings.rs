//! `settings.json` in the portable data folder.

use crate::canopen::types::DisplayMode;
use crate::library::{write_atomic, LibError, SETTINGS_FILE};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AdapterKind {
    #[default]
    Simulator,
    Peak,
}

impl AdapterKind {
    pub fn label(self) -> &'static str {
        match self {
            AdapterKind::Simulator => "Simulator",
            AdapterKind::Peak => "PEAK PCAN",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedFrame {
    pub name: String,
    pub id: u32,
    pub extended: bool,
    pub rtr: bool,
    pub dlc: u8,
    pub data: String,
    pub period_ms: u32,
}

impl Default for SavedFrame {
    fn default() -> Self {
        SavedFrame { name: String::new(), id: 0x123, extended: false, rtr: false, dlc: 8, data: String::new(), period_ms: 100 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub sdo_timeout_ms: u64,
    pub scan_timeout_ms: u64,
    pub trace_capacity: usize,
    pub adapter: AdapterKind,
    pub channel: u32,
    pub bitrate_kbit: u32,
    pub listen_only: bool,
    pub value_display: DisplayMode,
    pub trace_display: DisplayMode,
    pub window_width: f32,
    pub window_height: f32,
    pub active_device: Option<String>,
    pub saved_frames: Vec<SavedFrame>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sdo_timeout_ms: 1000,
            scan_timeout_ms: 100,
            trace_capacity: 100_000,
            adapter: AdapterKind::Simulator,
            channel: 0x51,
            bitrate_kbit: 250,
            listen_only: false,
            value_display: DisplayMode::Hex,
            trace_display: DisplayMode::Hex,
            window_width: 1280.0,
            window_height: 800.0,
            active_device: None,
            saved_frames: Vec::new(),
        }
    }
}

impl Settings {
    /// Load; a missing file gives the defaults, a broken one an error text
    /// together with the defaults.
    pub fn load(dir: &Path) -> (Settings, Option<String>) {
        let p = dir.join(SETTINGS_FILE);
        match std::fs::read_to_string(&p) {
            Err(_) => (Settings::default(), None),
            Ok(t) => match serde_json::from_str::<Settings>(&t) {
                Ok(mut s) => {
                    s.sanitize();
                    (s, None)
                }
                Err(e) => (Settings::default(), Some(format!("{} not read ({e}); defaults used", p.display()))),
            },
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), LibError> {
        let t = serde_json::to_string_pretty(self).map_err(|e| LibError(e.to_string()))?;
        write_atomic(&dir.join(SETTINGS_FILE), t.as_bytes())
    }

    pub fn sanitize(&mut self) {
        self.sdo_timeout_ms = self.sdo_timeout_ms.clamp(10, 60_000);
        self.scan_timeout_ms = self.scan_timeout_ms.clamp(10, 5_000);
        self.trace_capacity = self.trace_capacity.clamp(1_000, 5_000_000);
        if !(400.0..=10_000.0).contains(&self.window_width) {
            self.window_width = 1280.0;
        }
        if !(300.0..=10_000.0).contains(&self.window_height) {
            self.window_height = 800.0;
        }
    }
}
