//! Dock tabs (egui_dock): the tab kinds, the default layouts A and B
//! (docs/UI_DESIGN.md section 7), open / close from the View menu, and the
//! layout file `layout.json` in the data folder.

use super::App;
use eframe::egui;
use egui_dock::{DockState, NodeIndex, SurfaceIndex, TabViewer};
use egui_phosphor::regular as icon;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) const LAYOUT_FILE: &str = "layout.json";
/// Most trace tabs at a time.
pub(crate) const MAX_TRACES: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Tab {
    Devices,
    Eds,
    Od,
    Network,
    /// Trace 1..4.
    Trace(u8),
    Graph,
    Pdo,
    TxList,
    Lss,
    Dcf,
    /// One row per COB-ID with the data-change display (0.3).
    Messages,
}

impl Tab {
    /// The tabs of the View menu, in menu order (trace tabs separately).
    pub(crate) const VIEWS: [Tab; 10] = [Tab::Devices, Tab::Eds, Tab::Od, Tab::Network, Tab::Messages, Tab::Graph, Tab::Pdo, Tab::TxList, Tab::Lss, Tab::Dcf];

    pub(crate) fn name(self) -> String {
        match self {
            Tab::Devices => "Devices".into(),
            Tab::Eds => "EDS library".into(),
            Tab::Od => "Object dictionary".into(),
            Tab::Network => "Network".into(),
            Tab::Trace(n) => format!("Trace {n}"),
            Tab::Graph => "Graph".into(),
            Tab::Pdo => "PDO configuration".into(),
            Tab::TxList => "Transmit list".into(),
            Tab::Lss => "LSS".into(),
            Tab::Dcf => "DCF save / restore".into(),
            Tab::Messages => "Messages".into(),
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Tab::Devices => icon::CPU,
            Tab::Eds => icon::BOOKS,
            Tab::Od => icon::LIST_MAGNIFYING_GLASS,
            Tab::Network => icon::GRAPH,
            Tab::Trace(_) => icon::LIST_DASHES,
            Tab::Graph => icon::CHART_LINE,
            Tab::Pdo => icon::SLIDERS_HORIZONTAL,
            Tab::TxList => icon::PAPER_PLANE_TILT,
            Tab::Lss => icon::IDENTIFICATION_CARD,
            Tab::Dcf => icon::FILE_CODE,
            Tab::Messages => icon::TABLE,
        }
    }
}

/// Layout A (window narrower than 1600) or B.
pub(crate) fn default_layout(width: f32) -> DockState<Tab> {
    if width < 1600.0 {
        layout_a()
    } else {
        layout_b()
    }
}

pub(crate) fn layout_a() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Od, Tab::Network, Tab::Pdo]);
    let s = dock.main_surface_mut();
    // egui_dock 0.16 gives `fraction` to the left (new) node here.
    let [main, _devices] = s.split_left(NodeIndex::root(), 0.21, vec![Tab::Devices, Tab::Eds]);
    let [_od, bottom] = s.split_below(main, 0.56, vec![Tab::Trace(1), Tab::Messages, Tab::TxList]);
    let [_trace, _graph] = s.split_right(bottom, 0.58, vec![Tab::Graph]);
    dock
}

pub(crate) fn layout_b() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Od, Tab::Pdo]);
    let s = dock.main_surface_mut();
    let [mid, _dev] = s.split_left(NodeIndex::root(), 0.15, vec![Tab::Devices, Tab::Eds]);
    let [mid, right] = s.split_right(mid, 0.60, vec![Tab::Network]);
    let [_od, _trace] = s.split_below(mid, 0.55, vec![Tab::Trace(1), Tab::Messages, Tab::TxList]);
    let [_net, _graph] = s.split_below(right, 0.40, vec![Tab::Graph]);
    dock
}

/// Load the saved layout; `None` when there is none or it is broken (a
/// broken file gives a note as well).
pub(crate) fn load_layout(dir: &Path) -> (Option<DockState<Tab>>, Option<String>) {
    let p = dir.join(LAYOUT_FILE);
    let Ok(t) = std::fs::read_to_string(&p) else { return (None, None) };
    match serde_json::from_str::<DockState<Tab>>(&t) {
        Ok(d) if valid(&d) => (Some(d), None),
        Ok(_) => (None, Some(format!("{} has no usable tab; default layout used", p.display()))),
        Err(e) => (None, Some(format!("{} not read ({e}); default layout used", p.display()))),
    }
}

/// A layout with at least one tab, no duplicate tab and trace numbers 1..4.
fn valid(d: &DockState<Tab>) -> bool {
    let tabs: Vec<Tab> = d.iter_all_tabs().map(|(_, t)| *t).collect();
    let mut seen = std::collections::HashSet::new();
    !tabs.is_empty() && tabs.iter().all(|t| seen.insert(*t) && !matches!(t, Tab::Trace(n) if *n == 0 || *n > MAX_TRACES))
}

pub(crate) fn save_layout(dir: &Path, d: &DockState<Tab>) -> Result<(), String> {
    // A leaf that was never drawn keeps `Rect::NOTHING` (infinite), which
    // JSON writes as null and cannot read back: save such rects as zero.
    let mut d = d.clone();
    let fix = |r: &mut egui::Rect| {
        if !(r.min.is_finite() && r.max.is_finite()) {
            *r = egui::Rect::ZERO;
        }
    };
    for (_, n) in d.iter_all_nodes_mut() {
        match n {
            egui_dock::Node::Leaf { rect, viewport, .. } => {
                fix(rect);
                fix(viewport);
            }
            egui_dock::Node::Vertical { rect, .. } | egui_dock::Node::Horizontal { rect, .. } => fix(rect),
            egui_dock::Node::Empty => {}
        }
    }
    let t = serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?;
    crate::library::write_atomic(&dir.join(LAYOUT_FILE), t.as_bytes()).map_err(|e| e.to_string())
}

/// The leaf of the main surface that holds `tab`.
fn leaf_of(d: &DockState<Tab>, tab: Tab) -> Option<NodeIndex> {
    match d.find_tab(&tab) {
        Some((SurfaceIndex(0), n, _)) => Some(n),
        _ => None,
    }
}

/// The largest leaf of the main surface (by its last drawn area; the
/// first leaf before the first frame).
fn largest_leaf(d: &DockState<Tab>) -> Option<NodeIndex> {
    let mut best: Option<(f32, NodeIndex)> = None;
    for (i, n) in d.main_surface().iter().enumerate() {
        if n.tabs().is_none() {
            continue;
        }
        let a = n.rect().map(|r| r.area()).unwrap_or(0.0);
        if best.is_none_or(|(ba, _)| a > ba) {
            best = Some((a, NodeIndex(i)));
        }
    }
    best.map(|(_, n)| n)
}

/// Open a tab (or bring it to the front when it is open). LSS opens beside
/// Network, DCF beside the object dictionary (spec 7) and Messages beside a
/// trace tab, else the largest
/// leaf takes it.
pub(crate) fn open_tab(d: &mut DockState<Tab>, tab: Tab) {
    if let Some(loc) = d.find_tab(&tab) {
        d.set_active_tab(loc);
        return;
    }
    let near = match tab {
        Tab::Lss => leaf_of(d, Tab::Network),
        Tab::Dcf => leaf_of(d, Tab::Od),
        Tab::Trace(_) | Tab::Messages => (1..=MAX_TRACES).find_map(|n| leaf_of(d, Tab::Trace(n))),
        _ => None,
    };
    match near.or_else(|| largest_leaf(d)) {
        Some(leaf) => {
            let s = d.main_surface_mut();
            s.set_focused_node(leaf);
            s.push_to_focused_leaf(tab);
        }
        None => {
            // Every tab was closed: a new root leaf.
            *d = DockState::new(vec![tab]);
        }
    }
}

pub(crate) fn close_tab(d: &mut DockState<Tab>, tab: Tab) -> bool {
    match d.find_tab(&tab) {
        Some(loc) => d.remove_tab(loc).is_some(),
        None => false,
    }
}

pub(crate) fn is_open(d: &DockState<Tab>, tab: Tab) -> bool {
    d.find_tab(&tab).is_some()
}

/// The trace tabs that are open.
pub(crate) fn open_traces(d: &DockState<Tab>) -> Vec<u8> {
    let mut v: Vec<u8> = d.iter_all_tabs().filter_map(|(_, t)| if let Tab::Trace(n) = t { Some(*n) } else { None }).collect();
    v.sort();
    v
}

/// Draws each tab with the views of the app.
pub(crate) struct Viewer<'a> {
    pub app: &'a mut App,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        let mut t = format!("{}  {}", tab.icon(), tab.name());
        if let Tab::Trace(n) = tab {
            if self.app.trace_paused(*n) {
                t.push_str(" \u{2022}");
            }
        }
        if *tab == Tab::Pdo && self.app.pdo.has_pending() {
            t.push_str(" \u{2022}");
        }
        t.into()
    }

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("dock_tab", *tab))
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        let tab = *tab;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| match tab {
            Tab::Devices => self.app.devices_tab(ui),
            Tab::Eds => self.app.eds_tab(ui),
            Tab::Od => self.app.od_tab(ui),
            Tab::Network => self.app.network_tab(ui),
            Tab::Trace(n) => self.app.trace_tab(ui, n),
            Tab::Graph => self.app.graph_tab(ui),
            Tab::Pdo => self.app.pdo_tab(ui),
            Tab::TxList => self.app.txlist_tab(ui),
            Tab::Lss => self.app.lss_tab(ui),
            Tab::Dcf => self.app.dcf_tab(ui),
            Tab::Messages => self.app.messages_tab(ui),
        });
    }

    fn on_close(&mut self, tab: &mut Tab) -> bool {
        if let Tab::Trace(n) = tab {
            self.app.close_trace(*n);
        }
        true
    }

    fn allowed_in_windows(&self, _tab: &mut Tab) -> bool {
        false
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs_by_leaf(d: &DockState<Tab>) -> Vec<Vec<Tab>> {
        d.main_surface().iter().filter_map(|n| n.tabs().map(|t| t.to_vec())).collect()
    }

    #[test]
    fn layout_a_and_b_hold_the_spec_tabs() {
        let a = tabs_by_leaf(&layout_a());
        assert_eq!(a.len(), 4);
        for want in [vec![Tab::Devices, Tab::Eds], vec![Tab::Od, Tab::Network, Tab::Pdo], vec![Tab::Trace(1), Tab::Messages, Tab::TxList], vec![Tab::Graph]] {
            assert!(a.contains(&want), "layout A lacks {want:?}: {a:?}");
        }
        let b = tabs_by_leaf(&layout_b());
        assert_eq!(b.len(), 5);
        for want in [vec![Tab::Devices, Tab::Eds], vec![Tab::Od, Tab::Pdo], vec![Tab::Trace(1), Tab::Messages, Tab::TxList], vec![Tab::Network], vec![Tab::Graph]] {
            assert!(b.contains(&want), "layout B lacks {want:?}: {b:?}");
        }
        for d in [layout_a(), layout_b()] {
            assert!(!is_open(&d, Tab::Lss) && !is_open(&d, Tab::Dcf), "LSS and DCF are closed by default");
        }
        assert_eq!(tabs_by_leaf(&default_layout(1280.0)), a);
        assert_eq!(tabs_by_leaf(&default_layout(1920.0)), b);
    }

    #[test]
    fn lss_opens_beside_network_and_dcf_beside_od() {
        for mut d in [layout_a(), layout_b()] {
            open_tab(&mut d, Tab::Lss);
            open_tab(&mut d, Tab::Dcf);
            assert_eq!(leaf_of(&d, Tab::Lss), leaf_of(&d, Tab::Network));
            assert_eq!(leaf_of(&d, Tab::Dcf), leaf_of(&d, Tab::Od));
            // Network gone: LSS goes to the largest (here the first) leaf.
            close_tab(&mut d, Tab::Lss);
            close_tab(&mut d, Tab::Network);
            open_tab(&mut d, Tab::Lss);
            assert!(is_open(&d, Tab::Lss));
        }
    }

    /// A layout file of 0.2 (no Messages tab) still loads; Messages then
    /// opens beside the trace tab.
    #[test]
    fn old_layout_loads_and_messages_opens_beside_trace() {
        let dir = std::env::temp_dir().join(format!("canoxide_layout_old_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut old = layout_a();
        close_tab(&mut old, Tab::Messages);
        save_layout(&dir, &old).unwrap();
        let t = std::fs::read_to_string(dir.join(LAYOUT_FILE)).unwrap();
        assert!(!t.contains("Messages"));
        let (back, note) = load_layout(&dir);
        assert!(note.is_none(), "{note:?}");
        let mut d = back.unwrap();
        assert!(!is_open(&d, Tab::Messages));
        open_tab(&mut d, Tab::Messages);
        assert_eq!(leaf_of(&d, Tab::Messages), leaf_of(&d, Tab::Trace(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn close_all_then_open_makes_a_new_root() {
        let mut d = layout_a();
        let all: Vec<Tab> = d.iter_all_tabs().map(|(_, t)| *t).collect();
        for t in all {
            close_tab(&mut d, t);
        }
        open_tab(&mut d, Tab::Graph);
        assert_eq!(d.iter_all_tabs().count(), 1);
    }

    #[test]
    fn layout_file_round_trip_and_broken_file() {
        let dir = std::env::temp_dir().join(format!("canoxide_layout_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = layout_b();
        open_tab(&mut d, Tab::Trace(2));
        save_layout(&dir, &d).unwrap();
        let (back, note) = load_layout(&dir);
        assert!(note.is_none(), "{note:?}");
        assert_eq!(tabs_by_leaf(&back.unwrap()), tabs_by_leaf(&d));
        std::fs::write(dir.join(LAYOUT_FILE), b"{ broken").unwrap();
        let (back, note) = load_layout(&dir);
        assert!(back.is_none() && note.unwrap().contains("default layout used"));
        std::fs::write(dir.join(LAYOUT_FILE), serde_json::to_string(&DockState::new(vec![Tab::Trace(9)])).unwrap()).unwrap();
        assert!(load_layout(&dir).0.is_none(), "trace number out of range");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
