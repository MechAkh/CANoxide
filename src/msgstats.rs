//! Per-identifier statistics for the Messages view (one row per COB-ID):
//! count, newest data, smoothed period, per-byte change time / direction /
//! rate and per-bit toggle counts. Fed from the same frame stream as the
//! trace (also while a trace tab is paused); the memory is bounded by
//! `MAX_IDS`. The UI only draws from it.

use crate::can::{CanFrame, CanId, Dir};
use crate::canopen::decode::{Decoded, MsgClass};
use std::collections::HashMap;

/// Most identifiers kept; frames of further identifiers are counted in
/// `MsgStats::ignored` only.
pub const MAX_IDS: usize = 2048;
/// A byte that changed in at least this share of the compared frames is
/// "noisy" (for example a counter).
pub const NOISY_RATE: f32 = 0.9;
/// A byte is judged noisy only after this many compared frames.
pub const NOISY_MIN_FRAMES: u64 = 10;
/// Weight of a new interval in the smoothed period.
const PERIOD_ALPHA: f64 = 0.2;

/// The identifier of a row: an 11-bit and a 29-bit frame with the same
/// number are different rows (the shared `CanId`; standard rows sort
/// before extended rows).
pub type MsgKey = CanId;

/// The direction of the last change of a byte value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ByteStat {
    /// Time stamp of the frame with the last change.
    pub last_change_us: Option<u64>,
    pub dir: Option<Change>,
    /// Frames in which this byte changed since the last reset.
    pub changes: u64,
}

#[derive(Clone, Debug)]
pub struct MsgEntry {
    pub key: MsgKey,
    pub class: MsgClass,
    pub node: Option<u8>,
    /// Type label of the decoder ("TPDO1", "SDO resp", "HB", ...).
    pub label: String,
    /// Frames since the last reset.
    pub count: u64,
    pub rtr: bool,
    pub dlc: u8,
    /// Data of the newest frame (bytes past the DLC are zero).
    pub data: [u8; 8],
    pub dir: Dir,
    pub last_us: u64,
    /// Smoothed interval between frames in microseconds.
    pub period_us: Option<f64>,
    pub bytes: [ByteStat; 8],
    /// Time stamp of the frame with the last change of bit `k` (byte
    /// `k / 8`, bit `k % 8`): the change flash of the bit view. Set per
    /// frame, so a 0 -> 1 -> 0 between two repaints is kept.
    pub bit_change_us: [Option<u64>; 64],
    /// Toggles of bit `k` (byte `k / 8`, bit `k % 8`) since the last reset.
    pub bit_toggles: [u32; 64],
    /// Frames that were compared with a previous frame since the last
    /// reset (the base of the change and toggle rates).
    pub compared: u64,
    /// Lower-case text for the filter (COB-ID hex, type, node).
    search: String,
}

impl MsgEntry {
    fn new(f: &CanFrame, d: &Decoded) -> MsgEntry {
        let node = d.node.map(|n| format!(" node {n}")).unwrap_or_default();
        let search = format!("{} {:x} 0x{:x} {}{node}", CanId::of(f), f.id, f.id, d.label).to_lowercase();
        MsgEntry {
            key: MsgKey::of(f),
            class: d.class,
            node: d.node,
            label: d.label.clone(),
            count: 1,
            rtr: f.rtr,
            dlc: f.dlc,
            data: payload8(f),
            dir: f.dir,
            last_us: f.timestamp_us,
            period_us: None,
            bytes: [ByteStat::default(); 8],
            bit_change_us: [None; 64],
            bit_toggles: [0; 64],
            compared: 0,
            search,
        }
    }

    fn update(&mut self, f: &CanFrame, d: &Decoded) {
        let dt = f.timestamp_us.saturating_sub(self.last_us);
        if self.count > 0 && dt > 0 {
            let dt = dt as f64;
            self.period_us = Some(match self.period_us {
                None => dt,
                Some(p) => p + PERIOD_ALPHA * (dt - p),
            });
        }
        let new = payload8(f);
        if self.count > 0 && !f.rtr && !self.rtr {
            let n = (self.dlc.min(8)).min(f.dlc.min(8)) as usize;
            for i in 0..n {
                let (a, b) = (self.data[i], new[i]);
                if a == b {
                    continue;
                }
                let s = &mut self.bytes[i];
                s.last_change_us = Some(f.timestamp_us);
                s.dir = Some(if b > a { Change::Up } else { Change::Down });
                s.changes += 1;
                let mut x = a ^ b;
                while x != 0 {
                    let bit = x.trailing_zeros() as usize;
                    let t = &mut self.bit_toggles[i * 8 + bit];
                    *t = t.saturating_add(1);
                    self.bit_change_us[i * 8 + bit] = Some(f.timestamp_us);
                    x &= x - 1;
                }
            }
            self.compared += 1;
        }
        // The decoder may know more now (a PDO configuration was read).
        if self.class != d.class || self.node != d.node || self.label != d.label {
            self.class = d.class;
            self.node = d.node;
            self.label = d.label.clone();
        }
        self.count += 1;
        self.rtr = f.rtr;
        self.dlc = f.dlc;
        self.data = new;
        self.dir = f.dir;
        self.last_us = f.timestamp_us;
    }

    /// The share of compared frames in which byte `i` changed.
    pub fn change_rate(&self, i: usize) -> f32 {
        if self.compared == 0 || i >= 8 {
            0.0
        } else {
            self.bytes[i].changes as f32 / self.compared as f32
        }
    }

    /// The share of compared frames in which bit `k` (0..64) toggled.
    pub fn toggle_rate(&self, k: usize) -> f32 {
        if self.compared == 0 || k >= 64 {
            0.0
        } else {
            self.bit_toggles[k] as f32 / self.compared as f32
        }
    }

    /// Byte `i` changes in nearly every frame (see `NOISY_RATE`).
    pub fn is_noisy(&self, i: usize) -> bool {
        self.compared >= NOISY_MIN_FRAMES && self.change_rate(i) >= NOISY_RATE
    }

    /// The bit value of bit `k` of the newest data.
    pub fn bit(&self, k: usize) -> bool {
        k < 64 && self.data[k / 8] >> (k % 8) & 1 == 1
    }

    /// The filter text (already lower case) matches this row.
    pub fn matches(&self, filter_lower: &str) -> bool {
        filter_lower.is_empty() || self.search.contains(filter_lower)
    }

    /// Zero the statistics; the newest data stays shown.
    fn reset_stats(&mut self) {
        self.count = 0;
        self.period_us = None;
        self.bytes = [ByteStat::default(); 8];
        self.bit_change_us = [None; 64];
        self.bit_toggles = [0; 64];
        self.compared = 0;
    }

    /// The change flash of bit `k` at the time `now_us` (see
    /// [`flash_strength`]).
    pub fn bit_flash(&self, k: usize, now_us: u64) -> f32 {
        if k >= 64 {
            return 0.0;
        }
        flash_strength(self.bit_change_us[k], now_us)
    }

    /// The change flash of byte `i`: that of its last changed bit.
    pub fn byte_flash(&self, i: usize, now_us: u64) -> f32 {
        (0..8).map(|b| self.bit_flash(i * 8 + b, now_us)).fold(0.0, f32::max)
    }
}

/// The change flash of the bit view holds at full strength this long after
/// the last change of a bit ...
pub const FLASH_HOLD_US: u64 = 1_000_000;
/// ... and then fades out over this time.
pub const FLASH_FADE_US: u64 = 1_000_000;

/// The strength (1 = full, 0 = none) of the change flash of a bit that
/// changed at `change_us`, at the time `now_us`: full for
/// [`FLASH_HOLD_US`], then a linear fade over [`FLASH_FADE_US`].
pub fn flash_strength(change_us: Option<u64>, now_us: u64) -> f32 {
    let Some(c) = change_us else { return 0.0 };
    let age = now_us.saturating_sub(c);
    if age <= FLASH_HOLD_US {
        1.0
    } else {
        (1.0 - (age - FLASH_HOLD_US) as f64 / FLASH_FADE_US as f64).clamp(0.0, 1.0) as f32
    }
}

fn payload8(f: &CanFrame) -> [u8; 8] {
    let mut d = [0u8; 8];
    let p = f.payload();
    d[..p.len()].copy_from_slice(p);
    d
}

/// The statistics of all identifiers.
#[derive(Clone, Debug, Default)]
pub struct MsgStats {
    map: HashMap<MsgKey, MsgEntry>,
    /// Frames of identifiers past `MAX_IDS` since the last reset.
    ignored: u64,
    /// The newest time stamp seen.
    latest_us: u64,
    /// Bumped on each change (a frame or a reset), so that a view can tell
    /// that it must sort again.
    generation: u64,
}

impl MsgStats {
    pub fn new() -> MsgStats {
        MsgStats::default()
    }

    /// Count one frame with its decoding (the trace's decoding of it).
    pub fn on_frame(&mut self, f: &CanFrame, d: &Decoded) {
        self.latest_us = self.latest_us.max(f.timestamp_us);
        self.generation = self.generation.wrapping_add(1);
        let key = MsgKey::of(f);
        if let Some(e) = self.map.get_mut(&key) {
            e.update(f, d);
        } else if self.map.len() < MAX_IDS {
            self.map.insert(key, MsgEntry::new(f, d));
        } else {
            self.ignored += 1;
        }
    }

    /// Forget every identifier (reset all, and on connect).
    pub fn reset(&mut self) {
        self.map.clear();
        self.ignored = 0;
        self.latest_us = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Zero the statistics of one identifier (its newest data stays).
    pub fn reset_id(&mut self, key: MsgKey) {
        if let Some(e) = self.map.get_mut(&key) {
            e.reset_stats();
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub fn get(&self, key: MsgKey) -> Option<&MsgEntry> {
        self.map.get(&key)
    }

    pub fn entries(&self) -> impl Iterator<Item = &MsgEntry> {
        self.map.values()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn ignored(&self) -> u64 {
        self.ignored
    }

    pub fn latest_us(&self) -> u64 {
        self.latest_us
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// The strength (1 = full, 0 = none) of the change tint of a byte that
/// changed at `change_us`, at the time `now_us`, with the fade time
/// `fade_us`.
pub fn tint_strength(change_us: Option<u64>, now_us: u64, fade_us: u64) -> f32 {
    match change_us {
        Some(c) if fade_us > 0 => {
            let age = now_us.saturating_sub(c) as f64;
            (1.0 - age / fade_us as f64).clamp(0.0, 1.0) as f32
        }
        _ => 0.0,
    }
}
