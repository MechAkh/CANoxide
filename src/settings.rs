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

/// Colour theme of the GUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    /// Follow the Windows setting.
    #[default]
    System,
    Dark,
    Light,
}

/// How the trace shows time stamps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimestampFormat {
    /// Seconds since the connection was opened.
    #[default]
    Relative,
    /// Time since the previous row, in ms.
    Delta,
    /// Local wall-clock time of day.
    WallClock,
}

/// The trace view shown first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DataView {
    /// One row per frame, oldest first.
    #[default]
    Chronological,
    /// One row per COB-ID.
    Fixed,
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
    // ---- v0.2 ----
    pub theme: Theme,
    /// GUI zoom factor, 0.5..3.0.
    pub ui_scale: f32,
    /// SDO block transfer: block size the client offers, 1..127.
    pub sdo_block_size: u8,
    /// Node-ID range of the scan.
    pub scan_first: u8,
    pub scan_last: u8,
    /// Points per signal ring buffer.
    pub signal_capacity: usize,
    pub timestamp_format: TimestampFormat,
    pub default_data_view: DataView,
    /// Consecutive missing PDOs that end the PDO read of one direction.
    pub pdo_scan_stop: u16,
    /// Ask before an SDO write (object dictionary, manual SDO, from file).
    pub confirm_before_write: bool,
    /// LSS answer timeout.
    pub lss_timeout_ms: u64,
    // ---- v0.2 UI ----
    /// Body text size offset in points, -2..4 (body 14 + offset).
    pub font_offset: i8,
    pub row_density: RowDensity,
    /// Alternate row background in tables.
    pub zebra: bool,
    /// Chips and pills outlined instead of filled.
    pub reduce_colour: bool,
    /// Connect with the saved adapter when the app starts (opens the
    /// channel only; nothing is sent).
    pub connect_on_start: bool,
    /// Transfer mode of the object dictionary view.
    pub sdo_transfer_mode: crate::canopen::sdo::TransferMode,
    /// Default switch delay of "Activate bit rate".
    pub lss_switch_delay_ms: u16,
    /// Trace time stamps in microseconds (false: milliseconds).
    pub timestamp_us: bool,
    pub trace_colour: TraceColour,
    /// New trace tabs start with auto-scroll on.
    pub trace_autoscroll: bool,
    /// Graph time window in seconds.
    pub graph_window_s: f32,
    pub graph_line_width: f32,
    pub graph_cursor_readout: bool,
    /// Ask before NMT to all nodes and "set heartbeat of all nodes".
    pub confirm_nmt_all: bool,
    /// Ask before a PDO apply / enable / disable.
    pub confirm_pdo_apply: bool,
    /// Ask before an LSS node-ID or bit rate change or Activate bit rate.
    pub confirm_lss: bool,
    /// Ask before a DCF restore (not before a dry run).
    pub confirm_dcf_restore: bool,
    // ---- v0.3 ----
    /// Messages view: seconds over which the change tint of a data byte
    /// fades, 0.2..10.
    pub msg_fade_s: f32,
    /// Messages view: no tint on a byte that changes in nearly every frame
    /// (for example a counter).
    pub msg_mute_noisy: bool,
}

/// Table row height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RowDensity {
    /// 22 px rows.
    #[default]
    Compact,
    /// 28 px rows.
    Comfortable,
}

impl RowDensity {
    pub fn row_height(self) -> f32 {
        match self {
            RowDensity::Compact => 22.0,
            RowDensity::Comfortable => 28.0,
        }
    }
}

/// How the trace colours message types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TraceColour {
    Off,
    #[default]
    Chips,
    ChipsAndTint,
}

impl TraceColour {
    pub const ALL: [TraceColour; 3] = [TraceColour::Off, TraceColour::Chips, TraceColour::ChipsAndTint];
    pub fn label(self) -> &'static str {
        match self {
            TraceColour::Off => "Off",
            TraceColour::Chips => "Chips",
            TraceColour::ChipsAndTint => "Chips + row tint",
        }
    }
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
            theme: Theme::System,
            ui_scale: 1.0,
            sdo_block_size: 127,
            scan_first: 1,
            scan_last: 127,
            signal_capacity: crate::signals::DEFAULT_SIGNAL_CAPACITY,
            timestamp_format: TimestampFormat::Relative,
            default_data_view: DataView::Chronological,
            pdo_scan_stop: crate::canopen::pdo::DEFAULT_PDO_SCAN_STOP,
            confirm_before_write: true,
            lss_timeout_ms: crate::canopen::lss::DEFAULT_LSS_TIMEOUT_MS,
            font_offset: 0,
            row_density: RowDensity::Compact,
            zebra: true,
            reduce_colour: false,
            connect_on_start: false,
            sdo_transfer_mode: crate::canopen::sdo::TransferMode::Auto,
            lss_switch_delay_ms: 100,
            timestamp_us: true,
            trace_colour: TraceColour::Chips,
            trace_autoscroll: true,
            graph_window_s: 10.0,
            graph_line_width: 1.5,
            graph_cursor_readout: true,
            confirm_nmt_all: true,
            confirm_pdo_apply: true,
            confirm_lss: true,
            confirm_dcf_restore: true,
            msg_fade_s: 2.0,
            msg_mute_noisy: false,
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
        if !(0.5..=3.0).contains(&self.ui_scale) {
            self.ui_scale = 1.0;
        }
        self.sdo_block_size = self.sdo_block_size.clamp(1, 127);
        self.scan_first = self.scan_first.clamp(1, 127);
        self.scan_last = self.scan_last.clamp(self.scan_first, 127);
        self.signal_capacity = self.signal_capacity.clamp(100, 1_000_000);
        self.pdo_scan_stop = self.pdo_scan_stop.clamp(1, crate::canopen::pdo::MAX_PDOS);
        self.lss_timeout_ms = self.lss_timeout_ms.clamp(10, 5_000);
        self.font_offset = self.font_offset.clamp(-2, 4);
        if !(1.0..=3600.0).contains(&self.graph_window_s) {
            self.graph_window_s = 10.0;
        }
        if !(0.5..=5.0).contains(&self.graph_line_width) {
            self.graph_line_width = 1.5;
        }
        if !(0.2..=10.0).contains(&self.msg_fade_s) {
            self.msg_fade_s = 2.0;
        }
    }
}
