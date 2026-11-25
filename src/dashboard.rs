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
/// The grid that the widget rectangles snap to (points).
pub const GRID: f32 = 10.0;
/// The gap between widgets that Arrange and a new widget keep (points).
pub const GAP: f32 = 10.0;
/// The smallest and the largest widget card (points).
pub const MIN_W: f32 = 120.0;
pub const MIN_H: f32 = 90.0;
pub const MAX_W: f32 = 800.0;
pub const MAX_H: f32 = 600.0;
/// The width that a file of an older version (no rectangles) is arranged in.
pub const DEFAULT_ARRANGE_WIDTH: f32 = 1000.0;

/// A value on the grid.
pub fn snap(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    (v / GRID).round() * GRID
}

/// The place and the size of a widget card in points, relative to the
/// canvas origin.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl WidgetRect {
    /// On the grid, x and y at least 0, the size between the minimum and
    /// the maximum.
    pub fn snapped(self) -> WidgetRect {
        WidgetRect { x: snap(self.x).max(0.0), y: snap(self.y).max(0.0), w: snap(self.w).clamp(MIN_W, MAX_W), h: snap(self.h).clamp(MIN_H, MAX_H) }
    }

    /// The two cards are closer than `gap` (or overlap).
    pub fn near(&self, o: &WidgetRect, gap: f32) -> bool {
        self.x < o.x + o.w + gap && o.x < self.x + self.w + gap && self.y < o.y + o.h + gap && o.y < self.y + self.h + gap
    }
}

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

    /// The size of the widget card in points (width, height): the preset
    /// of the widget dialog, and the size of a widget from a file of an
    /// older version.
    pub fn card(self) -> (f32, f32) {
        match self {
            WidgetSize::Small => (170.0, 130.0),
            WidgetSize::Medium => (240.0, 180.0),
            WidgetSize::Large => (340.0, 250.0),
        }
    }

    /// The preset whose card has this size.
    pub fn of_card(w: f32, h: f32) -> Option<WidgetSize> {
        WidgetSize::ALL.into_iter().find(|s| s.card() == (w, h))
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
    /// The size preset (a file of an older version has only this).
    #[serde(default)]
    pub size: WidgetSize,
    /// The place and the size on the canvas (None: not placed yet, or a
    /// file of an older version).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<WidgetRect>,
    /// The signal in the store while the widget is shown (not saved).
    #[serde(skip)]
    pub signal_id: Option<u64>,
    /// A number for the view that stays the same while the widget moves in
    /// the draw order (not saved).
    #[serde(skip)]
    pub uid: u64,
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
            rect: None,
            signal_id: None,
            uid: 0,
        }
    }

    /// The rectangle of the widget; one that is not placed yet has the
    /// card size of its preset at 0, 0.
    pub fn rect(&self) -> WidgetRect {
        self.rect.unwrap_or_else(|| {
            let (w, h) = self.size.card();
            WidgetRect { x: 0.0, y: 0.0, w, h }.snapped()
        })
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
    /// Place all widgets left to right in rows that fit `width`, gap 10,
    /// each with its size, top-aligned per row, in the order of the list.
    pub fn arrange(&mut self, width: f32) {
        let (mut x, mut y, mut row_h) = (0.0f32, 0.0f32, 0.0f32);
        for w in &mut self.widgets {
            let mut r = w.rect().snapped();
            if x > 0.0 && x + r.w > width {
                x = 0.0;
                y += row_h + GAP;
                row_h = 0.0;
            }
            r.x = x;
            r.y = y;
            w.rect = Some(r);
            x += r.w + GAP;
            row_h = row_h.max(r.h);
        }
    }

    /// The lowest edge of all cards (0 without widgets).
    pub fn bottom(&self) -> f32 {
        self.widgets.iter().map(|w| w.rect().y + w.rect().h).fold(0.0, f32::max)
    }

    /// The first place for a card of `w` x `h`, scanning in rows of the
    /// grid (top to bottom, each row left to right) within `width`, where
    /// it keeps the gap to every card; else under all cards.
    pub fn free_position(&self, w: f32, h: f32, width: f32) -> (f32, f32) {
        let rects: Vec<WidgetRect> = self.widgets.iter().filter(|x| x.rect.is_some()).map(|x| x.rect()).collect();
        let bottom = rects.iter().map(|r| r.y + r.h).fold(0.0, f32::max);
        let x_max = ((width - w).max(0.0) / GRID).floor() as i64;
        let y_max = (bottom / GRID).ceil() as i64;
        for yi in 0..=y_max {
            for xi in 0..=x_max {
                let c = WidgetRect { x: xi as f32 * GRID, y: yi as f32 * GRID, w, h };
                if !rects.iter().any(|r| c.near(r, GAP)) {
                    return (c.x, c.y);
                }
            }
        }
        (0.0, snap(bottom + GAP))
    }

    /// After a load: every rectangle on the grid; a file without rectangles
    /// (an older version) gets the preset sizes, arranged at the default
    /// width; a widget without one in a newer file goes to a free place.
    pub fn fix_layout(&mut self) {
        if self.widgets.iter().all(|w| w.rect.is_none()) {
            self.arrange(DEFAULT_ARRANGE_WIDTH);
            return;
        }
        for w in &mut self.widgets {
            if let Some(r) = w.rect {
                w.rect = Some(r.snapped());
            }
        }
        for i in 0..self.widgets.len() {
            if self.widgets[i].rect.is_none() {
                let r = self.widgets[i].rect();
                let (x, y) = self.free_position(r.w, r.h, DEFAULT_ARRANGE_WIDTH);
                self.widgets[i].rect = Some(WidgetRect { x, y, ..r });
            }
        }
    }

    /// Move widget `i` to the end of the list, so that it is drawn on top.
    /// Returns its new index.
    pub fn bring_to_front(&mut self, i: usize) -> usize {
        if i < self.widgets.len() {
            let w = self.widgets.remove(i);
            self.widgets.push(w);
        }
        self.widgets.len().saturating_sub(1)
    }

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
        let mut d: Dashboard = serde_json::from_slice(&b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        d.fix_layout();
        Ok(d)
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
        lamp.rect = Some(WidgetRect { x: 500.0, y: 30.0, w: 130.0, h: 90.0 });
        w.rect = Some(WidgetRect { x: 0.0, y: 0.0, w: 340.0, h: 250.0 });
        let d = Dashboard { widgets: vec![w.clone(), lamp.clone()] };
        d.save(&path).unwrap();
        let back = Dashboard::load(&path).unwrap();
        w.signal_id = None;
        assert_eq!(back.widgets, vec![w, lamp]);
        // A second save / load gives the same file.
        let first = std::fs::read(&path).unwrap();
        back.save(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), first, "round trip");
        assert_eq!(Dashboard::load(&path).unwrap(), back);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> WidgetRect {
        WidgetRect { x, y, w, h }
    }

    #[test]
    fn snap_and_limits() {
        assert_eq!(snap(14.9), 10.0);
        assert_eq!(snap(15.0), 20.0);
        assert_eq!(snap(-4.0), 0.0);
        assert_eq!(rect(-33.0, 47.0, 20.0, 2000.0).snapped(), rect(0.0, 50.0, MIN_W, MAX_H));
        assert_eq!(rect(12.0, 0.0, 999.0, 10.0).snapped(), rect(10.0, 0.0, MAX_W, MIN_H));
        assert_eq!(rect(0.0, 0.0, 243.0, 177.0).snapped(), rect(0.0, 0.0, 240.0, 180.0));
        assert_eq!(WidgetSize::of_card(240.0, 180.0), Some(WidgetSize::Medium));
        assert_eq!(WidgetSize::of_card(250.0, 180.0), None);
    }

    #[test]
    fn arrange_in_rows() {
        let mut d = Dashboard::default();
        for s in [WidgetSize::Medium, WidgetSize::Large, WidgetSize::Small, WidgetSize::Medium] {
            let mut w = Widget::new(sig());
            w.size = s;
            d.widgets.push(w);
        }
        // 240 + 10 + 340 + 10 + 170 = 770: three fit in 800, the fourth wraps
        // below the highest card of the row (250).
        d.arrange(800.0);
        let r: Vec<WidgetRect> = d.widgets.iter().map(|w| w.rect.unwrap()).collect();
        assert_eq!(r[0], rect(0.0, 0.0, 240.0, 180.0));
        assert_eq!(r[1], rect(250.0, 0.0, 340.0, 250.0));
        assert_eq!(r[2], rect(600.0, 0.0, 170.0, 130.0));
        assert_eq!(r[3], rect(0.0, 260.0, 240.0, 180.0));
        // Narrower than one card: one per row.
        d.arrange(100.0);
        let ys: Vec<f32> = d.widgets.iter().map(|w| w.rect.unwrap().y).collect();
        assert_eq!(ys, vec![0.0, 190.0, 450.0, 590.0]);
        assert!(d.widgets.iter().all(|w| w.rect.unwrap().x == 0.0));
        // Sizes stay.
        d.widgets[0].rect = Some(rect(0.0, 0.0, 400.0, 100.0));
        d.arrange(1000.0);
        assert_eq!(d.widgets[0].rect.unwrap(), rect(0.0, 0.0, 400.0, 100.0));
        assert_eq!(d.widgets[1].rect.unwrap().x, 410.0);
    }

    #[test]
    fn free_position_for_a_new_widget() {
        let mut d = Dashboard::default();
        assert_eq!(d.free_position(240.0, 180.0, 1000.0), (0.0, 0.0), "empty");
        let mut a = Widget::new(sig());
        a.rect = Some(rect(0.0, 0.0, 240.0, 180.0));
        d.widgets.push(a);
        assert_eq!(d.free_position(240.0, 180.0, 1000.0), (250.0, 0.0), "beside, with the gap");
        // A card in the middle of the first row: the scan finds the gap left
        // of it only when the new card fits there.
        let mut b = Widget::new(sig());
        b.rect = Some(rect(400.0, 0.0, 240.0, 180.0));
        d.widgets.push(b);
        assert_eq!(d.free_position(240.0, 180.0, 1000.0), (650.0, 0.0));
        assert_eq!(d.free_position(240.0, 180.0, 880.0), (0.0, 190.0), "no room in the row: below");
        // No room in the width at all: below everything.
        assert_eq!(d.free_position(240.0, 180.0, 100.0), (0.0, 190.0));
        let mut c = Widget::new(sig());
        c.rect = Some(rect(0.0, 190.0, 800.0, 600.0));
        d.widgets.push(c);
        assert_eq!(d.free_position(240.0, 180.0, 700.0), (0.0, 800.0));
        // Bring to front.
        let i = d.bring_to_front(0);
        assert_eq!(i, 2);
        assert_eq!(d.widgets[2].rect.unwrap(), rect(0.0, 0.0, 240.0, 180.0));
    }

    #[test]
    fn old_file_is_arranged() {
        let dir = temp("arr");
        let path = dir.join(DASHBOARD_FILE);
        let one = r#"{"title":"T","size":"SIZE","signal":{"name":"a","scale":1.0,"offset":0.0,"source":{"kind":"pdo","node":1,"index":24576,"sub":1}}}"#;
        let ws: Vec<String> = ["medium", "large", "small", "medium", "medium"].iter().map(|s| one.replace("SIZE", s)).collect();
        std::fs::write(&path, format!(r#"{{"widgets":[{}]}}"#, ws.join(","))).unwrap();
        let d = Dashboard::load(&path).unwrap();
        let r: Vec<WidgetRect> = d.widgets.iter().map(|w| w.rect.unwrap()).collect();
        // 240 + 10 + 340 + 10 + 170 + 10 = 780, the next 240 passes 1000.
        assert_eq!(r[0], rect(0.0, 0.0, 240.0, 180.0));
        assert_eq!(r[1], rect(250.0, 0.0, 340.0, 250.0));
        assert_eq!(r[2], rect(600.0, 0.0, 170.0, 130.0));
        assert_eq!(r[3], rect(0.0, 260.0, 240.0, 180.0));
        assert_eq!(r[4], rect(250.0, 260.0, 240.0, 180.0));
        // Saved by the new version: the rectangles are in the file and load
        // as they are.
        d.save(&path).unwrap();
        let t = std::fs::read_to_string(&path).unwrap();
        assert!(t.contains("\"rect\"") && t.contains("\"x\": 250.0"), "{t}");
        assert_eq!(Dashboard::load(&path).unwrap(), d);
        // A rectangle off the grid is snapped on load.
        std::fs::write(&path, r#"{"widgets":[{"title":"T","signal":{"name":"a","scale":1.0,"offset":0.0,"source":{"kind":"pdo","node":1,"index":24576,"sub":1}},"rect":{"x":-5.0,"y":33.0,"w":20.0,"h":95.0}}]}"#).unwrap();
        assert_eq!(Dashboard::load(&path).unwrap().widgets[0].rect, Some(rect(0.0, 30.0, MIN_W, 100.0)));
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
        assert_eq!(d.widgets[0].rect, Some(WidgetRect { x: 0.0, y: 0.0, w: 240.0, h: 180.0 }), "placed by Arrange");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
