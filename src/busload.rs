//! Bus load from frame bit lengths and the bitrate.
//!
//! Each frame counts with its worst-case length: the stuffable part (SOF to
//! CRC) gets one stuff bit per 4 bits after the first, plus the fixed tail
//! (CRC delimiter, ACK, EOF) and the 3-bit intermission.

use crate::can::CanFrame;
use serde::{Deserialize, Serialize};

/// Worst-case bits on the bus of one frame, intermission included.
pub fn frame_bits(f: &CanFrame) -> u32 {
    let n = if f.rtr { 0 } else { f.dlc.min(8) as u32 * 8 };
    // Standard: SOF 1, ID 11, RTR 1, IDE 1, r0 1, DLC 4, data, CRC 15 = 34 + n.
    // Extended: SOF 1, ID 11, SRR 1, IDE 1, ID 18, RTR 1, r1 r0 2, DLC 4, data, CRC 15 = 54 + n.
    let stuffable = if f.extended { 54 + n } else { 34 + n };
    let stuff = (stuffable - 1) / 4;
    // CRC delimiter 1, ACK 2, EOF 7, intermission 3.
    stuffable + stuff + 13
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BusLoadStats {
    /// Load of the last full window, percent.
    pub current_pct: f64,
    /// Highest window load since the start, percent.
    pub peak_pct: f64,
    /// Load since the start, percent.
    pub average_pct: f64,
    /// Frames per second of the last window.
    pub frames_per_s: f64,
    pub total_frames: u64,
}

/// Measures the load over windows of `window_us` (default 1 s) by frame
/// time stamps, so the result does not depend on when it is polled.
#[derive(Clone, Debug)]
pub struct BusLoad {
    bitrate_bps: f64,
    window_us: u64,
    start_us: Option<u64>,
    win_start_us: u64,
    win_bits: u64,
    win_frames: u64,
    total_bits: u64,
    last_us: u64,
    stats: BusLoadStats,
}

impl BusLoad {
    pub fn new(bitrate_kbit: u32) -> BusLoad {
        BusLoad::with_window(bitrate_kbit, 1_000_000)
    }

    pub fn with_window(bitrate_kbit: u32, window_us: u64) -> BusLoad {
        BusLoad {
            bitrate_bps: bitrate_kbit.max(1) as f64 * 1000.0,
            window_us: window_us.max(1000),
            start_us: None,
            win_start_us: 0,
            win_bits: 0,
            win_frames: 0,
            total_bits: 0,
            last_us: 0,
            stats: BusLoadStats::default(),
        }
    }

    pub fn reset(&mut self) {
        *self = BusLoad::with_window((self.bitrate_bps / 1000.0) as u32, self.window_us);
    }

    /// Count a frame (sent or received) at its time stamp.
    pub fn add(&mut self, f: &CanFrame) {
        let t = f.timestamp_us;
        self.advance(t);
        let b = frame_bits(f) as u64;
        self.win_bits += b;
        self.win_frames += 1;
        self.total_bits += b;
        self.stats.total_frames += 1;
    }

    /// Close the windows that ended before `now_us`.
    pub fn advance(&mut self, now_us: u64) {
        let start = *self.start_us.get_or_insert_with(|| {
            self.win_start_us = now_us;
            now_us
        });
        while now_us >= self.win_start_us + self.window_us {
            let secs = self.window_us as f64 / 1e6;
            let pct = self.win_bits as f64 / (self.bitrate_bps * secs) * 100.0;
            self.stats.current_pct = pct;
            self.stats.frames_per_s = self.win_frames as f64 / secs;
            if pct > self.stats.peak_pct {
                self.stats.peak_pct = pct;
            }
            self.win_bits = 0;
            self.win_frames = 0;
            self.win_start_us += self.window_us;
        }
        self.last_us = self.last_us.max(now_us);
        let el = (self.win_start_us.saturating_sub(start)) as f64 / 1e6;
        if el > 0.0 {
            let closed_bits = self.total_bits - self.win_bits;
            self.stats.average_pct = closed_bits as f64 / (self.bitrate_bps * el) * 100.0;
        }
    }

    pub fn stats(&self) -> BusLoadStats {
        self.stats
    }
}
