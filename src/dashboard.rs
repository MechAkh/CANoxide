//! Dashboard: a few process values as large displays (number, bar, dial,
//! lamp). Each widget shows one signal of the signal store (`signals`), so
//! the dashboard and the Graph read the same samples; this module holds
//! the widget settings, the scaling, the thresholds, the stale rule and the
//! file. The `ui` draws it.

use crate::signals::SignalDef;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The dashboard file next to the Graph signals (`signals.json`).
pub const DASHBOARD_FILE: &str = "dashboard.json";
/// A value is stale when older than this many sample periods ...
pub const STALE_PERIODS: f64 = 5.0;
/// ... or than this many seconds, whichever is longer.
pub const STALE_MIN_S: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    /// A large number with the unit.
    #[default]
    Value,
    /// A horizontal bar from min to max.
    Bar,
    /// A round gauge from min to max.
    Dial,
    /// On when the value is not 0, or above a threshold.
    Lamp,
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 4] = [WidgetKind::Value, WidgetKind::Bar, WidgetKind::Dial, WidgetKind::Lamp];

    pub fn name(self) -> &'static str {
        match self {
            WidgetKind::Value => "Value",
            WidgetKind::Bar => "Bar",
            WidgetKind::Dial => "Dial",
            WidgetKind::Lamp => "Lamp",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl WidgetSize {
    pub const ALL: [WidgetSize; 3] = [WidgetSize::Small, WidgetSize::Medium, WidgetSize::Large];

    pub fn name(self) -> &'static str {
        match self {
            WidgetSize::Small => "Small",
            WidgetSize::Medium => "Medium",
            WidgetSize::Large => "Large",
        }
    }

    /// The size of the widget card in points (width, height).
    pub fn card(self) -> (f32, f32) {
        match self {
            WidgetSize::Small => (170.0, 130.0),
            WidgetSize::Medium => (240.0, 180.0),
            WidgetSize::Large => (340.0, 250.0),
        }
    }
}

/// The level of a value against the thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warning,
    Alarm,
}

fn default_max() -> f64 {
    100.0
}

fn default_decimals() -> u8 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    pub title: String,
    #[serde(default)]
    pub kind: WidgetKind,
    /// The signal (source, scale, offset, unit) as the Graph defines it.
    pub signal: SignalDef,
    #[serde(default)]
    pub min: f64,
    #[serde(default = "default_max")]
    pub max: f64,
    #[serde(default = "default_decimals")]
    pub decimals: u8,
    /// Warning at or above this value.
    #[serde(default)]
    pub warn: Option<f64>,
    /// Alarm at or above this value.
    #[serde(default)]
    pub alarm: Option<f64>,
    /// Lamp: on above this value (None: on when not 0).
    #[serde(default)]
    pub lamp_threshold: Option<f64>,
    #[serde(default)]
    pub size: WidgetSize,
    /// The signal in the store while the widget is shown (not saved).
    #[serde(skip)]
    pub signal_id: Option<u64>,
}

impl Widget {
    pub fn new(signal: SignalDef) -> Widget {
        Widget {
            title: signal.name.clone(),
            kind: WidgetKind::Value,
            signal,
            min: 0.0,
            max: 100.0,
            decimals: 1,
            warn: None,
            alarm: None,
            lamp_threshold: None,
            size: WidgetSize::Medium,
            signal_id: None,
        }
    }

    /// The place of a value between min and max, 0..=1 (clamped; 0 when
    /// min and max are equal).
    pub fn fraction(&self, v: f64) -> f64 {
        let span = self.max - self.min;
        if !span.is_finite() || span == 0.0 || !v.is_finite() {
            return 0.0;
        }
        ((v - self.min) / span).clamp(0.0, 1.0)
    }

    /// Alarm at or above `alarm`, else warning at or above `warn`.
    pub fn level(&self, v: f64) -> Level {
        if self.alarm.is_some_and(|a| v >= a) {
            Level::Alarm
        } else if self.warn.is_some_and(|w| v >= w) {
            Level::Warning
        } else {
            Level::Normal
        }
    }

    /// The lamp is on: above the threshold, or not 0 without one.
    pub fn lamp_on(&self, v: f64) -> bool {
        match self.lamp_threshold {
            Some(t) => v > t,
            None => v != 0.0,
        }
    }

    /// The value with the decimals of the widget.
    pub fn format(&self, v: f64) -> String {
        format!("{v:.*}", self.decimals.min(9) as usize)
    }
}

/// The age after which a value counts as stale: 5 sample periods, at least
/// 2 s.
pub fn stale_limit_s(period_s: Option<f64>) -> f64 {
    (STALE_PERIODS * period_s.unwrap_or(0.0)).max(STALE_MIN_S)
}

/// The value of time `t_s` is stale at `now_s`.
pub fn is_stale(t_s: f64, now_s: f64, period_s: Option<f64>) -> bool {
    now_s - t_s > stale_limit_s(period_s)
}

/// "3.4 s" / "2 min 5 s" for the age of a value.
pub fn age_text(age_s: f64) -> String {
    let a = age_s.max(0.0);
    if a < 60.0 {
        format!("{a:.1} s")
    } else if a < 3600.0 {
        format!("{} min {} s", (a / 60.0) as u64, (a % 60.0) as u64)
    } else {
        format!("{} h {} min", (a / 3600.0) as u64, ((a % 3600.0) / 60.0) as u64)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    #[serde(default)]
    pub widgets: Vec<Widget>,
}

impl Dashboard {
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let s = serde_json::to_vec_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        crate::library::write_atomic(path, &s).map_err(|e| std::io::Error::other(e.0))
    }

    /// Load the dashboard; a missing file (a data folder of an older
    /// version) gives an empty dashboard.
    pub fn load(path: &Path) -> std::io::Result<Dashboard> {
        let b = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Dashboard::default()),
            Err(e) => return Err(e),
        };
        serde_json::from_slice(&b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::{SignalSource, SignalStore};

    fn sig() -> SignalDef {
        SignalDef::new("Speed", SignalSource::SdoPoll { node: 1, index: 0x606C, sub: 0, period_ms: 200, data_type: None })
    }

    #[test]
    fn scaling() {
        let mut w = Widget::new(sig());
        w.min = -50.0;
        w.max = 150.0;
        assert_eq!(w.fraction(-50.0), 0.0);
        assert_eq!(w.fraction(50.0), 0.5);
        assert_eq!(w.fraction(150.0), 1.0);
        assert_eq!(w.fraction(500.0), 1.0, "clamped above");
        assert_eq!(w.fraction(-500.0), 0.0, "clamped below");
        w.max = w.min;
        assert_eq!(w.fraction(3.0), 0.0, "no span");
        w.decimals = 2;
        assert_eq!(w.format(1.23456), "1.23");
        w.decimals = 0;
        assert_eq!(w.format(9.6), "10");
    }

    #[test]
    fn thresholds_and_lamp() {
        let mut w = Widget::new(sig());
        assert_eq!(w.level(1e9), Level::Normal, "no thresholds");
        w.warn = Some(70.0);
        w.alarm = Some(90.0);
        assert_eq!(w.level(69.9), Level::Normal);
        assert_eq!(w.level(70.0), Level::Warning);
        assert_eq!(w.level(90.0), Level::Alarm);
        w.warn = None;
        assert_eq!(w.level(80.0), Level::Normal);
        assert!(!w.lamp_on(0.0) && w.lamp_on(-1.0) && w.lamp_on(0.5));
        w.lamp_threshold = Some(10.0);
        assert!(!w.lamp_on(10.0) && w.lamp_on(10.5));
    }

    #[test]
    fn stale_detection() {
        // 200 ms period: 5 periods = 1 s, so the 2 s floor applies.
        assert_eq!(stale_limit_s(Some(0.2)), 2.0);
        assert_eq!(stale_limit_s(Some(1.0)), 5.0);
        assert_eq!(stale_limit_s(None), 2.0);
        assert!(!is_stale(10.0, 11.9, Some(0.2)));
        assert!(is_stale(10.0, 12.1, Some(0.2)));
        assert!(!is_stale(10.0, 14.9, Some(1.0)));
        assert!(is_stale(10.0, 15.1, Some(1.0)));
        assert_eq!(age_text(3.44), "3.4 s");
        assert_eq!(age_text(125.0), "2 min 5 s");
    }

    #[test]
    fn signal_shared_with_the_graph_is_polled_once() {
        let mut s = SignalStore::new(100);
        let g = s.add(sig());
        let mut faster = sig();
        if let SignalSource::SdoPoll { period_ms, .. } = &mut faster.source {
            *period_ms = 100;
        }
        let d = s.acquire_dash(&faster);
        assert_eq!(g, d, "the same signal");
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.polls_due(0).len(), 1, "one poll");
        assert!(matches!(s.get(g).unwrap().def.source, SignalSource::SdoPoll { period_ms: 100, .. }), "the shorter period wins");
        // Removed from the Graph: still sampled for the dashboard, not saved.
        assert!(s.remove(g));
        assert_eq!(s.list().len(), 1);
        assert!(s.defs().is_empty());
        assert_eq!(s.graph_signals().count(), 0);
        // Added to the Graph again: the same signal.
        assert_eq!(s.add(sig()), g);
        s.release_dash(g);
        assert_eq!(s.list().len(), 1, "the Graph still shows it");
        s.remove(g);
        assert!(s.list().is_empty());
        // A dashboard-only signal goes with its last widget.
        let d = s.acquire_dash(&sig());
        let d2 = s.acquire_dash(&sig());
        assert_eq!(d, d2);
        s.release_dash(d);
        assert_eq!(s.list().len(), 1);
        s.release_dash(d);
        assert!(s.list().is_empty());
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let p = std::env::temp_dir().join(format!("canoxide_dash_{name}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn session_round_trip() {
        let dir = temp("rt");
        let path = dir.join(DASHBOARD_FILE);
        let mut w = Widget::new(sig());
        w.kind = WidgetKind::Dial;
        w.warn = Some(1.0);
        w.alarm = Some(2.0);
        w.size = WidgetSize::Large;
        w.signal_id = Some(7);
        let mut lamp = Widget::new(SignalDef::new("Fault", SignalSource::Pdo { node: 1, index: 0x6000, sub: 1 }));
        lamp.kind = WidgetKind::Lamp;
        lamp.lamp_threshold = Some(3.0);
        let d = Dashboard { widgets: vec![w.clone(), lamp.clone()] };
        d.save(&path).unwrap();
        let back = Dashboard::load(&path).unwrap();
        w.signal_id = None;
        assert_eq!(back.widgets, vec![w, lamp]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_without_a_dashboard() {
        let dir = temp("old");
        // A data folder of an older version: Graph signals, no dashboard.
        let mut s = SignalStore::new(100);
        s.add(sig());
        s.save_defs(&dir.join(crate::signals::SIGNALS_FILE)).unwrap();
        let d = Dashboard::load(&dir.join(DASHBOARD_FILE)).unwrap();
        assert!(d.widgets.is_empty());
        let mut s2 = SignalStore::new(100);
        assert_eq!(s2.load_defs(&dir.join(crate::signals::SIGNALS_FILE)).unwrap(), 1);
        assert_eq!(s2.graph_signals().count(), 1);
        // A widget written with only the required fields loads with defaults.
        std::fs::write(dir.join(DASHBOARD_FILE), r#"{"widgets":[{"title":"T","signal":{"name":"a","scale":1.0,"offset":0.0,"source":{"kind":"pdo","node":1,"index":24576,"sub":1}}}]}"#).unwrap();
        let d = Dashboard::load(&dir.join(DASHBOARD_FILE)).unwrap();
        assert_eq!(d.widgets[0].kind, WidgetKind::Value);
        assert_eq!(d.widgets[0].max, 100.0);
        assert_eq!(d.widgets[0].size, WidgetSize::Medium);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
