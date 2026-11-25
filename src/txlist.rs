//! Transmit list: named frames, each with a trigger (manual, periodic, or
//! on reception of a COB-ID). The bus worker runs it; it stops on
//! disconnect and is refused in listen-only mode. An entry sends only while
//! it is started (`enabled`) or when it is fired by hand.

use crate::can::{CanFrame, Dir};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const TXLIST_FILE: &str = "txlist.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    Manual,
    Periodic { period_ms: u64 },
    /// Send when a frame with this identifier (number and format) is
    /// received.
    OnRx {
        cob_id: u32,
        #[serde(default)]
        extended: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxEntry {
    /// Unique key in the list.
    pub key: u64,
    pub name: String,
    pub id: u32,
    #[serde(default)]
    pub extended: bool,
    pub rtr: bool,
    pub dlc: u8,
    pub data: [u8; 8],
    pub trigger: Trigger,
    /// Periodic / on-rx entries send only while enabled (started).
    #[serde(default)]
    pub enabled: bool,
}

impl TxEntry {
    pub fn frame(&self) -> CanFrame {
        let mut f = CanFrame::new(self.id, &self.data[..(self.dlc.min(8) as usize)]);
        f.extended = self.extended;
        f.rtr = self.rtr;
        f.dlc = self.dlc.min(8);
        f
    }

    pub fn from_frame(key: u64, name: impl Into<String>, f: &CanFrame, trigger: Trigger) -> TxEntry {
        TxEntry { key, name: name.into(), id: f.id, extended: f.extended, rtr: f.rtr, dlc: f.dlc, data: f.data, trigger, enabled: false }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TxList {
    pub entries: Vec<TxEntry>,
    #[serde(skip)]
    due: Vec<(u64, u64)>,
}

impl TxList {
    pub fn new(entries: Vec<TxEntry>) -> TxList {
        TxList { entries, due: Vec::new() }
    }

    pub fn get(&self, key: u64) -> Option<&TxEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Insert or replace by key.
    pub fn upsert(&mut self, e: TxEntry) {
        self.due.retain(|d| d.0 != e.key);
        match self.entries.iter_mut().find(|x| x.key == e.key) {
            Some(x) => *x = e,
            None => self.entries.push(e),
        }
    }

    pub fn remove(&mut self, key: u64) {
        self.entries.retain(|e| e.key != key);
        self.due.retain(|d| d.0 != key);
    }

    /// A key not used yet.
    pub fn free_key(&self) -> u64 {
        self.entries.iter().map(|e| e.key).max().map(|k| k + 1).unwrap_or(1)
    }

    /// Start (enable) an entry; a periodic one sends at once.
    pub fn start(&mut self, key: u64, now_ms: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.key == key) {
            e.enabled = true;
            self.due.retain(|d| d.0 != key);
            self.due.push((key, now_ms));
        }
    }

    pub fn stop(&mut self, key: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.key == key) {
            e.enabled = false;
        }
        self.due.retain(|d| d.0 != key);
    }

    /// Stop every entry (disconnect).
    pub fn stop_all(&mut self) {
        for e in &mut self.entries {
            e.enabled = false;
        }
        self.due.clear();
    }

    pub fn any_running(&self) -> bool {
        self.entries.iter().any(|e| e.enabled && e.trigger != Trigger::Manual)
    }

    /// The frame of a manual send of one entry.
    pub fn fire(&self, key: u64) -> Option<CanFrame> {
        self.get(key).map(|e| e.frame())
    }

    /// Frames of the periodic entries that are due at `now_ms`.
    pub fn due(&mut self, now_ms: u64) -> Vec<CanFrame> {
        let mut out = Vec::new();
        for e in &self.entries {
            let Trigger::Periodic { period_ms } = e.trigger else { continue };
            if !e.enabled {
                continue;
            }
            let period = period_ms.max(1);
            let slot = match self.due.iter_mut().find(|d| d.0 == e.key) {
                Some(s) => s,
                None => {
                    self.due.push((e.key, now_ms));
                    self.due.last_mut().unwrap_or_else(|| unreachable!())
                }
            };
            if now_ms >= slot.1 {
                out.push(e.frame());
                slot.1 = if now_ms > slot.1 + period { now_ms + period } else { slot.1 + period };
            }
        }
        out
    }

    /// Frames of the enabled on-rx entries that a received frame triggers.
    /// Sent frames do not trigger (no loops).
    pub fn on_rx(&self, f: &CanFrame) -> Vec<CanFrame> {
        if f.dir != Dir::Rx {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.enabled && e.trigger == Trigger::OnRx { cob_id: f.id, extended: f.extended })
            .map(|e| e.frame())
            .collect()
    }

    /// Load from a JSON file; a missing file gives an empty list.
    pub fn load(path: &Path) -> std::io::Result<TxList> {
        match std::fs::read(path) {
            Ok(b) => {
                let mut l: TxList = serde_json::from_slice(&b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
                l.stop_all();
                Ok(l)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TxList::default()),
            Err(e) => Err(e),
        }
    }

    /// Save as JSON (all entries stopped in the file).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut l = self.clone();
        l.stop_all();
        let s = serde_json::to_vec_pretty(&l).map_err(|e| std::io::Error::other(e.to_string()))?;
        crate::library::write_atomic(path, &s).map_err(|e| std::io::Error::other(e.0))
    }
}
