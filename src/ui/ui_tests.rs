//! Headless UI tests (egui_kittest).
//!
//! Each test runs the real `App` (the real `update` code and the real bus
//! worker thread) with the Simulator adapter, in its own temporary data
//! folder, with no window and no GPU. Widgets are found and clicked through
//! the accessibility tree; every assertion reads labels, values, toggle and
//! disabled states from that tree.
//!
//! Most tests show one view at a time (`only`), so that the labels of other
//! dock tabs do not get in the way; the layout tests check the default
//! layouts themselves.
//!
//! No test selects the PEAK or the CANfox adapter, so no test loads the PCAN
//! or the Sontheim driver, opens a channel or reads the channel list: `build`
//! refuses settings that name a hardware adapter before the app (and its
//! first `ListChannels`) starts, and the picker test only opens the list.
//!
//! The ignored test `screenshots` renders PNG files with wgpu (offscreen,
//! no window) to `target\screens\`.

use super::dock::{self, Tab};
use super::{theme, App, Boot};
use crate::bus::BusCommand;
use crate::settings::{AdapterKind, Settings, Theme, TimestampFormat};
use eframe::egui;
use egui::accesskit::{NodeId, Rect, Role, Toggled};
use egui_dock::{DockState, NodeIndex};
use egui_kittest::kittest::{Key, Node, Queryable};
use egui_kittest::Harness;
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type H = Harness<'static, App>;

/// Longest wait for one UI condition.
const WAIT: Duration = Duration::from_secs(8);

// ------------------------------------------------------------ test fixture

/// A temporary folder, removed at the end of the test.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let p = std::env::temp_dir().join(format!("canoxide_uitest_{name}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).expect("create temp dir");
        TempDir(p)
    }

    /// The data folder of the app (created by the app itself).
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_eds() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join("test.eds")
}

/// Write settings into a data folder before the app starts.
fn preset(dir: &Path, s: Settings) {
    std::fs::create_dir_all(dir).unwrap();
    s.save(dir).unwrap();
}

/// Build the app on a data folder, as `ui::run` does, but without a window.
fn build(dir: &Path) -> H {
    build_sized(dir, 1600.0, 1200.0)
}

fn build_sized(dir: &Path, w: f32, h: f32) -> H {
    let boot = Boot::load_dir(dir.to_path_buf());
    assert_eq!(boot.settings.adapter, AdapterKind::Simulator, "a UI test must never use the PEAK or the CANfox adapter");
    Harness::builder().with_size(egui::Vec2::new(w, h)).build_eframe(move |cc| App::new(cc, boot))
}

/// Show only these tabs, in one dock leaf (the first one in front).
fn only(h: &mut H, tabs: Vec<Tab>) {
    set_dock(h, DockState::new(tabs));
}

/// Two leaves side by side.
fn side_by_side(h: &mut H, left: Vec<Tab>, right: Vec<Tab>) {
    let mut d = DockState::new(left);
    d.main_surface_mut().split_right(NodeIndex::root(), 0.5, right);
    set_dock(h, d);
}

fn set_dock(h: &mut H, d: DockState<Tab>) {
    {
        let app = h.state_mut();
        app.dock = Some(d);
        app.first_layout = false;
        app.sync_traces();
    }
    step2(h);
}

/// The tabs of each leaf of the main surface.
fn leaves(h: &H) -> Vec<Vec<Tab>> {
    h.state().dock.as_ref().map(|d| d.main_surface().iter().filter_map(|n| n.tabs().map(|t| t.to_vec())).collect()).unwrap_or_default()
}

// ------------------------------------------------------------ tree helpers

/// The text of a node: the value for a label, else the label.
fn text_of(n: &Node<'_>) -> String {
    let t = if n.role() == Role::Label { n.value() } else { n.label() };
    t.unwrap_or_default()
}

fn rect_of(n: &Node<'_>) -> Rect {
    n.raw_bounds().unwrap_or(Rect { x0: -1.0, y0: -1.0, x1: -1.0, y1: -1.0 })
}

fn cy(r: &Rect) -> f64 {
    (r.y0 + r.y1) / 2.0
}

/// All texts of the tree, for failure messages.
fn dump(h: &H) -> String {
    let mut v = Vec::new();
    for n in h.query_all_by(|_| true) {
        let t = text_of(&n);
        let val = if n.role() != Role::Label { n.value().map(|v| format!(" value={v:?}")).unwrap_or_default() } else { String::new() };
        if t.is_empty() && val.is_empty() {
            continue;
        }
        let r = rect_of(&n);
        v.push(format!("{:?} {t:?}{val}{} @({:.0},{:.0})", n.role(), if n.is_disabled() { " [disabled]" } else { "" }, r.x0, r.y0));
    }
    v.join("\n")
}

fn has(h: &H, s: &str) -> bool {
    h.query_all_by_label_contains(s).next().is_some()
}

fn has_exact(h: &H, s: &str) -> bool {
    h.query_all_by_label(s).next().is_some()
}

fn count_exact(h: &H, s: &str) -> usize {
    h.query_all_by_label(s).count()
}

/// Id of the node with this exact label that is highest on the screen.
#[track_caller]
fn top_by_label(h: &H, label: &str, role: Option<Role>) -> NodeId {
    top_in(h, label, role, f64::MIN, f64::MAX)
}

/// As `top_by_label`, but only nodes that start between `x0` and `x1`.
#[track_caller]
fn top_in(h: &H, label: &str, role: Option<Role>, x0: f64, x1: f64) -> NodeId {
    let mut best: Option<(f64, NodeId)> = None;
    for n in h.query_all_by_label(label) {
        if role.is_some_and(|r| n.role() != r) {
            continue;
        }
        let r = rect_of(&n);
        if r.x0 < x0 || r.x0 > x1 {
            continue;
        }
        if best.is_none_or(|(by, _)| r.y0 < by) {
            best = Some((r.y0, n.id()));
        }
    }
    match best {
        Some((_, id)) => id,
        None => panic!("no node labelled {label:?} ({role:?})\nUI:\n{}", dump(h)),
    }
}

/// The control (not a label) whose label contains `s`.
#[track_caller]
fn control_containing(h: &H, s: &str) -> NodeId {
    match h.query_all_by_label_contains(s).find(|n| n.role() != Role::Label) {
        Some(n) => n.id(),
        None => panic!("no control containing {s:?}\nUI:\n{}", dump(h)),
    }
}

#[track_caller]
fn node(h: &H, id: NodeId) -> Node<'_> {
    h.get_by(move |n| n.id() == id)
}

fn rect_id(h: &H, id: NodeId) -> Rect {
    rect_of(&node(h, id))
}

/// The node of `role` on the same row, right of the label `label` (the
/// widget that the label names, for example a text field).
#[track_caller]
fn right_of(h: &H, label: &str, role: Role) -> NodeId {
    right_of_in(h, label, role, f64::MIN, f64::MAX)
}

#[track_caller]
fn right_of_in(h: &H, label: &str, role: Role, x0: f64, x1: f64) -> NodeId {
    let a = rect_id(h, top_in(h, label, Some(Role::Label), x0, x1));
    let mut best: Option<(f64, NodeId)> = None;
    for n in h.query_all_by_role(role) {
        let r = rect_of(&n);
        if (cy(&r) - cy(&a)).abs() < 8.0 && r.x0 >= a.x1 - 1.0 && best.is_none_or(|(bx, _)| r.x0 < bx) {
            best = Some((r.x0, n.id()));
        }
    }
    match best {
        Some((_, id)) => id,
        None => panic!("no {role:?} right of {label:?}\nUI:\n{}", dump(h)),
    }
}

/// The node on the row of `anchor` in the column whose header is `col`
/// (the header nearest above the anchor). A pill sits a few pixels right
/// of its column start.
fn cell(h: &H, anchor: NodeId, col: &str, role: Option<Role>) -> Option<NodeId> {
    let a = rect_id(h, anchor);
    let mut header: Option<Rect> = None;
    for n in h.query_all_by_label(col) {
        let r = rect_of(&n);
        if r.y1 <= a.y0 + 1.0 && header.is_none_or(|hr| r.y0 > hr.y0) {
            header = Some(r);
        }
    }
    let hx = header?.x0;
    let mut best: Option<(f64, NodeId)> = None;
    for n in h.query_all_by(|_| true) {
        if role.is_some_and(|r| n.role() != r) || n.id() == anchor {
            continue;
        }
        if role.is_none() && n.role() != Role::Label {
            continue;
        }
        let r = rect_of(&n);
        let d = r.x0 - hx;
        if (cy(&r) - cy(&a)).abs() < 6.0 && (-6.0..16.0).contains(&d) && best.is_none_or(|(bd, _)| d.abs() < bd) {
            best = Some((d.abs(), n.id()));
        }
    }
    best.map(|(_, id)| id)
}

/// Text of a table cell ("" when the cell is empty).
fn cell_text(h: &H, anchor: NodeId, col: &str) -> String {
    cell(h, anchor, col, None).map(|id| text_of(&node(h, id))).unwrap_or_default()
}

/// A button with this label on the row of `anchor`.
#[track_caller]
fn button_on_row(h: &H, anchor: NodeId, label: &str) -> NodeId {
    let a = rect_id(h, anchor);
    for n in h.query_all_by_role_and_label(Role::Button, label) {
        if (cy(&rect_of(&n)) - cy(&a)).abs() < 6.0 {
            return n.id();
        }
    }
    panic!("no button {label:?} on the row\nUI:\n{}", dump(h));
}

fn disabled(h: &H, id: NodeId) -> bool {
    node(h, id).is_disabled()
}

fn toggled(h: &H, id: NodeId) -> bool {
    node(h, id).toggled() == Some(Toggled::True)
}

/// Every control with this exact label is disabled (and there is one).
#[track_caller]
fn all_disabled(h: &H, label: &str) {
    let v: Vec<bool> = h.query_all_by_label(label).filter(|n| n.role() != Role::Label).map(|n| n.is_disabled()).collect();
    assert!(!v.is_empty(), "no control {label:?}\nUI:\n{}", dump(h));
    assert!(v.iter().all(|d| *d), "control {label:?} is enabled\nUI:\n{}", dump(h));
}

/// Every control whose label contains `s` is disabled (and there is one).
#[track_caller]
fn all_disabled_containing(h: &H, s: &str) {
    let v: Vec<bool> = h.query_all_by_label_contains(s).filter(|n| n.role() != Role::Label).map(|n| n.is_disabled()).collect();
    assert!(!v.is_empty(), "no control containing {s:?}\nUI:\n{}", dump(h));
    assert!(v.iter().all(|d| *d), "a control containing {s:?} is enabled\nUI:\n{}", dump(h));
}

// ---------------------------------------------------------- input helpers

fn step2(h: &mut H) {
    h.step();
    h.step();
}

#[track_caller]
fn click_id(h: &mut H, id: NodeId) {
    node(h, id).click();
    step2(h);
}

/// Click the (topmost) control with this exact label; a plain label (for
/// example a heading with the same text) only when no control has it.
#[track_caller]
fn click(h: &mut H, label: &str) {
    let control = h.query_all_by_label(label).find(|n| n.role() != Role::Label).map(|n| n.role());
    let id = top_by_label(h, label, control);
    click_id(h, id);
}

#[track_caller]
fn click_role(h: &mut H, role: Role, label: &str) {
    let id = top_by_label(h, label, Some(role));
    click_id(h, id);
}

/// Click the control whose label contains `s`.
#[track_caller]
fn click_containing(h: &mut H, s: &str) {
    let id = control_containing(h, s);
    click_id(h, id);
}

/// Click the button `label` that was not there before (the button of a
/// dialog that has just opened).
#[track_caller]
fn click_new_button(h: &mut H, label: &str, before: &[NodeId]) {
    let id = h.query_all_by_role_and_label(Role::Button, label).map(|n| n.id()).find(|id| !before.contains(id));
    match id {
        Some(id) => click_id(h, id),
        None => panic!("no new button {label:?}\nUI:\n{}", dump(h)),
    }
}

fn buttons(h: &H, label: &str) -> Vec<NodeId> {
    h.query_all_by_role_and_label(Role::Button, label).map(|n| n.id()).collect()
}

/// Choose `option` in the (topmost) combo box named `combo` that starts
/// between `x0` and `x1`.
#[track_caller]
fn pick_in(h: &mut H, combo: &str, option: &str, x0: f64, x1: f64) {
    let before = buttons(h, option);
    let id = top_in(h, combo, Some(Role::ComboBox), x0, x1);
    click_id(h, id);
    click_new_button(h, option, &before);
}

#[track_caller]
fn pick(h: &mut H, combo: &str, option: &str) {
    pick_in(h, combo, option, f64::MIN, f64::MAX);
}

/// Open the Filters popup of the trace tab that starts between `x0` and `x1`.
#[track_caller]
fn open_filters_in(h: &mut H, x0: f64, x1: f64) {
    let id = top_in(h, "Filters", Some(Role::Button), x0, x1);
    click_id(h, id);
    assert!(has_exact(h, "Reset filters"), "filter popup did not open\nUI:\n{}", dump(h));
}

/// Open the "More" row of the Network view (SYNC, simulator EMCY).
#[track_caller]
fn open_more(h: &mut H) {
    if !has_exact(h, "Send SYNC") {
        click(h, "More");
    }
}

#[track_caller]
fn open_filters(h: &mut H) {
    if !has_exact(h, "Reset filters") {
        open_filters_in(h, f64::MIN, f64::MAX);
    }
}

/// Replace the text of a text field.
#[track_caller]
fn set_text(h: &mut H, id: NodeId, text: &str) {
    node(h, id).focus();
    h.step();
    node(h, id).key_combination(&[Key::Command, Key::A]);
    h.step();
    if text.is_empty() {
        node(h, id).key_press(Key::Backspace);
    } else {
        node(h, id).type_text(text);
    }
    step2(h);
    let now = node(h, id).value().unwrap_or_default();
    assert_eq!(now, text, "text field not set\nUI:\n{}", dump(h));
}

/// Drop files onto the window (the drag-and-drop code path).
fn drop_files(h: &mut H, paths: &[PathBuf]) {
    for p in paths {
        h.input_mut().dropped_files.push(egui::DroppedFile { path: Some(p.clone()), ..Default::default() });
    }
    step2(h);
}

/// Click a check box of the View menu (the menu opens first when needed).
#[track_caller]
fn view_menu(h: &mut H, name: &str) {
    if !h.query_all_by_role_and_label(Role::CheckBox, name).any(|_| true) {
        click_role(h, Role::Button, "View");
    }
    click_role(h, Role::CheckBox, name);
}

// ----------------------------------------------------------- time helpers

/// Step the UI with short real waits until `cond` holds.
#[track_caller]
fn wait_for(h: &mut H, what: &str, cond: impl Fn(&H) -> bool) {
    let t0 = Instant::now();
    loop {
        h.step();
        if cond(h) {
            return;
        }
        if t0.elapsed() > WAIT {
            panic!("timeout waiting for {what}\nUI:\n{}", dump(h));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Step the UI for a while (only where the test needs time to pass).
fn run_for(h: &mut H, d: Duration) {
    let t0 = Instant::now();
    while t0.elapsed() < d {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ------------------------------------------------------------ app helpers

fn connect(h: &mut H) {
    click(h, "Connect");
    wait_for(h, "connected", |h| has_exact(h, "Disconnect") && has(h, "Connected: Simulator"));
}

/// (shown, in buffer, received) of the count line that contains `s`
/// (the leftmost when `left`, else the rightmost).
#[track_caller]
fn trace_counts_at(h: &H, left: bool) -> (usize, usize, u64) {
    let mut lines: Vec<(f64, String)> = h.query_all_by_label_contains(" shown, ").map(|n| (rect_of(&n).x0, text_of(&n))).collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let t = if left { lines.first() } else { lines.last() };
    let t = t.map(|x| x.1.clone()).unwrap_or_else(|| panic!("no trace count line\nUI:\n{}", dump(h)));
    let num = |s: &str| s.trim().split(' ').next().unwrap_or("").parse::<u64>().unwrap_or(u64::MAX);
    let parts: Vec<&str> = t.split(',').collect();
    (num(parts[0]) as usize, num(parts[1]) as usize, num(parts[2]))
}

#[track_caller]
fn trace_counts(h: &H) -> (usize, usize, u64) {
    trace_counts_at(h, true)
}

/// The lowest row (newest, the table sticks to the bottom) whose COB-ID is `id`.
fn last_row(h: &H, id: &str) -> Option<NodeId> {
    let mut best: Option<(f64, NodeId)> = None;
    for n in h.query_all_by_label(id) {
        let y = rect_of(&n).y0;
        if best.is_none_or(|(by, _)| y > by) {
            best = Some((y, n.id()));
        }
    }
    best.map(|(_, id)| id)
}

/// Add test.eds by drag and drop and add the device `name` (node 1) with it.
fn add_eds_and_device(h: &mut H, name: &str) {
    only(h, vec![Tab::Devices]);
    drop_files(h, &[test_eds()]);
    assert!(has(h, "Added EDS: test.eds"), "EDS not added\nUI:\n{}", dump(h));
    click(h, "Add");
    assert!(has_exact(h, "Add device"), "device dialog not open\nUI:\n{}", dump(h));
    let name_id = right_of(h, "Name", Role::TextInput);
    set_text(h, name_id, name);
    click(h, "Save");
    wait_for(h, "device saved", |h| has(h, &format!("Device {name} saved")));
    assert!(!has_exact(h, "Add device"), "dialog still open");
    let dev = top_by_label(h, &format!("{name}  (node 1)"), None);
    assert!(toggled(h, dev), "new device is not active\nUI:\n{}", dump(h));
}

/// Send NMT Start to node 1 from the Network view.
fn start_node(h: &mut H) {
    only(h, vec![Tab::Network]);
    wait_for(h, "node 1 in the list", |h| has_exact(h, "Node 1"));
    let start = top_by_label(h, "Start", Some(Role::Button));
    click_id(h, start);
    wait_for(h, "NMT Start sent", |h| has_exact(h, "NMT Start node 1 sent"));
}

fn tx_glyph() -> &'static str {
    icon::ARROW_UP_RIGHT
}

/// Frames with this COB-ID in the buffer (view 0 = Trace 1, no filter).
fn cob_count(h: &H, id: u32) -> usize {
    let t = &h.state().trace;
    (0..t.view_len_of(0)).filter_map(|i| t.view_row_of(0, i)).filter(|e| e.frame.id == id).count()
}

// ================================================================== tests

/// Scenario 1: start state, send controls disabled while not connected.
#[test]
fn s1_start_not_connected() {
    let tmp = TempDir::new("s1");
    let mut h = build(&tmp.data());
    assert!(has_exact(&h, "Not connected"));
    assert!(has_exact(&h, "Connect"));
    assert!(has(&h, "Nothing is sent until you press a button."));
    // Object dictionary: manual SDO box.
    only(&mut h, vec![Tab::Od]);
    assert!(has_exact(&h, "not connected"));
    all_disabled(&h, "Read");
    all_disabled(&h, "Write");
    all_disabled(&h, "Read all (select an object)");
    all_disabled(&h, "Upload to file...");
    all_disabled(&h, "Download from file...");
    // Network.
    only(&mut h, vec![Tab::Network]);
    assert!(has_exact(&h, "Not connected: send controls are disabled."));
    open_more(&mut h);
    for l in ["Start", "Stop", "Enter pre-operational", "Reset node", "Reset communication", "Send SYNC", "Scan nodes", "Trigger EMCY", "Refresh node info", "Set heartbeat"] {
        all_disabled(&h, l);
    }
    assert_eq!(count_exact(&h, "Start"), 2, "NMT Start for the node and for all nodes");
    // Transmit list.
    only(&mut h, vec![Tab::TxList]);
    assert!(has_exact(&h, "Not connected: send controls are disabled."));
    all_disabled(&h, "Send once");
    // A command that reaches the worker anyway is refused.
    h.state().bus.send(BusCommand::Sync);
    wait_for(&mut h, "refusal", |h| has(h, "SYNC: not connected"));
}

/// Scenario 2: connect, disconnect, reconnect.
#[test]
fn s2_connect_disconnect_reconnect() {
    let tmp = TempDir::new("s2");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Network]);
    connect(&mut h);
    assert!(!has_exact(&h, "Not connected"));
    assert!(has(&h, "Bus: ") || has_exact(&h, "Error active"), "bus status shown\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "fr/s"));
    assert!(has_exact(&h, "Load"));
    assert!(has_exact(&h, "Connected: Simulator at 250 kbit/s"));
    assert!(has_exact(&h, "Connected · Simulator · virtual bus · 250 kbit/s"), "status bar summary\nUI:\n{}", dump(&h));
    // Adapter, bitrate and listen-only are locked while connected.
    let bar: Vec<bool> = h.query_all_by_role(Role::ComboBox).filter(|n| rect_of(n).y0 < 70.0).map(|n| n.is_disabled()).collect();
    assert_eq!(bar.len(), 2, "adapter and bitrate combo boxes in the connect bar\nUI:\n{}", dump(&h));
    assert!(bar.iter().all(|d| *d), "a connect-bar combo box is enabled while connected\nUI:\n{}", dump(&h));
    assert!(node(&h, top_by_label(&h, "Listen only", Some(Role::CheckBox))).is_disabled());
    click(&mut h, "Disconnect");
    wait_for(&mut h, "disconnected", |h| has_exact(h, "Not connected") && has_exact(h, "Disconnected"));
    assert!(has_exact(&h, "Connect"));
    connect(&mut h);
    assert!(has_exact(&h, "Connected: Simulator at 250 kbit/s"));
}

/// Scenario 3: trace rows, counter, Pause, Clear, data view, Fixed, filters.
/// The adapter picker offers the CANfox. The list is only opened and closed
/// again: choosing an entry would ask its driver for the channel list.
#[test]
fn s2_adapter_picker_offers_canfox() {
    let tmp = TempDir::new("s2_canfox");
    let mut h = build(&tmp.data());
    let before_canfox = buttons(&h, "CANfox");
    let before_peak = buttons(&h, "PEAK PCAN");
    let combo = right_of(&h, "Adapter", Role::ComboBox);
    click_id(&mut h, combo);
    let canfox: Vec<NodeId> = buttons(&h, "CANfox").into_iter().filter(|id| !before_canfox.contains(id)).collect();
    let peak: Vec<NodeId> = buttons(&h, "PEAK PCAN").into_iter().filter(|id| !before_peak.contains(id)).collect();
    assert_eq!(canfox.len(), 1, "no CANfox entry in the adapter list\nUI:\n{}", dump(&h));
    assert_eq!(peak.len(), 1, "no PEAK entry in the adapter list\nUI:\n{}", dump(&h));
    assert!(!buttons(&h, "Log file (replay)").is_empty(), "replay entry\nUI:\n{}", dump(&h));
    // Close the list without a choice.
    h.input_mut().events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    assert!(buttons(&h, "CANfox").iter().all(|id| before_canfox.contains(id)), "the list did not close\nUI:\n{}", dump(&h));
    assert_eq!(h.state().settings.adapter, AdapterKind::Simulator);
    assert_eq!(AdapterKind::Canfox.label(), "CANfox");
}

/// The adapter picker offers Kvaser and both SLCAN entries; "SLCAN (TCP)"
/// shows the host:port field, saved in the settings. Kvaser and SLCAN
/// (serial) are not chosen (no driver is loaded, no COM port listed), and
/// Connect is never pressed.
#[test]
fn s2_adapter_picker_offers_slcan_and_kvaser() {
    let tmp = TempDir::new("s2_slcan");
    let mut h = build(&tmp.data());
    let labels = ["Kvaser", "SLCAN (serial)", "SLCAN (TCP)"];
    let before: Vec<Vec<NodeId>> = labels.iter().map(|l| buttons(&h, l)).collect();
    let combo = right_of(&h, "Adapter", Role::ComboBox);
    click_id(&mut h, combo);
    for (l, b) in labels.iter().zip(&before) {
        let new: Vec<NodeId> = buttons(&h, l).into_iter().filter(|id| !b.contains(id)).collect();
        assert_eq!(new.len(), 1, "no {l} entry in the adapter list\nUI:\n{}", dump(&h));
    }
    click_new_button(&mut h, "SLCAN (TCP)", &before[2]);
    assert_eq!(h.state().settings.adapter, AdapterKind::SlcanTcp);
    let field = right_of(&h, "Address", Role::TextInput);
    set_text(&mut h, field, "127.0.0.1:2001");
    assert_eq!(h.state().settings.slcan_tcp, "127.0.0.1:2001");
    assert!(!has(&h, "not host:port"), "a valid address is flagged\nUI:\n{}", dump(&h));
    set_text(&mut h, field, "gateway");
    assert!(has(&h, "not host:port"), "a bad address is not flagged\nUI:\n{}", dump(&h));
}

#[test]
fn s3_trace() {
    let tmp = TempDir::new("s3");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Trace(1)]);
    assert!(has_exact(&h, "No frames. Connect to see the bus traffic."), "empty state\nUI:\n{}", dump(&h));
    connect(&mut h);
    wait_for(&mut h, "heartbeat row", |h| has_exact(h, "Heartbeat Pre-operational"));
    assert!(has_exact(&h, "Boot-up"), "boot-up row of the simulated node");
    let hb = last_row(&h, "701").expect("row 701");
    assert_eq!(cell_text(&h, hb, "Type"), "Heartbeat");
    assert_eq!(cell_text(&h, hb, "Node"), "1");
    assert_eq!(cell_text(&h, hb, "Dir"), icon::ARROW_DOWN_LEFT);
    let (_, _, rx0) = trace_counts(&h);
    wait_for(&mut h, "received counter rises", |h| trace_counts(h).2 > rx0);

    // Pause: no new rows; the frames are counted as not recorded.
    click_role(&mut h, Role::CheckBox, "Pause");
    // (Pause is per trace tab: the shared buffer keeps recording.)
    let (shown, _, total) = trace_counts(&h);
    wait_for(&mut h, "paused frames counted", |h| has(h, "frame(s) not recorded") && trace_counts(h).2 > total);
    assert_eq!(trace_counts(&h).0, shown, "rows added while paused");
    click_role(&mut h, Role::CheckBox, "Pause");
    wait_for(&mut h, "rows again", |h| trace_counts(h).0 > shown);

    // Clear.
    click(&mut h, "Clear");
    let (shown, buf, _) = trace_counts(&h);
    assert_eq!((shown, buf), (0, 0), "Clear did not empty the list\nUI:\n{}", dump(&h));

    // A known Tx frame "41 42" on 0x123 from the transmit list.
    only(&mut h, vec![Tab::TxList]);
    let data = right_of(&h, "Data", Role::TextInput);
    set_text(&mut h, data, "41 42");
    click(&mut h, "Send once");
    only(&mut h, vec![Tab::Trace(1)]);
    wait_for(&mut h, "Tx row 123", |h| last_row(h, "123").is_some());
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Dir"), tx_glyph());
    assert_eq!(cell_text(&h, row, "DLC"), "2");
    assert_eq!(cell_text(&h, row, "Data"), "41 42");
    pick(&mut h, "Data format", "Decimal");
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Data"), "65 66");
    pick(&mut h, "Data format", "String");
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Data"), "AB");
    pick(&mut h, "Data format", "Hex");
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Data"), "41 42");

    // Fixed: one row per COB-ID, the count rises.
    pick(&mut h, "Display mode", "Fixed");
    let count = |h: &H| last_row(h, "701").map(|r| cell_text(h, r, "Count").parse::<u64>().unwrap_or(0)).unwrap_or(0);
    wait_for(&mut h, "two heartbeats in the fixed view", |h| count(h) >= 2);
    assert_eq!(count_exact(&h, "701"), 1, "fixed view: one row for 0x701\nUI:\n{}", dump(&h));
    assert_eq!(count_exact(&h, "123"), 1, "fixed view: one row for 0x123");
    let c0 = count(&h);
    wait_for(&mut h, "fixed count rises", |h| count(h) > c0);
    assert_eq!(count_exact(&h, "701"), 1, "fixed view: still one row for 0x701");
    pick(&mut h, "Display mode", "Scroll");

    // Node filter in the Filters popup: node 1 hides the Tx frame (no
    // node), node 5 hides all.
    let (all, _, _) = trace_counts(&h);
    open_filters(&mut h);
    let node_f = right_of(&h, "Node ID", Role::TextInput);
    set_text(&mut h, node_f, "1");
    let (n1, _, _) = trace_counts(&h);
    assert!(n1 < all, "node filter 1: {n1} of {all} rows");
    assert!(last_row(&h, "123").is_none(), "Tx frame without node shown with node filter 1");
    set_text(&mut h, node_f, "5");
    assert_eq!(trace_counts(&h).0, 0, "node filter 5 shows rows\nUI:\n{}", dump(&h));
    click(&mut h, "Reset filters");
    assert!(trace_counts(&h).0 >= all, "Reset filters");

    // Type filter: no heartbeat rows.
    open_filters(&mut h);
    assert!(has_exact(&h, "Types (11/11)"), "type count\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::CheckBox, "Heartbeat");
    assert!(has_exact(&h, "Types (10/11)"), "type count\nUI:\n{}", dump(&h));
    let (shown, buf, _) = trace_counts(&h);
    assert!(shown < buf, "type filter: {shown} of {buf} rows");
    assert!(last_row(&h, "701").is_none(), "heartbeat row shown with the Heartbeat type off\nUI:\n{}", dump(&h));
}

/// Scenario 4: node list and NMT, TPDOs, scan, SYNC.
#[test]
fn s4_network() {
    let tmp = TempDir::new("s4");
    // A short scan timeout from the settings file, so a full scan is quick.
    preset(&tmp.data(), Settings { scan_timeout_ms: 20, ..Settings::default() });
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Network]);
    assert!(has(&h, "Scan nodes or Refresh node info read them by SDO."), "empty state\nUI:\n{}", dump(&h));
    connect(&mut h);
    wait_for(&mut h, "node 1 in the list", |h| has(h, "Pre-operational") && has_exact(h, "Node 1"));
    let start = top_by_label(&h, "Start", Some(Role::Button));
    click_id(&mut h, start);
    wait_for(&mut h, "Operational", |h| has(h, " Operational"));
    assert!(has_exact(&h, "NMT Start node 1 sent"));

    // TPDOs in the trace while operational.
    only(&mut h, vec![Tab::Trace(1)]);
    wait_for(&mut h, "TPDO rows", |h| has_exact(h, "TPDO1"));
    only(&mut h, vec![Tab::Network]);

    let id = top_by_label(&h, "Stop", Some(Role::Button));
    click_id(&mut h, id);
    wait_for(&mut h, "Stopped", |h| has(h, " Stopped"));
    let id = top_by_label(&h, "Enter pre-operational", Some(Role::Button));
    click_id(&mut h, id);
    wait_for(&mut h, "Pre-operational", |h| has(h, " Pre-operational"));
    let id = top_by_label(&h, "Start", Some(Role::Button));
    click_id(&mut h, id);
    wait_for(&mut h, "Operational again", |h| has(h, " Operational"));
    let id = top_by_label(&h, "Reset node", Some(Role::Button));
    click_id(&mut h, id);
    wait_for(&mut h, "pre-operational after the reset", |h| has(h, " Pre-operational") && has_exact(h, "NMT Reset node node 1 sent"));

    // Scan: exactly the simulated node.
    click(&mut h, "Scan nodes");
    wait_for(&mut h, "scan finished", |h| has(h, "Scan finished"));
    assert!(has_exact(&h, "Scan finished: 1 node(s) answered"), "\nUI:\n{}", dump(&h));
    assert_eq!(h.query_all_by_label_contains("device type 0x").count(), 1);
    assert!(has_exact(&h, "device type 0x00000191 (profile 401)"));
    // Scan again and cancel it.
    click(&mut h, "Scan nodes");
    wait_for(&mut h, "scan running", |h| has_exact(h, "Cancel"));
    click(&mut h, "Cancel");
    wait_for(&mut h, "scan cancelled", |h| has(h, "Scan cancelled"));
    assert!(has_exact(&h, "Scan nodes"));

    // SYNC.
    open_more(&mut h);
    click(&mut h, "Send SYNC");
    assert!(has_exact(&h, "SYNC sent"));
    only(&mut h, vec![Tab::Trace(1)]);
    wait_for(&mut h, "SYNC row", |h| last_row(h, "080").is_some());
    let s = last_row(&h, "080").unwrap();
    assert_eq!(cell_text(&h, s, "Type"), "SYNC");
    assert_eq!(cell_text(&h, s, "Decoded"), "SYNC");
    assert_eq!(cell_text(&h, s, "Dir"), tx_glyph());
}

/// Scenario 34: a node found by the scan is added as a device from its
/// scan result row.
#[test]
fn s34_scan_result_add_as_device() {
    let tmp = TempDir::new("s34");
    preset(&tmp.data(), Settings { scan_timeout_ms: 20, ..Settings::default() });
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Network]);
    connect(&mut h);
    wait_for(&mut h, "node 1 in the list", |h| has_exact(h, "Node 1"));
    click(&mut h, "Scan nodes");
    wait_for(&mut h, "scan finished", |h| has_exact(h, "Scan finished: 1 node(s) answered"));
    let row = top_by_label(&h, "device type 0x00000191 (profile 401)", None);
    let add = button_on_row(&h, row, "Add as device");
    assert!(!disabled(&h, add), "Add as device disabled\nUI:\n{}", dump(&h));
    assert_eq!(buttons(&h, "Add as device").len(), 1);

    click_id(&mut h, add);
    assert!(has_exact(&h, "Add device"), "device dialog not open\nUI:\n{}", dump(&h));
    match &h.state().dialog {
        Some(super::Dialog::Device { idx: None, dev, .. }) => {
            assert_eq!(dev.node_id, 1);
            assert_eq!(dev.name, "Node 1");
            assert_eq!(dev.eds_file, None);
            assert!(dev.notes.is_empty());
        }
        _ => panic!("no Add device dialog in the state"),
    }
    click(&mut h, "Save");
    wait_for(&mut h, "device saved", |h| has(h, "Device Node 1 saved"));
    assert!(!has_exact(&h, "Add device"), "dialog still open");
    assert!(has_exact(&h, "Device: Node 1"), "device label on the scan row\nUI:\n{}", dump(&h));
    assert!(buttons(&h, "Add as device").is_empty(), "Add as device still shown\nUI:\n{}", dump(&h));
    assert_eq!(h.state().settings.active_device.as_deref(), Some("Node 1"));
}

/// Scenario 5: EDS library, device, object dictionary, decoded trace.
#[test]
fn s5_eds_device_od() {
    let tmp = TempDir::new("s5");
    // Writes without the question here; the question has its own test.
    preset(&tmp.data(), Settings { confirm_before_write: false, ..Settings::default() });
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Bench");
    only(&mut h, vec![Tab::Eds]);
    assert!(has_exact(&h, "test.eds"), "EDS file listed\nUI:\n{}", dump(&h));
    only(&mut h, vec![Tab::Od]);
    // The simulator uses the EDS of the active device: connect after it.
    connect(&mut h);

    for l in ["Device type", "Manufacturer device name", "Identity object"] {
        assert!(has_exact(&h, l), "object {l:?} not listed\nUI:\n{}", dump(&h));
    }
    let filter = right_of(&h, "Filter", Role::TextInput);
    for (f, l) in [("2001", "Mode"), ("6401", "Read analog input 16 bit")] {
        set_text(&mut h, filter, f);
        assert!(has_exact(&h, l), "object {l:?} not listed\nUI:\n{}", dump(&h));
    }
    set_text(&mut h, filter, "1018");
    assert!(has_exact(&h, "Identity object"));
    assert!(has_exact(&h, "Vendor ID"), "filter opens the record");
    assert!(!has_exact(&h, "Device type"), "filter did not narrow\nUI:\n{}", dump(&h));

    // Read 0x1000 and the display modes.
    set_text(&mut h, filter, "1000");
    let row = top_by_label(&h, "1000", Some(Role::Button));
    assert!(disabled(&h, button_on_row(&h, row, "Write")), "Write enabled on a ro object");
    let id = button_on_row(&h, row, "Read");
    click_id(&mut h, id);
    wait_for(&mut h, "0x1000 value", |h| cell_text(h, top_by_label(h, "1000", Some(Role::Button)), "Value") == "0x00030191");
    click(&mut h, "Decimal");
    assert_eq!(cell_text(&h, top_by_label(&h, "1000", Some(Role::Button)), "Value"), "197009");
    click(&mut h, "Hex");

    // Segmented string read.
    set_text(&mut h, filter, "1008");
    let row = top_by_label(&h, "1008", Some(Role::Button));
    assert!(disabled(&h, button_on_row(&h, row, "Write")), "Write enabled on a const object");
    let id = button_on_row(&h, row, "Read");
    click_id(&mut h, id);
    let v = |h: &H| cell_text(h, top_by_label(h, "1008", Some(Role::Button)), "Value");
    wait_for(&mut h, "0x1008 value", |h| !v(h).is_empty() && v(h) != "..." && v(h) != "\u{2014}");
    assert_eq!(v(&h), "54 65 73 74 20 49 4F 20 6D 6F 64 75 6C 65", "hex view of a VISIBLE_STRING");
    click(&mut h, "String");
    assert_eq!(v(&h), "Test IO module");
    click(&mut h, "Decimal");
    assert_eq!(v(&h), "84 101 115 116 32 73 79 32 109 111 100 117 108 101");
    click(&mut h, "Hex");

    // Write rw then Read.
    set_text(&mut h, filter, "2001");
    let row = top_by_label(&h, "2001", Some(Role::Button));
    let edit = cell(&h, row, "New value", Some(Role::TextInput)).expect("New value field");
    set_text(&mut h, edit, "7");
    let id = button_on_row(&h, row, "Write");
    click_id(&mut h, id);
    wait_for(&mut h, "written", |h| has_exact(h, "Written node 1 0x2001:00"));
    let v = |h: &H| cell_text(h, top_by_label(h, "2001", Some(Role::Button)), "Value");
    assert_eq!(v(&h), "0x07");
    let id = button_on_row(&h, row, "Read");
    click_id(&mut h, id);
    wait_for(&mut h, "read back", |h| has_exact(h, "Read node 1 0x2001:00"));
    assert_eq!(v(&h), "0x07");
    // Outside the limits (1..10): refused before sending.
    set_text(&mut h, edit, "11");
    let id = button_on_row(&h, row, "Write");
    click_id(&mut h, id);
    assert!(v(&h).starts_with("not sent:"), "value cell {:?}", v(&h));
    assert!(has(&h, "0x2001:00 not written:"), "status\nUI:\n{}", dump(&h));
    let id = button_on_row(&h, row, "Read");
    click_id(&mut h, id);
    wait_for(&mut h, "read back", |h| v(h) == "0x07");

    // Read all of a RECORD.
    set_text(&mut h, filter, "1018");
    let rec = top_by_label(&h, "1018", Some(Role::Button));
    let id = button_on_row(&h, rec, "Read all");
    click_id(&mut h, id);
    let sub = |h: &H, s: &str| cell_text(h, top_by_label(h, s, Some(Role::Label)), "Value");
    wait_for(&mut h, "subs read", |h| sub(h, "04") == "0x00000000");
    assert_eq!(sub(&h, "00"), "0x04");
    assert_eq!(sub(&h, "01"), "0x0000069B");
    assert_eq!(sub(&h, "02"), "0x00000042");
    assert_eq!(sub(&h, "03"), "0x00010002");

    // Trace: SDO rows carry object names.
    only(&mut h, vec![Tab::Trace(1)]);
    open_filters(&mut h);
    let text = right_of(&h, "Text", Role::TextInput);
    set_text(&mut h, text, "device type");
    assert!(has(&h, "SDO upload req 0x1000:00 (Device type)"), "\nUI:\n{}", dump(&h));
    assert!(has(&h, "SDO upload resp 0x1000:00 = 0x00030191 (Device type)"), "\nUI:\n{}", dump(&h));
    click(&mut h, "Reset filters");
    // PDO rows carry signal names once the node is operational.
    start_node(&mut h);
    only(&mut h, vec![Tab::Trace(1)]);
    wait_for(&mut h, "TPDO1 with signal names", |h| has(h, "Read input 8 bit / Inputs 1-8 = "));
    assert!(has(&h, "Read analog input 16 bit / Analog input 1 = "));
}

/// Scenario 6: manual SDO without an EDS.
#[test]
fn s6_manual_sdo() {
    let tmp = TempDir::new("s6");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Od]);
    connect(&mut h);
    assert!(has_exact(&h, "No active device, node 1"));
    let index = right_of(&h, "Index 0x", Role::TextInput);
    let sub = right_of(&h, "Sub", Role::TextInput);
    set_text(&mut h, index, "1018");
    set_text(&mut h, sub, "1");
    click_role(&mut h, Role::Button, "Read");
    wait_for(&mut h, "1018:01 read", |h| has(h, "0x1018:01: 4 byte(s)"));
    assert!(has_exact(&h, "0x0000069B"), "\nUI:\n{}", dump(&h));
    click(&mut h, "Decimal");
    assert!(has_exact(&h, "1691"));
    click(&mut h, "Hex");
    // An object that does not exist: abort 0x06020000.
    set_text(&mut h, index, "1234");
    set_text(&mut h, sub, "0");
    click_role(&mut h, Role::Button, "Read");
    wait_for(&mut h, "abort", |h| has(h, "0x1234:00: abort 0x06020000"));
    assert!(has(&h, "does not exist"), "\nUI:\n{}", dump(&h));
}

/// Scenario 7: listen only disables the sending controls of every view.
#[test]
fn s7_listen_only_every_view() {
    let tmp = TempDir::new("s7");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Quiet");
    // A transmit list entry (adding sends nothing) and a graph SDO signal.
    only(&mut h, vec![Tab::TxList]);
    click(&mut h, "Add to list");
    wait_for(&mut h, "entry listed", |h| has_exact(h, "stopped"));
    click_role(&mut h, Role::CheckBox, "Listen only");
    connect(&mut h);
    assert!(has(&h, "Listen only") && has(&h, &format!("{}  Listen only", icon::INFO)), "listen-only pill\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "Connected: Simulator at 250 kbit/s, listen only"));
    assert!(has_exact(&h, "Connected · Simulator · virtual bus · 250 kbit/s · listen only"));

    only(&mut h, vec![Tab::Od]);
    assert!(has_exact(&h, "listen only: SDO disabled"));
    for l in ["Read", "Write", "Read all", "Upload to file...", "Download from file..."] {
        all_disabled(&h, l);
    }
    only(&mut h, vec![Tab::Network]);
    assert!(has_exact(&h, "Listen only: every send control is disabled."));
    // (Trigger EMCY stays enabled: the simulated node sends it, not this tool.)
    open_more(&mut h);
    for l in ["Start", "Stop", "Enter pre-operational", "Reset node", "Reset communication", "Send SYNC", "Scan nodes", "Refresh node info", "Set heartbeat"] {
        all_disabled(&h, l);
    }
    // The node is still heard.
    wait_for(&mut h, "heartbeat heard", |h| has(h, "Pre-operational"));
    only(&mut h, vec![Tab::TxList]);
    assert!(has_exact(&h, "Listen only: every send control is disabled."));
    for l in ["Send once", "Send", "Start"] {
        all_disabled(&h, l);
    }
    only(&mut h, vec![Tab::Pdo]);
    for l in ["Read PDOs", "Disable all TPDO", "Enable all TPDO", "Disable all RPDO", "Enable all RPDO"] {
        all_disabled_containing(&h, l);
    }
    only(&mut h, vec![Tab::Lss]);
    assert!(has_exact(&h, "Listen only: every LSS service is disabled."));
    for l in [
        "Switch all to configuration",
        "Switch all to waiting",
        "Switch selective",
        "Inquire identity",
        "Inquire node-ID",
        "Fastscan",
        "Identify non-configured",
        "Configure node-ID",
        "Configure bit timing",
        "Activate bit timing",
        "Store configuration",
    ] {
        all_disabled(&h, l);
    }
    only(&mut h, vec![Tab::Dcf]);
    for l in ["Save DCF...", "Compare", "Dry run", "Restore"] {
        all_disabled_containing(&h, l);
    }
    // Graph: polling cannot start with an SDO signal.
    h.state_mut().add_signal(crate::signals::SignalDef::new("poll", crate::signals::SignalSource::SdoPoll { node: 1, index: 0x6401, sub: 1, period_ms: 200, data_type: None }));
    only(&mut h, vec![Tab::Graph]);
    all_disabled_containing(&h, "Start polling");
    assert!(has(&h, "SDO signals record only while polling runs. Listen-only mode."), "\nUI:\n{}", dump(&h));

    // A send command that reaches the worker is refused.
    h.state().bus.send(BusCommand::Sync);
    wait_for(&mut h, "refusal", |h| has(h, "SYNC: listen-only mode, nothing sent"));
    only(&mut h, vec![Tab::Trace(1)]);
    assert!(last_row(&h, "080").is_none(), "a SYNC was sent in listen-only mode");
    assert!(!h.query_all_by_label(tx_glyph()).any(|_| true), "a frame was sent in listen-only mode\nUI:\n{}", dump(&h));
}

/// Scenario 8: settings and devices survive a restart.
#[test]
fn s8_persistence() {
    let tmp = TempDir::new("s8");
    {
        let mut h = build(&tmp.data());
        only(&mut h, vec![Tab::Od]);
        click(&mut h, "Decimal"); // OD value display
        only(&mut h, vec![Tab::Eds]);
        let combo = right_of(&h, "Bitrate", Role::ComboBox);
        click_id(&mut h, combo);
        click_role(&mut h, Role::Button, "500 kbit/s");
        add_eds_and_device(&mut h, "Persist");
        // The settings are saved one second after the last change.
        run_for(&mut h, Duration::from_millis(1500));
    }
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Devices]);
    let dev = top_by_label(&h, "Persist  (node 1)", None);
    assert!(toggled(&h, dev), "device not active after restart\nUI:\n{}", dump(&h));
    only(&mut h, vec![Tab::Od]);
    assert!(has_exact(&h, "Persist, node 1"));
    assert!(toggled(&h, top_by_label(&h, "Decimal", Some(Role::Button))), "value display not restored");
    only(&mut h, vec![Tab::Eds]);
    assert!(has_exact(&h, "test.eds"), "EDS file in the library");
    let combo = right_of(&h, "Bitrate", Role::ComboBox);
    click_id(&mut h, combo);
    assert!(toggled(&h, top_by_label(&h, "500 kbit/s", Some(Role::Button))), "bitrate not restored\nUI:\n{}", dump(&h));
    click_id(&mut h, combo);
    connect(&mut h);
    assert!(has_exact(&h, "Connected: Simulator at 500 kbit/s"));
}

/// Scenario 9: a data folder that cannot be created.
#[test]
fn s9_data_folder_not_writable() {
    let tmp = TempDir::new("s9");
    // A file where a folder must be: the folder cannot be created.
    let blocker = tmp.0.join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let dir = blocker.join("data");
    let mut h = build(&dir);
    assert!(has(&h, "is not writable"), "no data folder error shown\nUI:\n{}", dump(&h));
    assert!(has(&h, "Nothing is saved."));
    only(&mut h, vec![Tab::Devices]);
    assert!(disabled(&h, top_by_label(&h, "Add", Some(Role::Button))), "Add device enabled without a library");
    only(&mut h, vec![Tab::Eds]);
    assert!(disabled(&h, top_by_label(&h, "Add...", Some(Role::Button))), "Add EDS enabled without a library");
    drop_files(&mut h, &[test_eds()]);
    assert!(has_exact(&h, "The library is not available (data folder error)"));
    // A settings change and the autosave time: no panic, nothing written.
    only(&mut h, vec![Tab::Od]);
    click(&mut h, "Decimal");
    run_for(&mut h, Duration::from_millis(1300));
    h.state_mut().save_layout();
    assert!(!dir.exists());
    // The app still works.
    connect(&mut h);
}

// -------------------------------------------------------- v0.2 scenarios

/// Theme Dark, Light and System (System follows the OS theme).
#[test]
fn s10_theme_dark_light_system() {
    let tmp = TempDir::new("s10");
    let mut h = build(&tmp.data());
    let panel = |h: &H| h.ctx.style().visuals.panel_fill;
    let dark = |h: &H| h.ctx.style().visuals.dark_mode;

    h.state_mut().settings.theme = Theme::Dark;
    step2(&mut h);
    assert!(dark(&h));
    assert_eq!(panel(&h), theme::DARK.bg_panel);

    h.state_mut().settings.theme = Theme::Light;
    step2(&mut h);
    assert!(!dark(&h));
    assert_eq!(panel(&h), theme::LIGHT.bg_panel);

    h.state_mut().settings.theme = Theme::System;
    h.input_mut().system_theme = Some(egui::Theme::Light);
    step2(&mut h);
    assert!(!dark(&h), "System with a light OS theme");
    h.input_mut().system_theme = Some(egui::Theme::Dark);
    step2(&mut h);
    assert!(dark(&h), "System with a dark OS theme");
    assert_eq!(panel(&h), theme::DARK.bg_panel);

    // The theme button of the menu bar cycles System -> Dark -> Light.
    let glyph = top_by_label(&h, icon::DESKTOP, Some(Role::Button));
    click_id(&mut h, glyph);
    assert_eq!(h.state().settings.theme, Theme::Dark);
    let glyph = top_by_label(&h, icon::MOON, Some(Role::Button));
    click_id(&mut h, glyph);
    assert_eq!(h.state().settings.theme, Theme::Light);
    assert!(!dark(&h));
}

/// Each view opens from the View menu and closes again.
#[test]
fn s11_view_menu_opens_and_closes_each_view() {
    let tmp = TempDir::new("s11");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Trace(1)]);
    for t in Tab::VIEWS {
        let name = t.name();
        assert!(!h.state().is_open(t));
        view_menu(&mut h, &name);
        assert!(h.state().is_open(t), "{name} not opened");
        assert!(toggled(&h, top_by_label(&h, &name, Some(Role::CheckBox))), "{name}: menu check box not ticked");
        view_menu(&mut h, &name);
        assert!(!h.state().is_open(t), "{name} not closed");
    }
    // Trace tabs: 2 opens, 1 closes; at most four at a time.
    view_menu(&mut h, "Trace 2");
    assert!(h.state().is_open(Tab::Trace(2)));
    view_menu(&mut h, "Trace 1");
    assert!(!h.state().is_open(Tab::Trace(1)));
    for _ in 0..4 {
        if !h.query_all_by_role_and_label(Role::Button, "New trace tab").any(|_| true) {
            click_role(&mut h, Role::Button, "View");
        }
        let id = top_by_label(&h, "New trace tab", Some(Role::Button));
        if !disabled(&h, id) {
            click_id(&mut h, id);
        }
    }
    assert_eq!(dock::open_traces(h.state().dock.as_ref().unwrap()), vec![1, 2, 3, 4]);
    if !h.query_all_by_role_and_label(Role::Button, "New trace tab").any(|_| true) {
        click_role(&mut h, Role::Button, "View");
    }
    assert!(disabled(&h, top_by_label(&h, "New trace tab", Some(Role::Button))), "a fifth trace tab");
    // Reset layout: layout B at this width (1600).
    click_role(&mut h, Role::Button, "Reset layout");
    assert!(has_exact(&h, "Layout B restored"), "\nUI:\n{}", dump(&h));
    assert_eq!(dock::open_traces(h.state().dock.as_ref().unwrap()), vec![1]);
}

/// Layout A below 1600 px, layout B from 1600 px (first start).
#[test]
fn s12_layouts_a_and_b() {
    let tmp = TempDir::new("s12a");
    let h = build_sized(&tmp.data(), 1280.0, 800.0);
    let a = leaves(&h);
    assert_eq!(a.len(), 4, "layout A: {a:?}");
    for want in [vec![Tab::Devices, Tab::Eds], vec![Tab::Od, Tab::Network, Tab::Pdo], vec![Tab::Trace(1), Tab::Messages, Tab::TxList], vec![Tab::Graph]] {
        assert!(a.contains(&want), "layout A lacks {want:?}: {a:?}");
    }
    // The front tab of each leaf is drawn: Devices narrow on the left,
    // the object dictionary right of it, trace and graph below.
    let x = |h: &H, l: &str| rect_of(&node(h, top_by_label(h, l, None))).x0;
    let y = |h: &H, l: &str| rect_of(&node(h, top_by_label(h, l, None))).y0;
    assert!(x(&h, "No devices yet.") < 320.0, "Devices pane too wide\nUI:\n{}", dump(&h));
    assert!(x(&h, "Filter") > 200.0, "object dictionary pane\nUI:\n{}", dump(&h));
    assert!(y(&h, "No signals.") > 400.0, "graph pane below\nUI:\n{}", dump(&h));
    assert!(x(&h, "No signals.") > 640.0, "graph pane right\nUI:\n{}", dump(&h));
    drop(h);
    let tmp = TempDir::new("s12b");
    let h = build_sized(&tmp.data(), 1920.0, 1080.0);
    let b = leaves(&h);
    assert_eq!(b.len(), 5, "layout B: {b:?}");
    for want in [vec![Tab::Devices, Tab::Eds], vec![Tab::Od, Tab::Pdo], vec![Tab::Trace(1), Tab::Messages, Tab::TxList], vec![Tab::Network], vec![Tab::Graph]] {
        assert!(b.contains(&want), "layout B lacks {want:?}: {b:?}");
    }
    let x = |h: &H, l: &str| rect_of(&node(h, top_by_label(h, l, None))).x0;
    assert!(x(&h, "Network overview") > 1100.0, "Network on the right in layout B\nUI:\n{}", dump(&h));
    assert!(x(&h, "No devices yet.") < 320.0, "Devices pane too wide\nUI:\n{}", dump(&h));
}

/// The layout is saved and restored; a broken layout file gives the default.
#[test]
fn s13_layout_save_restore_and_broken_file() {
    let tmp = TempDir::new("s13");
    {
        let mut h = build_sized(&tmp.data(), 1280.0, 800.0);
        view_menu(&mut h, "LSS");
        view_menu(&mut h, "Trace 2");
        view_menu(&mut h, "Graph");
        assert!(h.state().is_open(Tab::Lss) && !h.state().is_open(Tab::Graph));
        h.state_mut().save_layout();
    }
    assert!(tmp.data().join(dock::LAYOUT_FILE).exists());
    {
        let h = build_sized(&tmp.data(), 1280.0, 800.0);
        let s = h.state();
        assert!(s.is_open(Tab::Lss) && s.is_open(Tab::Trace(2)) && !s.is_open(Tab::Graph), "layout not restored: {:?}", leaves(&h));
        assert!(s.traces.contains_key(&2), "trace view 2 not made");
    }
    std::fs::write(tmp.data().join(dock::LAYOUT_FILE), b"{ broken").unwrap();
    let h = build_sized(&tmp.data(), 1280.0, 800.0);
    assert!(has(&h, "default layout used"), "no note\nUI:\n{}", dump(&h));
    let a = leaves(&h);
    assert!(a.contains(&vec![Tab::Od, Tab::Network, Tab::Pdo]) && a.contains(&vec![Tab::Graph]), "default layout A: {a:?}");
}

/// A second trace tab has its own filter and its own pause.
#[test]
fn s14_second_trace_tab_own_filter() {
    let tmp = TempDir::new("s14");
    let mut h = build(&tmp.data());
    side_by_side(&mut h, vec![Tab::Trace(1)], vec![Tab::Trace(2)]);
    connect(&mut h);
    wait_for(&mut h, "rows in both tabs", |h| trace_counts_at(h, true).0 > 1 && trace_counts_at(h, false).0 > 1);
    // Trace 2 shows only COB-ID 0x123 (not on this bus); trace 1 shows all.
    let mid = 800.0;
    open_filters_in(&mut h, mid, f64::MAX);
    let cob = right_of_in(&h, "COB-ID 0x", Role::TextInput, mid, f64::MAX);
    let cob_max = right_of_in(&h, "..", Role::TextInput, mid, f64::MAX);
    set_text(&mut h, cob, "123");
    set_text(&mut h, cob_max, "123");
    wait_for(&mut h, "filter in trace 2", |h| {
        let (s1, b1, _) = trace_counts_at(h, true);
        let (s2, b2, _) = trace_counts_at(h, false);
        s1 == b1 && s1 > 1 && s2 == 0 && b2 > 1
    });
    let left_rows = h.query_all_by_label("701").filter(|n| rect_of(n).x0 < mid).count();
    let right_rows = h.query_all_by_label("701").filter(|n| rect_of(n).x0 >= mid).count();
    assert!(left_rows > 0 && right_rows == 0, "rows {left_rows} / {right_rows}");
    set_text(&mut h, cob, "701");
    set_text(&mut h, cob_max, "701");
    wait_for(&mut h, "trace 2 rows of 0x701", |h| trace_counts_at(h, false).0 > 1);
    // The popup of trace 1 shows its own, empty range.
    open_filters_in(&mut h, f64::MIN, mid);
    let cob1 = right_of_in(&h, "COB-ID 0x", Role::TextInput, f64::MIN, mid);
    assert_eq!(node(&h, cob1).value().unwrap_or_default(), "", "trace 1 filter changed");
    // Pause trace 2 only.
    let pause2 = top_in(&h, "Pause", Some(Role::CheckBox), mid, f64::MAX);
    click_id(&mut h, pause2);
    assert!(h.state().trace_paused(2) && !h.state().trace_paused(1));
    let (_, b1, _) = trace_counts_at(&h, true);
    let (s2, _, _) = trace_counts_at(&h, false);
    wait_for(&mut h, "trace 1 still records", |h| trace_counts_at(h, true).1 > b1);
    assert_eq!(trace_counts_at(&h, false).0, s2, "paused trace 2 changed");
}

/// Graph: a PDO signal gets points from the simulator.
#[test]
fn s15_graph_pdo_signal() {
    let tmp = TempDir::new("s15");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Plot");
    connect(&mut h);
    start_node(&mut h);
    only(&mut h, vec![Tab::Graph]);
    assert!(has_exact(&h, "No signals."));
    click_containing(&mut h, "Add signal...");
    assert!(has_exact(&h, "Add signal"), "dialog\nUI:\n{}", dump(&h));
    // The three sources.
    for s in ["PDO object", "SDO poll", "Raw frame bits"] {
        assert!(has_exact(&h, s), "source {s}");
    }
    assert!(has_exact(&h, "Mapped object"), "mapped objects of the EDS TPDOs\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Add");
    assert!(!has_exact(&h, "Add signal"), "dialog still open\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "N1 6000:01"), "legend row\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "PDO, node 1, 6000:01"));
    let row = |h: &H| top_by_label(h, "N1 6000:01", Some(Role::Label));
    wait_for(&mut h, "points", |h| cell_text(h, row(h), "Points").parse::<u64>().unwrap_or(0) >= 2);
    assert_ne!(cell_text(&h, row(&h), "Last"), "-");
    // Saved with the data folder, and the CSV export.
    assert!(tmp.data().join("signals.json").exists() || std::fs::read_dir(tmp.data()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("signal")), "signal list not saved");
    let csv = tmp.0.join("signals.csv");
    let n = h.state().write_graph_csv(&csv).unwrap();
    assert!(n >= 2, "CSV points {n}");
    // Remove.
    let rm = button_on_row(&h, row(&h), "Remove");
    click_id(&mut h, rm);
    assert!(has_exact(&h, "No signals."));
}

/// PDO: read, disable TPDO1 (apply after the question), it stops in the trace.
#[test]
fn s16_pdo_read_disable_apply() {
    let tmp = TempDir::new("s16");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Pdo");
    connect(&mut h);
    start_node(&mut h);
    only(&mut h, vec![Tab::Trace(1)]);
    wait_for(&mut h, "TPDO1 rows", |h| last_row(h, "181").is_some());
    only(&mut h, vec![Tab::Pdo]);
    assert!(has_exact(&h, "No PDO configuration of node 1."));
    click_containing(&mut h, "Read PDOs");
    wait_for(&mut h, "PDOs read", |h| has(h, "Node 1: ") && has(h, "PDO(s) read"));
    let t1 = top_by_label(&h, "TPDO1", Some(Role::Label));
    assert_eq!(cell_text(&h, t1, "COB-ID"), "181");
    assert_eq!(cell_text(&h, t1, "State"), "enabled");
    let before = buttons(&h, "Apply");
    let dis = button_on_row(&h, t1, "Disable");
    click_id(&mut h, dis);
    assert!(has_exact(&h, "Apply PDO change"), "question before the apply\nUI:\n{}", dump(&h));
    click_new_button(&mut h, "Apply", &before);
    wait_for(&mut h, "applied", |h| has(h, "PDO change applied"));
    let t1 = top_by_label(&h, "TPDO1", Some(Role::Label));
    assert_eq!(cell_text(&h, t1, "State"), "disabled");
    // No more 0x181 frames.
    only(&mut h, vec![Tab::Trace(1)]);
    run_for(&mut h, Duration::from_millis(300));
    click(&mut h, "Clear");
    wait_for(&mut h, "heartbeats after the clear", |h| last_row(h, "701").is_some());
    run_for(&mut h, Duration::from_millis(500));
    assert!(last_row(&h, "181").is_none(), "TPDO1 still sent\nUI:\n{}", dump(&h));
    assert!(last_row(&h, "281").is_some() || trace_counts(&h).0 > 0);
}

/// PDO: an apply that fails part-way (an object the node cannot map) leaves
/// TPDO1 disabled: an error banner with Read PDOs, and the row is marked.
#[test]
fn s22_pdo_failed_apply_banner() {
    let tmp = TempDir::new("s22");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Pdo");
    connect(&mut h);
    only(&mut h, vec![Tab::Pdo]);
    click_containing(&mut h, "Read PDOs");
    wait_for(&mut h, "PDOs read", |h| has(h, "PDO(s) read"));
    let t1 = top_by_label(&h, "TPDO1", Some(Role::Label));
    assert_eq!(cell_text(&h, t1, "State"), "enabled");
    let edit = button_on_row(&h, t1, "Edit");
    click_id(&mut h, edit);
    // The entry field left of "Add entry": 0x1000 (device type) is not mappable.
    let add = rect_id(&h, top_by_label(&h, "Add entry", Some(Role::Button)));
    let field = h
        .query_all_by_role(Role::TextInput)
        .filter(|n| (cy(&rect_of(n)) - cy(&add)).abs() < 8.0 && rect_of(n).x1 <= add.x0 + 1.0)
        .max_by(|a, b| rect_of(a).x0.total_cmp(&rect_of(b).x0))
        .map(|n| n.id())
        .expect("entry field");
    set_text(&mut h, field, "1000:0:8");
    click_role(&mut h, Role::Button, "Add entry");
    // The PDO check finds an error (the length; not mappable per the EDS is
    // a warning): Apply asks first.
    assert!(has_exact(&h, "PDO check: 1 error(s), 1 warning(s)"), "findings\nUI:\n{}", dump(&h));
    let before_close = buttons(&h, "Close");
    click_role(&mut h, Role::Button, "Apply");
    assert!(has_exact(&h, "PDO mapping has errors"), "error question\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Write anyway");
    wait_for(&mut h, "apply failed", |h| has(h, "PDO change failed"));
    click_new_button(&mut h, "Close", &before_close);
    assert!(has(&h, "TPDO1 was left disabled on the device after a failed step: "), "banner\nUI:\n{}", dump(&h));
    let t1 = top_by_label(&h, "TPDO1", Some(Role::Label));
    assert_eq!(cell_text(&h, t1, "State"), "left disabled");
    // The banner action reads the PDOs again; the banner goes.
    click_role(&mut h, Role::Button, "Read PDOs");
    wait_for(&mut h, "read again", |h| !has(h, "left disabled on the device"));
    let t1 = top_by_label(&h, "TPDO1", Some(Role::Label));
    assert_eq!(cell_text(&h, t1, "State"), "disabled");
}

/// Transmit list: a manual send appears in the trace; a periodic entry runs
/// only while started.
#[test]
fn s17_transmit_list() {
    let tmp = TempDir::new("s17");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::TxList]);
    assert!(has(&h, "No frames in the transmit list."));
    connect(&mut h);
    let id = right_of(&h, "ID 0x", Role::TextInput);
    set_text(&mut h, id, "222");
    let data = right_of(&h, "Data", Role::TextInput);
    set_text(&mut h, data, "AA BB");
    click(&mut h, "Manual");
    click(&mut h, "Add to list");
    wait_for(&mut h, "entry", |h| has_exact(h, "stopped") || has_exact(h, "manual"));
    let row = top_by_label(&h, "222", None);
    let send = button_on_row(&h, row, "Send");
    click_id(&mut h, send);
    only(&mut h, vec![Tab::Trace(1)]);
    wait_for(&mut h, "Tx row 222", |h| last_row(h, "222").is_some());
    let r = last_row(&h, "222").unwrap();
    assert_eq!(cell_text(&h, r, "Data"), "AA BB");
    assert_eq!(cell_text(&h, r, "Dir"), tx_glyph());
    // A periodic entry sends nothing until Start, and stops on Stop.
    only(&mut h, vec![Tab::TxList]);
    set_text(&mut h, id, "333");
    click(&mut h, "Periodic");
    click(&mut h, "Add to list");
    run_for(&mut h, Duration::from_millis(300));
    assert!(cob_count(&h, 0x333) == 0, "periodic entry sent before Start");
    let row = top_by_label(&h, "333", None);
    let start = button_on_row(&h, row, "Start");
    click_id(&mut h, start);
    wait_for(&mut h, "running", |h| has_exact(h, "running"));
    wait_for(&mut h, "periodic frames", |h| cob_count(h, 0x333) >= 2);
    click(&mut h, "Stop all");
    run_for(&mut h, Duration::from_millis(200));
    let n = cob_count(&h, 0x333);
    run_for(&mut h, Duration::from_millis(400));
    assert_eq!(cob_count(&h, 0x333), n, "periodic frames after Stop all");
    // The list is saved.
    assert!(tmp.data().join("txlist.json").exists());
}

/// LSS: inquire the identity of the simulated slave.
#[test]
fn s18_lss_inquire() {
    let tmp = TempDir::new("s18");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Lss]);
    assert!(has_exact(&h, "Not connected: LSS services are disabled."));
    assert!(has_exact(&h, "No LSS service run yet."));
    connect(&mut h);
    click(&mut h, "Switch all to configuration");
    wait_for(&mut h, "switched", |h| h.state().lssv.running.is_none() && !h.state().lssv.log.is_empty());
    click(&mut h, "Inquire identity");
    wait_for(&mut h, "identity", |h| has(h, "vendor 0x0000069B"));
    assert!(has(&h, "product 0x00000001"), "\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "Use last identity"));
    // The identity fields are filled, as 8 hex digits with 0x.
    let field = |h: &H, l: &str| node(h, right_of(h, l, Role::TextInput)).value().unwrap_or_default();
    assert_eq!(field(&h, "Vendor ID"), "0x0000069B");
    assert_eq!(field(&h, "Product code"), "0x00000001");
    assert!(field(&h, "Revision").starts_with("0x") && field(&h, "Serial").len() == 10, "\nUI:\n{}", dump(&h));
    let current = |h: &H| text_of(&node(h, right_of(h, "Current node-ID", Role::Label)));
    assert_eq!(current(&h), "\u{2014}");
    click(&mut h, "Inquire node-ID");
    wait_for(&mut h, "node-ID", |h| has(h, ": node-ID 1"));
    assert_eq!(current(&h), "1");
    // The log table: service and result pill.
    assert!(has_exact(&h, "Inquire identity") && has_exact(&h, "OK"), "\nUI:\n{}", dump(&h));
    // Caution / danger buttons.
    for l in ["Configure node-ID", "Configure bit timing", "Activate bit timing"] {
        assert!(h.query_all_by_role_and_label(Role::Button, l).any(|n| !n.is_disabled()), "{l}\nUI:\n{}", dump(&h));
    }
    click(&mut h, "Configure node-ID");
    assert!(has(&h, "It may leave this network."), "question\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Cancel");
    // A changing service asks first.
    click(&mut h, "Store configuration");
    assert!(has(&h, "non-volatile memory"), "question\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Cancel");
    assert!(has_exact(&h, "Cancelled; nothing sent"));
}

/// DCF: save to a temporary file, compare: no difference.
#[test]
fn s19_dcf_save_compare() {
    let tmp = TempDir::new("s19");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Dcf");
    only(&mut h, vec![Tab::Dcf]);
    connect(&mut h);
    let path = tmp.0.join("node1.dcf");
    h.state_mut().dcf_save_to(path.clone()).unwrap();
    wait_for(&mut h, "saved", |h| has(h, "Saved ") && has(h, "value(s) to"));
    assert!(path.exists());
    h.state_mut().dcf_load(&path);
    step2(&mut h);
    click(&mut h, "Compare");
    wait_for(&mut h, "compared", |h| has(h, "Compared "));
    assert!(has(&h, ": 0 difference(s)"), "\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "no difference"));
    // Dry run sends no write.
    click(&mut h, "Dry run");
    wait_for(&mut h, "dry run", |h| has(h, "Dry run: "));
    assert!(has(&h, "Dry run: 0 object(s) would be written"), "\nUI:\n{}", dump(&h));
    // Restore asks first.
    click(&mut h, "Restore");
    assert!(has_exact(&h, "Restore DCF"), "question\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Cancel");
    assert!(has_exact(&h, "Cancelled; nothing sent"));
}

/// Settings: change, save, reload.
#[test]
fn s20_settings_change_save_reload() {
    let tmp = TempDir::new("s20");
    {
        let mut h = build(&tmp.data());
        only(&mut h, vec![Tab::Od]);
        click_role(&mut h, Role::Button, "Settings");
        // (The section buttons carry an icon in front of the name.)
        assert!(has(&h, "  Appearance") && has(&h, "  Safety") && has(&h, "  Data folder"), "\nUI:\n{}", dump(&h));
        click_role(&mut h, Role::Button, "Light");
        click_role(&mut h, Role::Button, "Delta");
        // The draft shows at once, the settings change only on Save.
        assert_eq!(h.state().settings.theme, Settings::default().theme);
        click_role(&mut h, Role::Button, "Save");
        assert!(has_exact(&h, "Settings saved"), "\nUI:\n{}", dump(&h));
        assert!(!has(&h, "  Appearance"), "dialog still open");
        assert_eq!(h.state().settings.theme, Theme::Light);
        assert!(!h.ctx.style().visuals.dark_mode);
        // Cancel keeps the saved values.
        click_role(&mut h, Role::Button, "Settings");
        click_role(&mut h, Role::Button, "Dark");
        click_role(&mut h, Role::Button, "Cancel");
        assert_eq!(h.state().settings.theme, Theme::Light);
    }
    let (s, _) = Settings::load(&tmp.data());
    assert_eq!(s.theme, Theme::Light);
    assert_eq!(s.timestamp_format, TimestampFormat::Delta);
    let mut h = build(&tmp.data());
    step2(&mut h);
    assert_eq!(h.state().settings.theme, Theme::Light);
    assert!(!h.ctx.style().visuals.dark_mode);
    only(&mut h, vec![Tab::Trace(1)]);
    assert!(h.state().trace_delta(1), "new trace tab uses the Delta default\nUI:\n{}", dump(&h));
}

/// A write asks first; Cancel sends nothing, Write sends it.
#[test]
fn s21_confirm_before_write() {
    let tmp = TempDir::new("s21");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Od]);
    connect(&mut h);
    let index = right_of(&h, "Index 0x", Role::TextInput);
    let sub = right_of(&h, "Sub", Role::TextInput);
    set_text(&mut h, index, "1017");
    set_text(&mut h, sub, "0");
    let value = right_of(&h, "Value", Role::TextInput);
    set_text(&mut h, value, "500");
    let before = buttons(&h, "Write");
    let id = top_by_label(&h, "Write", Some(Role::Button));
    click_id(&mut h, id);
    assert!(has_exact(&h, "Write object"), "no question\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Cancel");
    assert!(has_exact(&h, "Cancelled; nothing sent"));
    run_for(&mut h, Duration::from_millis(200));
    assert!(cob_count(&h, 0x601) == 0, "an SDO request was sent after Cancel");
    let id = top_by_label(&h, "Write", Some(Role::Button));
    click_id(&mut h, id);
    click_new_button(&mut h, "Write", &before);
    wait_for(&mut h, "written", |h| has(h, "Written node 1 0x1017:00"));
    assert!(cob_count(&h, 0x601) > 0);
}

// ------------------------------------------------- v0.3: trace scrolling

/// Push `count` frames with COB-ID `id` straight into the trace store; frame
/// `i` has the time stamp `i` ms (shown as `0.00i000`).
fn push_frames(h: &mut H, id: u32, from: u64, count: u64) {
    let ctx = crate::canopen::decode::DecodeContext::default();
    let app = h.state_mut();
    for i in from..from + count {
        let mut f = crate::can::CanFrame::new(id, &[(i & 0xFF) as u8, (i >> 8) as u8]);
        f.timestamp_us = i * 1000;
        let d = crate::canopen::decode::decode(&f, &ctx);
        app.trace.push(f, d);
    }
}

/// The Time cells of the drawn trace rows, top to bottom.
fn visible_times(h: &H) -> Vec<String> {
    let mut v: Vec<(f64, String)> = h
        .query_all_by_role(Role::Label)
        .map(|n| (rect_of(&n).y0, text_of(&n)))
        .filter(|(_, t)| t.len() > 7 && t.as_bytes()[t.len() - 7] == b'.' && t.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|x| x.1).collect()
}

fn time_of(ms: u64) -> String {
    format!("{:.6}", ms as f64 / 1000.0)
}

/// Move the pointer over the trace table (over the newest drawn row).
fn pointer_over_table(h: &mut H) {
    let id = h.query_all_by_role(Role::Label).filter(|n| visible_times_one(&text_of(n))).map(|n| n.id()).next_back().expect("a trace row");
    let r = rect_id(h, id);
    let pos = egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0 - 40.0) as f32);
    h.input_mut().events.push(egui::Event::PointerMoved(pos));
    h.step();
}

fn visible_times_one(t: &str) -> bool {
    t.len() > 7 && t.as_bytes()[t.len() - 7] == b'.' && t.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn press(h: &mut H, key: egui::Key) {
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE });
    }
    step2(h);
}

/// Auto-scroll: stays at the newest row; scrolling up switches it off and
/// keeps the rows in place (also while the full buffer drops old rows); on
/// again jumps to the newest row; keys; pause; filter and Clear.
#[test]
fn s23_trace_autoscroll() {
    let tmp = TempDir::new("s23");
    let mut h = build_sized(&tmp.data(), 1200.0, 700.0);
    only(&mut h, vec![Tab::Trace(1)]);
    h.state_mut().trace.set_capacity(1000);
    h.state_mut().settings.timestamp_us = true;
    let auto = |h: &H| toggled(h, top_by_label(h, "Auto-scroll", Some(Role::CheckBox)));
    let last = |h: &H| visible_times(h).last().cloned().unwrap_or_default();
    push_frames(&mut h, 0x100, 0, 500);
    step2(&mut h);
    assert!(auto(&h) && h.state().trace_autoscroll(1), "auto-scroll on by default");
    assert_eq!(last(&h), time_of(499), "\nUI:\n{}", dump(&h));
    push_frames(&mut h, 0x100, 500, 20);
    step2(&mut h);
    assert_eq!(last(&h), time_of(519), "follows new rows");

    // The wheel up: auto-scroll off, the button shows it.
    pointer_over_table(&mut h);
    h.input_mut().events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, 600.0), modifiers: egui::Modifiers::NONE });
    run_for(&mut h, Duration::from_millis(400));
    assert!(!auto(&h), "auto-scroll still on after the wheel\nUI:\n{}", dump(&h));
    assert_ne!(last(&h), time_of(519));
    let top = visible_times(&h)[0].clone();
    // New rows: no drift.
    push_frames(&mut h, 0x100, 520, 280);
    step2(&mut h);
    assert_eq!(visible_times(&h)[0], top, "rows moved while auto-scroll is off");
    // The full buffer drops old rows: the rows stay.
    push_frames(&mut h, 0x100, 800, 600);
    step2(&mut h);
    assert_eq!(h.state().trace.len(), 1000);
    assert_eq!(visible_times(&h)[0], top, "rows moved while the buffer dropped old rows");
    assert!(!auto(&h));

    // On again: the newest row.
    click_role(&mut h, Role::CheckBox, "Auto-scroll");
    assert!(auto(&h));
    assert_eq!(last(&h), time_of(1399));

    // Keys over the table: Page Up off, End on.
    pointer_over_table(&mut h);
    press(&mut h, egui::Key::PageUp);
    assert!(!auto(&h), "Page Up did not switch auto-scroll off");
    assert_ne!(last(&h), time_of(1399));
    press(&mut h, egui::Key::End);
    assert!(auto(&h), "End did not switch auto-scroll on");
    assert_eq!(last(&h), time_of(1399));

    // Pause freezes the rows; resume follows the toggle (on).
    click_role(&mut h, Role::CheckBox, "Pause");
    push_frames(&mut h, 0x100, 1400, 10);
    step2(&mut h);
    assert_eq!(last(&h), time_of(1399), "rows added while paused");
    click_role(&mut h, Role::CheckBox, "Pause");
    step2(&mut h);
    assert_eq!(last(&h), time_of(1409), "resume did not go to the newest row");

    // A filter (COB-ID 0x100 only): still the newest matching row.
    open_filters(&mut h);
    let lo = right_of(&h, "COB-ID 0x", Role::TextInput);
    let hi = right_of(&h, "..", Role::TextInput);
    set_text(&mut h, lo, "100");
    set_text(&mut h, hi, "100");
    h.input_mut().events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    for i in 1410..1460 {
        push_frames(&mut h, if i % 2 == 0 { 0x100 } else { 0x200 }, i, 1);
    }
    step2(&mut h);
    assert_eq!(last(&h), time_of(1458), "filtered view not at the newest row\nUI:\n{}", dump(&h));
    // Clear, then new rows: at the newest again.
    click(&mut h, "Clear");
    push_frames(&mut h, 0x100, 2000, 300);
    step2(&mut h);
    assert!(auto(&h));
    assert_eq!(last(&h), time_of(2299));
}

// ------------------------------------------- v0.3: scan, RPDO, Messages

/// A real pointer click at the centre of a node (for a table row, which
/// takes the click of the label on it).
fn pointer_click(h: &mut H, id: NodeId) {
    let r = rect_id(h, id);
    let pos = egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32);
    h.input_mut().events.push(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    step2(h);
}

/// The connection bar's Scan nodes: disabled when not connected and in
/// listen-only mode; it runs the Network scan, shows the progress and a
/// cancel, and brings the Network tab to the front.
#[test]
fn s24_scan_button_in_the_connection_bar() {
    let tmp = TempDir::new("s24");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Graph]);
    all_disabled(&h, "Scan nodes");
    click_role(&mut h, Role::CheckBox, "Listen only");
    connect(&mut h);
    all_disabled(&h, "Scan nodes");
    click(&mut h, "Disconnect");
    wait_for(&mut h, "disconnected", |h| has_exact(h, "Connect"));
    click_role(&mut h, Role::CheckBox, "Listen only");
    connect(&mut h);
    assert!(!disabled(&h, top_by_label(&h, "Scan nodes", Some(Role::Button))));
    assert!(!leaves(&h).iter().flatten().any(|t| *t == Tab::Network));
    click_role(&mut h, Role::Button, "Scan nodes");
    assert!(leaves(&h).iter().flatten().any(|t| *t == Tab::Network), "Network tab not opened");
    assert!(has(&h, "SDO read of 0x1000 on nodes"), "Network tab not in front\nUI:\n{}", dump(&h));
    wait_for(&mut h, "progress", |h| has_exact(h, "Cancel scan") && h.state().scan_progress().is_some());
    // The Network view shows its own cancel for the same scan.
    assert!(has_exact(&h, "Cancel"));
    wait_for(&mut h, "node 1 answered", |h| has_exact(h, "Node 1") && has(h, "device type 0x"));
    click(&mut h, "Cancel scan");
    wait_for(&mut h, "cancelled", |h| has(h, "Scan cancelled"));
    assert!(has_exact(&h, "Scan nodes"));
}

/// An RPDO object of the node is offered in Add signal (named as in the
/// trace) and decoded from the frames that the transmit list sends.
#[test]
fn s25_graph_rpdo_signal() {
    let tmp = TempDir::new("s25");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Rx");
    connect(&mut h);
    only(&mut h, vec![Tab::Graph]);
    click_containing(&mut h, "Add signal...");
    let combo = right_of(&h, "Mapped object", Role::ComboBox);
    let before: Vec<NodeId> = h.query_all_by_role(Role::Button).map(|n| n.id()).collect();
    click_id(&mut h, combo);
    let opt = h.query_all_by_label_contains("RPDO1 6200:01").find(|n| !before.contains(&n.id()) && n.role() != Role::Label).map(|n| n.id()).unwrap_or_else(|| panic!("no RPDO option\nUI:\n{}", dump(&h)));
    click_id(&mut h, opt);
    let name = right_of(&h, "Name", Role::TextInput);
    let auto = node(&h, name).value().unwrap_or_default();
    assert!(auto.starts_with("N1 RPDO1 6200:01"), "automatic name {auto:?}");
    assert!(has(&h, "RPDO: decoded from the frames"), "RPDO note\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Add");
    assert!(!has_exact(&h, "Add signal"), "dialog still open\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "PDO, node 1, 6200:01"));
    // The transmit list sends RPDO1 (0x201) to the node.
    only(&mut h, vec![Tab::TxList]);
    let id = right_of(&h, "ID 0x", Role::TextInput);
    set_text(&mut h, id, "201");
    let data = right_of(&h, "Data", Role::TextInput);
    set_text(&mut h, data, "5A 00");
    click(&mut h, "Manual");
    click(&mut h, "Add to list");
    let row = top_by_label(&h, "201", None);
    let send = button_on_row(&h, row, "Send");
    click_id(&mut h, send);
    // The trace names the RPDO and its signals the same way.
    only(&mut h, vec![Tab::Trace(1)]);
    wait_for(&mut h, "Tx row 201", |h| last_row(h, "201").is_some());
    let r = last_row(&h, "201").unwrap();
    assert_eq!(cell_text(&h, r, "Type"), "RPDO1");
    only(&mut h, vec![Tab::Graph]);
    let row = |h: &H| h.query_all_by_label_contains("N1 RPDO1 6200:01").find(|n| n.role() == Role::Label).map(|n| n.id()).expect("legend row");
    wait_for(&mut h, "a point", |h| cell_text(h, row(h), "Points").parse::<u64>().unwrap_or(0) >= 1);
    assert_eq!(cell_text(&h, row(&h), "Last"), "90");
}

/// The Add signal window moves by its title bar.
#[test]
fn s26_add_signal_window_moves() {
    let tmp = TempDir::new("s26");
    let mut h = build_sized(&tmp.data(), 1200.0, 800.0);
    only(&mut h, vec![Tab::Graph]);
    click_containing(&mut h, "Add signal...");
    let win = |h: &H| rect_id(h, top_by_label(h, "Add signal", Some(Role::Window)));
    let r0 = win(&h);
    // A point on the title bar.
    let from = egui::pos2((r0.x0 + 80.0) as f32, (r0.y0 + 12.0) as f32);
    let to = from + egui::vec2(-200.0, -150.0);
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    for i in 1..=10 {
        h.input_mut().events.push(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 10.0)));
        h.step();
    }
    h.input_mut().events.push(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    let r1 = win(&h);
    assert!(r1.x0 < r0.x0 - 150.0 && r1.y0 < r0.y0 - 100.0, "window did not move: {r0:?} -> {r1:?}");
    // Dragged far out: it stays inside the app window.
    let from = egui::pos2((r1.x0 + 80.0) as f32, (r1.y0 + 12.0) as f32);
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    let far = egui::pos2(-900.0, -900.0);
    h.input_mut().events.push(egui::Event::PointerMoved(far));
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton { pos: far, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    let r2 = win(&h);
    assert!(r2.x0 >= 0.0 && r2.y0 >= 0.0, "window left the app window: {r2:?}");
    // Escape closes it.
    h.input_mut().events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    assert!(!has_exact(&h, "Add signal"), "Escape did not close\nUI:\n{}", dump(&h));
}

/// Push a frame straight into the Messages statistics (as the bus would).
fn msg_frame(h: &mut H, id: u32, data: &[u8], t_us: u64) {
    let mut f = crate::can::CanFrame::new(id, data);
    f.timestamp_us = t_us;
    let app = h.state_mut();
    let d = crate::canopen::decode::decode(&f, &app.decode_ctx);
    app.msgs.on_frame(&f, &d);
    let l = app.msgs.latest_us();
    app.msgv.on_frames(l);
}

/// Messages: opens from the View menu beside Trace 1; one row per COB-ID
/// from the simulator; filter; selection with the detail pane; the change
/// marks (reduced colour) up and down, which fade and which "Mute noisy
/// bytes" hides for a counter; Add to Graph; reset.
#[test]
fn s27_messages_view() {
    let tmp = TempDir::new("s27");
    let mut h = build_sized(&tmp.data(), 1400.0, 900.0);
    only(&mut h, vec![Tab::Trace(1)]);
    view_menu(&mut h, "Messages");
    h.input_mut().events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    assert_eq!(leaves(&h), vec![vec![Tab::Trace(1), Tab::Messages]], "Messages beside Trace 1");
    assert!(has_exact(&h, "No frames yet. Connect to see one row per COB-ID."), "\nUI:\n{}", dump(&h));
    connect(&mut h);
    wait_for(&mut h, "heartbeat row", |h| has_exact(h, "701"));
    let row = top_by_label(&h, "701", None);
    wait_for(&mut h, "count", |h| cell_text(h, top_by_label(h, "701", None), "Count").parse::<u64>().unwrap_or(0) >= 1);
    assert_eq!(cell_text(&h, row, "DLC"), "1");
    click(&mut h, "Disconnect");
    wait_for(&mut h, "disconnected", |h| has_exact(h, "Connect"));

    // Own frames: byte 0 up, byte 1 down.
    h.state_mut().settings.reduce_colour = true;
    let t = h.state().msgs.latest_us() + 1000;
    msg_frame(&mut h, 0x3A0, &[0x10, 0x20, 0x55], t);
    msg_frame(&mut h, 0x3A0, &[0x11, 0x1F, 0x55], t + 1000);
    step2(&mut h);
    assert!(has_exact(&h, "3A0"));
    assert!(has_exact(&h, icon::CARET_UP) && has_exact(&h, icon::CARET_DOWN), "change marks\nUI:\n{}", dump(&h));
    let r = top_by_label(&h, "3A0", None);
    assert_eq!(cell_text(&h, r, "B0"), "11");
    assert_eq!(cell_text(&h, r, "B2"), "55");
    // Filter.
    let f = h.query_all_by_role(Role::TextInput).next().map(|n| n.id()).expect("filter field");
    set_text(&mut h, f, "3a0");
    assert!(has_exact(&h, "3A0") && !has_exact(&h, "701"), "filter\nUI:\n{}", dump(&h));
    set_text(&mut h, f, "");
    assert!(has_exact(&h, "701"));
    // The marks fade.
    h.state_mut().settings.msg_fade_s = 0.2;
    run_for(&mut h, Duration::from_millis(400));
    assert!(!has_exact(&h, icon::CARET_UP) && !has_exact(&h, icon::CARET_DOWN), "marks did not fade");
    // A counter byte: marked, unless muted.
    // (5 s later on the frame clock: the older changes have faded.)
    h.state_mut().settings.msg_fade_s = 2.0;
    for i in 0..12u64 {
        msg_frame(&mut h, 0x3A0, &[i as u8, 0x1F, 0x55], t + 5_000_000 + i * 1000);
    }
    step2(&mut h);
    assert!(has_exact(&h, icon::CARET_UP) || has_exact(&h, icon::CARET_DOWN), "counter not marked");
    click_role(&mut h, Role::CheckBox, "Mute noisy bytes");
    assert!(h.state().settings.msg_mute_noisy);
    assert!(!has_exact(&h, icon::CARET_UP) && !has_exact(&h, icon::CARET_DOWN), "noisy byte still marked\nUI:\n{}", dump(&h));

    // Select the row: the detail pane with the bit grid.
    let r = top_by_label(&h, "3A0", None);
    pointer_click(&mut h, r);
    assert!(has_exact(&h, "Reset statistics"), "detail pane\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "Byte 0") && has_exact(&h, "Byte 2") && !has_exact(&h, "Byte 3"));
    assert!(disabled(&h, top_by_label(&h, "Add to Graph", Some(Role::Button))));
    click(&mut h, "Byte 1");
    assert!(has_exact(&h, "Selected: bit 8, 8 bit"), "selection\nUI:\n{}", dump(&h));
    click(&mut h, "Add to Graph");
    assert!(has(&h, "Signal 3A0 bits 8+8 added to the Graph"), "\nUI:\n{}", dump(&h));
    let sigs: Vec<crate::signals::SignalSource> = h.state().bus.signals.lock().unwrap().list().iter().map(|s| s.def.source.clone()).collect();
    assert!(sigs.iter().any(|s| matches!(s, crate::signals::SignalSource::Raw { cob_id: 0x3A0, start_bit: 8, bits: 8, extended: false, .. })), "{sigs:?}");
    // Reset the row, then all.
    let key = crate::msgstats::MsgKey { id: 0x3A0, extended: false };
    assert!(h.state().msgs.get(key).unwrap().count > 1);
    click(&mut h, "Reset statistics");
    assert_eq!(h.state().msgs.get(key).unwrap().count, 0);
    click(&mut h, "Reset all statistics");
    assert!(h.state().msgs.is_empty());
    assert!(has_exact(&h, "No frames yet. Connect to see one row per COB-ID."));
}

// ------------------------------------------------------------ screenshots

/// Render PNG screenshots with wgpu (offscreen) to `target\screens\`.
/// Run: `cargo test screenshots -- --ignored`.
#[test]
#[ignore]
fn screenshots() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screens");
    std::fs::create_dir_all(&out).unwrap();
    let shot = |h: &mut H, name: &str| {
        step2(h);
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render of {name} failed: {e}"));
        img.save(out.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}.png not written: {e}"));
    };
    let scene = |w: f32, ht: f32, theme: Theme, name: &str| -> (TempDir, H) {
        let tmp = TempDir::new(name);
        preset(&tmp.data(), Settings { theme, ..Settings::default() });
        let boot = Boot::load_dir(tmp.data());
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(w, ht)).wgpu().build_eframe(move |cc| App::new(cc, boot));
        h.state_mut().first_layout = false;
        // Traffic with an EDS device, an operational node and a graph signal.
        add_eds_and_device(&mut h, "Bench");
        connect(&mut h);
        start_node(&mut h);
        h.state_mut().add_signal(crate::signals::SignalDef::new("Analog input 1", crate::signals::SignalSource::Pdo { node: 1, index: 0x6401, sub: 1 }));
        h.state_mut().add_signal(crate::signals::SignalDef::new("Inputs 1-8", crate::signals::SignalSource::Pdo { node: 1, index: 0x6000, sub: 1 }));
        let width = w;
        h.state_mut().reset_layout(width);
        run_for(&mut h, Duration::from_millis(1500));
        (tmp, h)
    };
    {
        let (_t, mut h) = scene(1280.0, 800.0, Theme::Dark, "shot_a_dark");
        shot(&mut h, "layout_a_1280x800_dark");
        h.state_mut().open_settings();
        shot(&mut h, "settings_dark");
        h.state_mut().settings_dlg = None;
        h.state_mut().read_pdos(1);
        wait_for(&mut h, "PDOs read", |h| has(h, "PDO(s) read"));
        only(&mut h, vec![Tab::Pdo]);
        shot(&mut h, "pdo_dark");
        only(&mut h, vec![Tab::Network]);
        shot(&mut h, "network_dark");
        only(&mut h, vec![Tab::Lss]);
        click(&mut h, "Switch all to configuration");
        click(&mut h, "Inquire identity");
        wait_for(&mut h, "identity", |h| has(h, "vendor 0x"));
        shot(&mut h, "lss_dark");
        only(&mut h, vec![Tab::Dcf]);
        let path = out.join("node1_sample.dcf");
        h.state_mut().dcf_save_to(path.clone()).unwrap();
        wait_for(&mut h, "saved", |h| has(h, "value(s) to"));
        h.state_mut().dcf_load(&path);
        click(&mut h, "Compare");
        wait_for(&mut h, "compared", |h| has(h, "Compared "));
        shot(&mut h, "dcf_dark");
        let _ = std::fs::remove_file(&path);
    }
    // Messages (0.3): the simulator running, a TPDO row selected with a
    // byte selected in the bit grid, tints visible.
    let messages = |h: &mut H, name: &str| {
        only(h, vec![Tab::Trace(1), Tab::Messages]);
        h.state_mut().open_tab(Tab::Messages);
        step2(h);
        wait_for(h, "TPDO1 row", |h| has_exact(h, "181"));
        let r = top_by_label(h, "181", None);
        pointer_click(h, r);
        click(h, "Byte 0");
        // Shoot while a change tint is still visible.
        let t0 = Instant::now();
        loop {
            step2(h);
            let now = h.state().msgv_now_us();
            let key = crate::msgstats::MsgKey { id: 0x181, extended: false };
            let fresh = h.state().msgs.get(key).is_some_and(|e| e.bytes.iter().any(|b| b.last_change_us.is_some_and(|c| now.saturating_sub(c) < 600_000)));
            if fresh || t0.elapsed() > WAIT {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        shot(h, name);
    };
    {
        let (_t, mut h) = scene(1280.0, 800.0, Theme::Dark, "shot_msg_dark");
        messages(&mut h, "messages_dark");
    }
    {
        let (_t, mut h) = scene(1280.0, 800.0, Theme::Light, "shot_a_light");
        shot(&mut h, "layout_a_1280x800_light");
        messages(&mut h, "messages_light");
    }
    // The connection bar with Scan nodes, then with a running scan.
    {
        let tmp = TempDir::new("shot_bar");
        preset(&tmp.data(), Settings { theme: Theme::Dark, ..Settings::default() });
        let boot = Boot::load_dir(tmp.data());
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(1280.0, 120.0)).wgpu().build_eframe(move |cc| App::new(cc, boot));
        h.state_mut().first_layout = false;
        only(&mut h, vec![Tab::Network]);
        connect(&mut h);
        shot(&mut h, "connection_bar_dark");
        click_role(&mut h, Role::Button, "Scan nodes");
        wait_for(&mut h, "progress", |h| h.state().scan_progress().is_some_and(|(d, _)| d >= 3));
        shot(&mut h, "connection_bar_scan_dark");
        click(&mut h, "Cancel scan");
    }
    {
        let (_t, mut h) = scene(1920.0, 1080.0, Theme::Dark, "shot_b_dark");
        shot(&mut h, "layout_b_1920x1080_dark");
    }
    shots_v03(&out);
}

/// The bit view of the Messages tab with one changed bit lit (byte 0 bit 3,
/// and the Hex cell of byte 0), 0.5 s after the change on a fixed clock,
/// dark and light: `cargo test screenshots_bit_flash -- --ignored`.
#[test]
#[ignore]
fn screenshots_bit_flash() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screens");
    std::fs::create_dir_all(&out).unwrap();
    for (name, theme) in [("bit_flash_dark", Theme::Dark), ("bit_flash_light", Theme::Light)] {
        let tmp = TempDir::new(name);
        preset(&tmp.data(), Settings { theme, ..Settings::default() });
        let boot = Boot::load_dir(tmp.data());
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(1280.0, 800.0)).wgpu().build_eframe(move |cc| App::new(cc, boot));
        h.state_mut().first_layout = false;
        only(&mut h, vec![Tab::Messages]);
        let t = 10_000_000;
        msg_frame(&mut h, 0x1E4, &[0x00, 0x5A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], t);
        msg_frame(&mut h, 0x1E4, &[0x08, 0x5A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], t + 10_000);
        h.state_mut().msgv.set_fixed_clock(Some(t + 510_000));
        step2(&mut h);
        let r = top_by_label(&h, "1E4", None);
        pointer_click(&mut h, r);
        step2(&mut h);
        let key = crate::msgstats::MsgKey { id: 0x1E4, extended: false };
        let e = h.state().msgs.get(key).unwrap();
        assert_eq!(e.bit_flash(3, t + 510_000), 1.0);
        assert_eq!(e.bit_flash(4, t + 510_000), 0.0);
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render of {name} failed: {e}"));
        img.save(out.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}.png not written: {e}"));
    }
}

/// Only the screenshots of 0.3: `cargo test screenshots_v03 -- --ignored`.
#[test]
#[ignore]
fn screenshots_v03() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screens");
    std::fs::create_dir_all(&out).unwrap();
    shots_v03(&out);
}

/// The EDS info grid (one long value that wraps) in a narrow tab, a wide
/// tab, a narrow tab at 125 % scale and a short window that scrolls, each
/// after a switch from another EDS: `cargo test screenshots_eds -- --ignored`.
#[test]
#[ignore]
fn screenshots_eds_info() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screens");
    std::fs::create_dir_all(&out).unwrap();
    for (name, frac, ppp, ht) in [("eds_info_dark", 0.61, 1.0, 800.0), ("eds_info_wide_dark", 0.2, 1.0, 800.0), ("eds_info_125_dark", 0.61, 1.25, 800.0), ("eds_info_short_dark", 0.61, 1.0, 520.0)] {
        let tmp = TempDir::new(name);
        preset(&tmp.data(), Settings { theme: Theme::Dark, ..Settings::default() });
        // The built-in simulator EDS with one long value that must wrap.
        let eds = tmp.0.join("sim_long_info.eds");
        let text = crate::can::sim::BUILTIN_EDS
            .replace("FileName=sim_builtin.eds", "FileName=sim_long_info.eds")
            .replace("CreatedBy=CANoxide", "CreatedBy=CANoxide test fixture, written by the screenshot test with a long value that wraps to more than one line");
        std::fs::write(&eds, text).unwrap();
        let boot = Boot::load_dir(tmp.data());
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(1280.0, ht)).with_pixels_per_point(ppp).wgpu().build_eframe(move |cc| App::new(cc, boot));
        h.state_mut().first_layout = false;
        only(&mut h, vec![Tab::Eds]);
        drop_files(&mut h, &[test_eds(), eds]);
        assert!(has(&h, "sim_long_info.eds"), "EDS not added\nUI:\n{}", dump(&h));
        let mut d = DockState::new(vec![Tab::Trace(1)]);
        d.main_surface_mut().split_right(NodeIndex::root(), frac, vec![Tab::Eds]);
        set_dock(&mut h, d);
        h.state_mut().selected_eds = Some("test.eds".to_string());
        step2(&mut h);
        step2(&mut h);
        let id = top_by_label(&h, "sim_long_info.eds", None);
        click_id(&mut h, id);
        step2(&mut h);
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render of {name} failed: {e}"));
        img.save(out.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}.png not written: {e}"));
    }
    // The user's layout: a maximised 2560 x 1369 window, Compact rows, zebra
    // on, an active device, the EDS tab narrow on the left and the Network
    // view right of it. CANOXIDE_TEST_EDS names a local EDS for a local run
    // (it is copied only into the temporary data folder); without it the
    // test uses the temp EDS.
    for (name, zebra, density) in [("eds_info_user_dark", true, crate::settings::RowDensity::Compact), ("eds_info_user_comf_dark", true, crate::settings::RowDensity::Comfortable), ("eds_info_user_nozebra_dark", false, crate::settings::RowDensity::Compact)] {
        let tmp = TempDir::new(name);
        preset(&tmp.data(), Settings { theme: Theme::Dark, zebra, row_density: density, ..Settings::default() });
        let eds = match std::env::var_os("CANOXIDE_TEST_EDS") {
            Some(p) => PathBuf::from(p),
            None => {
                let p = tmp.0.join("sim_long_info.eds");
                let text = crate::can::sim::BUILTIN_EDS
                    .replace("FileName=sim_builtin.eds", "FileName=sim_long_info.eds")
                    .replace("CreatedBy=CANoxide", "CreatedBy=CANoxide test fixture, written by the screenshot test with a long value that wraps to more than one line");
                std::fs::write(&p, text).unwrap();
                p
            }
        };
        let file = eds.file_name().unwrap().to_string_lossy().to_string();
        let boot = Boot::load_dir(tmp.data());
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(2560.0, 1369.0)).wgpu().build_eframe(move |cc| App::new(cc, boot));
        h.state_mut().first_layout = false;
        only(&mut h, vec![Tab::Eds]);
        drop_files(&mut h, &[eds]);
        {
            let app = h.state_mut();
            let dev = crate::library::Device { name: "HMI".into(), node_id: 1, eds_file: Some(file.clone()), bitrate_kbit: app.settings.bitrate_kbit, notes: String::new() };
            app.lib.as_mut().unwrap().add_device(dev).unwrap();
            app.settings.active_device = Some("HMI".into());
        }
        let mut d = DockState::new(vec![Tab::Eds]);
        d.main_surface_mut().split_right(NodeIndex::root(), 530.0 / 2560.0, vec![Tab::Network]);
        set_dock(&mut h, d);
        h.state_mut().selected_eds = Some(file.clone());
        step2(&mut h);
        step2(&mut h);
        for l in ["FileName", "FileVersion", "FileRevision"] {
            let r = rect_id(&h, top_by_label(&h, l, Some(Role::Label)));
            println!("{name} {l} {:.1} {:.1} {:.1} {:.1}", r.x0, r.y0, r.x1, r.y1);
        }
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render of {name} failed: {e}"));
        img.save(out.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}.png not written: {e}"));
    }
    // The user's report: a 2560 x 1440 window, the EDS library in front at
    // the top left beside Devices (layout B, the left leaf about 500 px
    // wide), Compact rows, zebra on, an EDS whose first values have
    // underscores. The first three rows carry the same value, so their
    // rendered text must be the same pixel for pixel.
    let mut bad = Vec::new();
    // 1369: the client height of a maximised window on a 2560 x 1440 monitor
    // with a 48 px taskbar (measured with the real exe, 2026-11-20).
    for (ppp, phys_h) in [(1.0_f32, 1440.0_f32), (1.0, 1369.0), (1.25, 1440.0), (1.5, 1440.0)] {
        let name = format!("eds_info_rows_{}_{}_dark", (ppp * 100.0) as u32, phys_h as u32);
        let tmp = TempDir::new(&name);
        preset(&tmp.data(), Settings { theme: Theme::Dark, zebra: true, row_density: crate::settings::RowDensity::Compact, ..Settings::default() });
        let eds = tmp.0.join("uitest_rows.eds");
        let value = "AB1140_HMI_Slave.eds";
        let text = crate::can::sim::BUILTIN_EDS
            .replace("FileName=sim_builtin.eds", &format!("FileName={value}"))
            .replace("FileVersion=1", &format!("FileVersion={value}"))
            .replace("FileRevision=0", &format!("FileRevision={value}"));
        std::fs::write(&eds, text).unwrap();
        let boot = Boot::load_dir(tmp.data());
        let (w, ht) = (2560.0 / ppp, phys_h / ppp);
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(w, ht)).with_pixels_per_point(ppp).wgpu().build_eframe(move |cc| App::new(cc, boot));
        h.state_mut().first_layout = false;
        only(&mut h, vec![Tab::Eds]);
        drop_files(&mut h, &[eds]);
        let mut d = dock::layout_b();
        if let Some(egui_dock::Node::Horizontal { fraction, .. }) = d.main_surface_mut().root_node_mut() {
            *fraction = 500.0 / 2560.0;
        }
        if let Some(t) = d.find_tab(&Tab::Eds) {
            d.set_active_tab(t);
        }
        set_dock(&mut h, d);
        h.state_mut().selected_eds = Some("uitest_rows.eds".to_string());
        step2(&mut h);
        step2(&mut h);
        let mut rows: Vec<Rect> = h.query_all_by(|n| n.role() == Role::Label && n.value().as_deref() == Some(value)).map(|n| rect_of(&n)).collect();
        rows.sort_by(|a, b| a.y0.total_cmp(&b.y0));
        for (i, r) in rows.iter().enumerate() {
            println!("{name} value row {i}: {:.3} {:.3} {:.3} {:.3} (px y {:.3}..{:.3})", r.x0, r.y0, r.x1, r.y1, r.y0 * ppp as f64, r.y1 * ppp as f64);
        }
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render of {name} failed: {e}"));
        img.save(out.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}.png not written: {e}"));
        assert_eq!(rows.len(), 3, "three rows with the same value\nUI:\n{}", dump(&h));
        let (iw, ih) = img.dimensions();
        let profiles: Vec<Vec<u32>> = rows.iter().map(|r| text_profile(img.as_raw(), iw, ih, r, ppp)).collect();
        for (i, p) in profiles.iter().enumerate() {
            println!("{name} row {i} bright pixels per pixel row: {p:?}");
        }
        // Rows 0 and 2 have the same background (row 1 is striped): the same
        // glyph rows, and the same pitch from row to row (at 125 % a pitch of
        // 30 points is 37.5 px, so there the pitch may differ by 1 px).
        let (d0, d1) = (profiles[1][0] as i64 - profiles[0][0] as i64, profiles[2][0] as i64 - profiles[1][0] as i64);
        let slack = if (30.0 * ppp).fract() == 0.0 { 0 } else { 1 };
        if profiles[0][1..] != profiles[2][1..] || (d0 - d1).abs() > slack {
            bad.push(name.clone());
        }
    }
    assert!(bad.is_empty(), "rows of the EDS info grid render differently: {bad:?}");
}

/// Sweep of window sizes (default layout, EDS library in front, Compact,
/// zebra): rows 0 and 2 of the EDS info grid carry the same value on the
/// same background, so their rendered text must be equal at every size.
/// `cargo test eds_info_sweep -- --ignored --nocapture`.
#[test]
#[ignore]
fn eds_info_sweep() {
    let value = "AB1140_HMI_Slave.eds";
    let mut bad = Vec::new();
    // (width, height, rebuild the default layout for this width). Without a
    // rebuild the saved split fractions stay, as when the user resizes.
    let mut sizes: Vec<(f32, f32, bool)> = (700..=1000).map(|ht| (1280.0, ht as f32, true)).collect();
    for w in [1281.0, 1333.0, 1599.0, 1700.0, 1919.0, 2001.0, 2333.0, 2559.0] {
        for ht in [700.0, 777.0, 801.0, 853.0, 999.0, 1080.0, 1369.0] {
            sizes.push((w, ht, true));
        }
    }
    sizes.push((1280.0, 800.0, true));
    sizes.push((2560.0, 1369.0, true));
    sizes.extend((1200..=2560).map(|w| (w as f32, 1369.0, false)));
    sizes.extend((600..=1369).map(|ht| (2000.0, ht as f32, false)));
    sizes.extend((600..=1369).step_by(23).flat_map(|ht| (1200..=2560).step_by(101).map(move |w| (w as f32, ht as f32, false))));
    let tmp = TempDir::new("eds_sweep");
    preset(&tmp.data(), Settings { theme: Theme::Dark, zebra: true, row_density: crate::settings::RowDensity::Compact, ..Settings::default() });
    let eds = tmp.0.join("uitest_rows.eds");
    let text = crate::can::sim::BUILTIN_EDS
        .replace("FileName=sim_builtin.eds", &format!("FileName={value}"))
        .replace("FileVersion=1", &format!("FileVersion={value}"))
        .replace("FileRevision=0", &format!("FileRevision={value}"));
    std::fs::write(&eds, text).unwrap();
    let boot = Boot::load_dir(tmp.data());
    let mut h: H = Harness::builder().with_size(egui::Vec2::new(1280.0, 800.0)).wgpu().build_eframe(move |cc| App::new(cc, boot));
    h.state_mut().first_layout = false;
    only(&mut h, vec![Tab::Eds]);
    drop_files(&mut h, &[eds]);
    let mut last_w = 0.0;
    let mut checked = 0;
    for (w, ht, rebuild) in sizes {
        h.set_size(egui::Vec2::new(w, ht));
        if rebuild && w != last_w {
            let mut d = dock::default_layout(w);
            if w >= 1600.0 {
                // The user's left leaf, about 500 px wide.
                if let Some(egui_dock::Node::Horizontal { fraction, .. }) = d.main_surface_mut().root_node_mut() {
                    *fraction = 500.0 / 2560.0;
                }
            }
            if let Some(t) = d.find_tab(&Tab::Eds) {
                d.set_active_tab(t);
            }
            set_dock(&mut h, d);
            h.state_mut().selected_eds = Some("uitest_rows.eds".to_string());
        }
        last_w = w;
        checked += 1;
        step2(&mut h);
        step2(&mut h);
        let mut rows: Vec<Rect> = h.query_all_by(|n| n.role() == Role::Label && n.value().as_deref() == Some(value)).map(|n| rect_of(&n)).collect();
        rows.sort_by(|a, b| a.y0.total_cmp(&b.y0));
        assert_eq!(rows.len(), 3, "{w}x{ht}: three rows with the same value\nUI:\n{}", dump(&h));
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render failed: {e}"));
        let (iw, ih) = img.dimensions();
        let p: Vec<Vec<u32>> = rows.iter().map(|r| text_profile(img.as_raw(), iw, ih, r, 1.0)).collect();
        let ys: Vec<String> = rows.iter().map(|r| format!("{:.3}..{:.3}", r.y0, r.y1)).collect();
        if p[0][1..] != p[2][1..] || p[1][0] - p[0][0] != p[2][0] - p[1][0] {
            println!("MISMATCH {w}x{ht} rows {ys:?}\n  row0 {:?}\n  row1 {:?}\n  row2 {:?}", p[0], p[1], p[2]);
            if bad.len() < 5 {
                let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screens");
                std::fs::create_dir_all(&out).unwrap();
                img.save(out.join(format!("eds_sweep_{w}x{ht}.png"))).unwrap();
            }
            bad.push(format!("{w}x{ht}"));
        } else if rows.iter().any(|r| r.y0.fract().abs() > 1e-6) {
            println!("frac ok {w}x{ht} rows {ys:?}");
        }
    }
    println!("checked {checked} sizes");
    assert!(bad.is_empty(), "rows 0 and 2 render differently at {} size(s): {bad:?}", bad.len());
}

/// The first pixel row with text, then the bright pixels per pixel row of a
/// text rect (RGBA image, 2 px margin above and below) without the empty
/// rows at both ends: a pixel level compare of two rows with the same text.
fn text_profile(raw: &[u8], w: u32, ht: u32, r: &Rect, ppp: f32) -> Vec<u32> {
    let ppp = ppp as f64;
    let (x0, x1) = ((r.x0 * ppp).floor() as u32, ((r.x1 * ppp).ceil() as u32).min(w));
    let (y0, y1) = (((r.y0 * ppp).floor() as u32).saturating_sub(2), ((r.y1 * ppp).ceil() as u32 + 2).min(ht));
    let mut v: Vec<u32> = (y0..y1)
        .map(|y| {
            (x0..x1)
                .filter(|&x| {
                    let i = ((y * w + x) * 4) as usize;
                    (raw[i] as u32 + raw[i + 1] as u32 + raw[i + 2] as u32) > 330
                })
                .count() as u32
        })
        .collect();
    while v.last() == Some(&0) {
        v.pop();
    }
    let lead = v.iter().take_while(|&&c| c == 0).count();
    v.drain(..lead);
    v.insert(0, y0 + lead as u32);
    v
}

fn shots_v03(out: &Path) {
    let shot = |h: &mut H, name: &str| {
        step2(h);
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render of {name} failed: {e}"));
        img.save(out.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}.png not written: {e}"));
    };
    let dark = |name: &str, w: f32, ht: f32| -> (TempDir, H) {
        let tmp = TempDir::new(name);
        preset(&tmp.data(), Settings { theme: Theme::Dark, ..Settings::default() });
        let boot = Boot::load_dir(tmp.data());
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(w, ht)).wgpu().build_eframe(move |cc| App::new(cc, boot));
        h.state_mut().first_layout = false;
        (tmp, h)
    };
    // Replay: a log loaded and scrubbed to the middle, paused.
    {
        let (t, mut h) = dark("shot_replay", 1600.0, 900.0);
        let log = t.0.join("bench_run.log");
        write_replay_log(&log, 3000);
        let mut d = DockState::new(vec![Tab::Trace(1)]);
        d.main_surface_mut().split_right(NodeIndex::root(), 0.66, vec![Tab::Messages]);
        set_dock(&mut h, d);
        {
            let app = h.state_mut();
            app.settings.adapter = AdapterKind::Replay;
            app.settings.replay_file = log.display().to_string();
        }
        step2(&mut h);
        click(&mut h, "Connect");
        wait_for(&mut h, "bad-line question", |h| has(h, "Log file with bad lines"));
        click(&mut h, "Load the good frames");
        wait_for(&mut h, "replay connected", |h| replay_info(h).is_some());
        h.state_mut().replay_seek(17_420_000);
        wait_for(&mut h, "seek done", |h| replay_info(h).is_some_and(|i| i.seeking.is_none() && i.cursor > 0));
        run_for(&mut h, Duration::from_millis(200));
        shot(&mut h, "replay_dark");
    }
    // Messages detail with two user signals and the Define signal dialog.
    let user_sigs = |h: &mut H| {
        let mut a = crate::usersig::UserSignal::new("level", 0x3A0, false, 8, 16);
        a.unit = "mm".into();
        a.factor = 0.1;
        let mut b = crate::usersig::UserSignal::new("pump on", 0x3A0, false, 24, 1);
        b.colour = crate::usersig::COLOURS[3];
        h.state_mut().set_user_signals(vec![a, b]);
    };
    {
        let (_t, mut h) = dark("shot_define", 1500.0, 950.0);
        set_dock(&mut h, DockState::new(vec![Tab::Messages]));
        user_sigs(&mut h);
        analysis_frames(&mut h, 100);
        step2(&mut h);
        let r = top_by_label(&h, "3A0", None);
        pointer_click(&mut h, r);
        analysis_frames(&mut h, 500);
        step2(&mut h);
        click(&mut h, "Byte 0");
        click(&mut h, "Define signal...");
        h.state_mut().an.draft.as_mut().unwrap().sig.name = "flags".into();
        run_for(&mut h, Duration::from_millis(100));
        shot(&mut h, "signal_define_dark");
    }
    // History with a filter on a user signal.
    {
        let (_t, mut h) = dark("shot_history", 1500.0, 950.0);
        set_dock(&mut h, DockState::new(vec![Tab::Messages]));
        user_sigs(&mut h);
        analysis_frames(&mut h, 100);
        step2(&mut h);
        let r = top_by_label(&h, "3A0", None);
        pointer_click(&mut h, r);
        analysis_frames(&mut h, 500);
        step2(&mut h);
        {
            let hv = &mut h.state_mut().an.hist;
            hv.filter_on = true;
            hv.col = 1;
            hv.op = Some(crate::analysis::CmpOp::Eq);
            hv.value_text = "1".into();
        }
        run_for(&mut h, Duration::from_millis(400));
        shot(&mut h, "history_dark");
    }
    // Find signal after two steps, beside the Messages view.
    {
        let (_t, mut h) = dark("shot_find", 1500.0, 900.0);
        let mut d = DockState::new(vec![Tab::Messages]);
        d.main_surface_mut().split_right(NodeIndex::root(), 0.45, vec![Tab::FindSignal]);
        set_dock(&mut h, d);
        analysis_frames(&mut h, 50);
        msg_frame(&mut h, 0x2B0, &[9, 9, 0, 0, 4, 0, 0, 0], 2_000_000);
        h.state_mut().an.find.fs.settings.sizes = vec![8, 16];
        step2(&mut h);
        click(&mut h, "Start");
        analysis_frames(&mut h, 60);
        pick(&mut h, "Step", "increased");
        close_popup(&mut h);
        click(&mut h, "Apply step");
        run_for(&mut h, Duration::from_millis(100));
        shot(&mut h, "find_signal_dark");
    }
}

// ------------------------------------------------------- 0.3 analysis

/// Push a frame into the Messages statistics and the trace buffer.
fn hist_frame(h: &mut H, id: u32, data: &[u8], t_us: u64) {
    msg_frame(h, id, data, t_us);
    let mut f = crate::can::CanFrame::new(id, data);
    f.timestamp_us = t_us;
    let app = h.state_mut();
    let d = crate::canopen::decode::decode(&f, &app.decode_ctx);
    app.an.sparks.on_frame(&f);
    app.trace.push(f, d);
}

/// Made-up frames of COB-ID 3A0: byte 0 a counter, bytes 1-2 a LE value
/// that rises, byte 3 bit 0 follows counter bit 2. A second call goes on
/// from the frames already in the trace.
fn analysis_frames(h: &mut H, n: u64) {
    let i0 = h.state().trace.frames().filter(|f| f.id == 0x3A0).count() as u64;
    for i in i0..i0 + n {
        let v = 1000 + i * 7;
        let b3 = ((i >> 2) & 1) as u8;
        hist_frame(h, 0x3A0, &[i as u8, v as u8, (v >> 8) as u8, b3], 1_000_000 + i * 20_000);
    }
}

/// Centre of the bit cell `bit` (7 = left) in the row of `byte`.
fn bit_cell(h: &H, byte: usize, bit: u32) -> egui::Pos2 {
    let r = rect_id(h, top_by_label(h, &format!("Byte {byte}"), Some(Role::Button)));
    let x = r.x1 as f32 + h.ctx.style().spacing.item_spacing.x + 12.0 + (7 - bit) as f32 * (24.0 + h.ctx.style().spacing.item_spacing.x);
    egui::pos2(x, ((r.y0 + r.y1) / 2.0) as f32)
}

/// Define a signal: a drag over bits across a byte border selects them;
/// Byte 1 + Define signal opens the dialog filled in; Save stores it in
/// user_signals.json; it shows in the detail list, the trace decoded text,
/// the history columns and the Graph "User signal" source.
#[test]
fn s31_define_signal() {
    let tmp = TempDir::new("s31");
    let mut h = build_sized(&tmp.data(), 1500.0, 1000.0);
    only(&mut h, vec![Tab::Messages]);
    analysis_frames(&mut h, 40);
    step2(&mut h);
    let r = top_by_label(&h, "3A0", None);
    pointer_click(&mut h, r);
    assert!(has_exact(&h, "Define signal..."), "detail\nUI:\n{}", dump(&h));
    // Drag from byte 0 bit 4 to byte 1 bit 2 (LE: bits 4..=10).
    // (The header text changes with the selection, so the cells are
    // measured again on each move.)
    let a = bit_cell(&h, 0, 4);
    h.input_mut().events.push(egui::Event::PointerMoved(a));
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton { pos: a, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    let mut z = a;
    for i in 1..=6 {
        let (a, end) = (bit_cell(&h, 0, 4), bit_cell(&h, 1, 2));
        z = a + (end - a) * (i as f32 / 6.0);
        h.input_mut().events.push(egui::Event::PointerMoved(z));
        h.step();
    }
    h.input_mut().events.push(egui::Event::PointerButton { pos: z, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    assert!(has_exact(&h, "Selected: bit 4, 7 bit"), "drag selection\nUI:\n{}", dump(&h));
    // Byte 1 + Define signal: the dialog is filled in from the selection.
    click(&mut h, "Byte 1");
    assert!(has_exact(&h, "Selected: bit 8, 8 bit"));
    click(&mut h, "Define signal...");
    assert!(has_exact(&h, "Define signal"), "dialog\nUI:\n{}", dump(&h));
    {
        let d = h.state().an.draft.as_ref().expect("draft");
        assert_eq!((d.sig.cob_id, d.sig.start_bit, d.sig.bits, d.sig.big_endian), (0x3A0, 8, 8, false));
    }
    // A default name is offered; Save without a name is refused.
    assert_eq!(h.state().an.draft.as_ref().unwrap().sig.name, "3A0 sig1");
    h.state_mut().an.draft.as_mut().unwrap().sig.name.clear();
    click_role(&mut h, Role::Button, "Save");
    assert!(h.state().an.draft.as_ref().is_some_and(|d| d.error.is_some()), "empty name accepted");
    h.state_mut().an.draft.as_mut().unwrap().sig.name = "level".into();
    h.state_mut().an.draft.as_mut().unwrap().sig.unit = "mm".into();
    click_role(&mut h, Role::Button, "Save");
    assert!(h.state().an.draft.is_none(), "dialog still open\nUI:\n{}", dump(&h));
    assert_eq!(h.state().an.usigs.len(), 1);
    let file = std::fs::read_to_string(tmp.data().join(crate::usersig::USER_SIGNALS_FILE)).expect("user_signals.json");
    assert!(file.contains("\"level\"") && file.contains("\"version\""), "{file}");
    // In the detail list (newest byte 1 = (1000 + 39 * 7) & 0xFF = 249).
    assert!(has_exact(&h, "level = 249 mm"), "value list\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "level"), "history column\nUI:\n{}", dump(&h));
    // The trace decoded text.
    let mut f = crate::can::CanFrame::new(0x3A0, &[0, 42, 0, 0]);
    f.timestamp_us = 3_000_000;
    let d = crate::canopen::decode::decode(&f, &h.state().decode_ctx);
    assert!(d.text.contains("level=42 mm"), "decoded {:?}", d.text);
    // The Graph source.
    only(&mut h, vec![Tab::Graph]);
    click_containing(&mut h, "Add signal...");
    click(&mut h, "User signal");
    // The combo of the dialog (it has no label) lists the user signal.
    let win = rect_id(&h, top_by_label(&h, "Add signal", Some(Role::Window)));
    let combo = h.query_all_by_role(Role::ComboBox).map(|n| (n.id(), rect_of(&n))).find(|(_, r)| r.y0 > win.y0 && r.y1 < win.y1 && r.x0 > win.x0).map(|(id, _)| id).expect("user signal combo");
    click_id(&mut h, combo);
    assert!(has_exact(&h, "level (3A0, bit 8, 8 bit)"), "graph source\nUI:\n{}", dump(&h));
    click(&mut h, "level (3A0, bit 8, 8 bit)");
    click_role(&mut h, Role::Button, "Add");
    assert!(has_exact(&h, "level [mm]") && has_exact(&h, "raw 3A0, bit 8, 8 bit unsigned"), "legend row\nUI:\n{}", dump(&h));
}

/// An access-kit click on a combo option does not close the popup (a real
/// click does): Escape closes it.
fn close_popup(h: &mut H) {
    if h.ctx.memory(|m| m.any_popup_open()) {
        h.input_mut().events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        step2(h);
    }
}

/// Find signal: Start takes every field; "increased" keeps the rising
/// ones; "unchanged" after no new frame keeps all; Undo step goes back.
#[test]
fn s32_find_signal_steps() {
    let tmp = TempDir::new("s32");
    let mut h = build_sized(&tmp.data(), 1500.0, 1000.0);
    only(&mut h, vec![Tab::Trace(1)]);
    view_menu(&mut h, "Find signal");
    h.input_mut().events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    assert!(disabled(&h, top_by_label(&h, "Start", Some(Role::Button))), "Start without frames");
    // Only 8-bit fields: one COB-ID with 4 bytes = 4 candidates.
    msg_frame(&mut h, 0x3A0, &[1, 10, 0x55, 7], 1_000_000);
    msg_frame(&mut h, 0x2B0, &[9, 9], 1_000_000);
    h.state_mut().an.find.fs.settings.sizes = vec![8];
    step2(&mut h);
    click(&mut h, "Start");
    assert!(has_exact(&h, "6 candidates"), "start\nUI:\n{}", dump(&h));
    // Byte 1 of 3A0 rises, byte 0 of 2B0 falls.
    msg_frame(&mut h, 0x3A0, &[1, 11, 0x55, 7], 1_020_000);
    msg_frame(&mut h, 0x2B0, &[8, 9], 1_020_000);
    pick(&mut h, "Step", "increased");
    close_popup(&mut h);
    click(&mut h, "Apply step");
    assert!(has_exact(&h, "1 candidates"), "increased\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "Define signal..."), "small set listed\nUI:\n{}", dump(&h));
    click(&mut h, "Undo step");
    assert!(has_exact(&h, "6 candidates"), "undo\nUI:\n{}", dump(&h));
    pick(&mut h, "Step", "changed");
    close_popup(&mut h);
    click(&mut h, "Apply step");
    assert!(has_exact(&h, "2 candidates"), "changed\nUI:\n{}", dump(&h));
    // Define signal from a candidate opens the dialog filled in.
    click(&mut h, "Define signal...");
    assert!(h.state().an.draft.as_ref().is_some_and(|d| d.sig.bits == 8), "dialog from a candidate");
    click_role(&mut h, Role::Button, "Cancel");
    click(&mut h, "Reset");
    assert!(has_exact(&h, "6 candidates"), "reset\nUI:\n{}", dump(&h));
}

/// The embedded window icon decodes; the About dialog shows it.
#[test]
fn s33_app_icon() {
    let icon = super::app_icon().expect("window icon decodes");
    assert_eq!((icon.width, icon.height), (64, 64));
    assert_eq!(icon.rgba.len(), 64 * 64 * 4);
    // Transparent outside the tile, opaque in the middle.
    assert_eq!(icon.rgba[3], 0, "corner pixel");
    assert_eq!(icon.rgba[(32 * 64 + 32) * 4 + 3], 255, "centre pixel");
    let tmp = TempDir::new("s33");
    let mut h = build(&tmp.data());
    click_role(&mut h, Role::Button, "Help");
    click(&mut h, "About CANoxide");
    assert!(has_exact(&h, "CANoxide icon"), "About icon\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, &format!("Version {}", crate::VERSION)));
    assert_eq!(crate::VERSION, "0.5.2");
}

// ------------------------------------------------------------ 0.3 replay

/// A made-up candump log: `n` frames 10 ms apart (a heartbeat of node 5
/// every 10th frame, else TPDO1 of node 5 with a counter), and one bad line.
fn write_replay_log(path: &Path, n: u64) {
    let mut s = String::new();
    for i in 0..n {
        let us = 100_000_000 + i * 10_000;
        let (sec, usec) = (us / 1_000_000, us % 1_000_000);
        if i % 10 == 0 {
            s.push_str(&format!("({sec}.{usec:06}) can0 705#05\n"));
        } else {
            s.push_str(&format!("({sec}.{usec:06}) can0 185#{:02X}{:02X}\n", i & 0xFF, i >> 8));
        }
        if i == 3 {
            s.push_str("this is not a frame\n");
        }
    }
    std::fs::write(path, s).unwrap();
}

fn replay_info(h: &H) -> Option<crate::bus::ReplayInfo> {
    h.state().bus.shared.lock().ok().and_then(|s| s.replay.clone())
}

/// Press and release the primary button at a point.
fn press_at(h: &mut H, pos: egui::Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    step2(h);
}

/// Replay: choose a log file, accept its bad line, connect, scrub with the
/// slider, play as fast as possible to the end; nothing can be sent.
#[test]
fn s30_replay_connect_and_scrub() {
    let t = TempDir::new("replay");
    let log = t.0.join("bench.log");
    write_replay_log(&log, 1000);
    let mut h = build(&t.data());
    only(&mut h, vec![Tab::Trace(1)]);
    {
        let app = h.state_mut();
        app.settings.adapter = AdapterKind::Replay;
        app.settings.replay_file = log.display().to_string();
    }
    step2(&mut h);
    assert!(has_exact(&h, "bench.log"), "file name in the connection bar\nUI:\n{}", dump(&h));
    // No transport bar before the log is loaded.
    assert!(!has_exact(&h, "Replay speed"));
    click(&mut h, "Connect");
    wait_for(&mut h, "bad-line question", |h| has(h, "Log file with bad lines"));
    assert!(has(&h, "line 5: "), "the bad line is named\nUI:\n{}", dump(&h));
    click(&mut h, "Load the good frames");
    wait_for(&mut h, "replay connected", |h| has(h, "Replay: nothing is sent") && has_exact(h, "frame 0 / 1000"));
    assert!(has(&h, "Connected: log file bench.log"), "status text\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "0.000 s / 9.990 s"));
    // Scrub: a click in the middle of the slider.
    let id = h.query_all_by_role(Role::Slider).next().expect("scrubber").id();
    let r = rect_id(&h, id);
    press_at(&mut h, egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32));
    wait_for(&mut h, "seek done", |h| replay_info(h).is_some_and(|i| i.seeking.is_none() && i.cursor > 0));
    let info = replay_info(&h).unwrap();
    assert!((3_500_000..6_500_000).contains(&info.pos_us), "pos {}", info.pos_us);
    // Frames at 0, 10 ms, ...: the cursor is one past the frame at pos.
    assert_eq!(info.cursor as u64, info.pos_us / 10_000 + 1);
    assert!(!info.playing);
    step2(&mut h);
    assert_eq!(trace_counts(&h).0, info.cursor, "the trace holds the frames up to the cursor");
    assert!(has_exact(&h, &format!("frame {} / 1000", info.cursor)));
    // Play as fast as possible to the end.
    pick(&mut h, "Replay speed", "As fast as possible");
    click(&mut h, "Play");
    wait_for(&mut h, "end of the log", |h| has_exact(h, "frame 1000 / 1000") && replay_info(h).is_some_and(|i| !i.playing));
    step2(&mut h);
    assert_eq!(trace_counts(&h).0, 1000);
    // Nothing is sent.
    h.state().bus.send(BusCommand::Sync);
    run_for(&mut h, Duration::from_millis(100));
    assert!(has(&h, "a log is played back, nothing is sent"), "send refused\nUI:\n{}", dump(&h));
    assert_eq!(trace_counts(&h).0, 1000, "no frame was added");
    click(&mut h, "Disconnect");
    wait_for(&mut h, "disconnected", |h| has_exact(h, "Connect") && !has(h, "Replay: nothing is sent"));
    assert!(!has_exact(&h, "Replay speed"));
}

/// Replay of a PEAK .trc file (version 1.1): it loads with its format in
/// the status; the records that are no frame are counted, not bad lines.
#[test]
fn s35_replay_peak_trc() {
    let t = TempDir::new("replay_trc");
    let log = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join("peak_v11.trc");
    let mut h = build(&t.data());
    only(&mut h, vec![Tab::Trace(1)]);
    {
        let app = h.state_mut();
        app.settings.adapter = AdapterKind::Replay;
        app.settings.replay_file = log.display().to_string();
    }
    step2(&mut h);
    assert!(has_exact(&h, "peak_v11.trc"), "file name in the connection bar\nUI:\n{}", dump(&h));
    click(&mut h, "Connect");
    wait_for(&mut h, "replay connected", |h| has(h, "Replay: nothing is sent") && has_exact(h, "frame 0 / 4"));
    assert!(!has(&h, "Log file with bad lines"));
    assert!(has(&h, "Connected: log file peak_v11.trc (PEAK trace 1.1, 4 frames"), "status text\nUI:\n{}", dump(&h));
    assert!(has(&h, "2 other record(s) skipped"), "skipped count\nUI:\n{}", dump(&h));
    pick(&mut h, "Replay speed", "As fast as possible");
    click(&mut h, "Play");
    wait_for(&mut h, "end of the log", |h| has_exact(h, "frame 4 / 4") && replay_info(h).is_some_and(|i| !i.playing));
    step2(&mut h);
    assert_eq!(trace_counts(&h).0, 4);
    click(&mut h, "Disconnect");
    wait_for(&mut h, "disconnected", |h| has_exact(h, "Connect"));
}

/// Import DBC: the dialog lists the import; Add stores the signals, which
/// then decode in the Messages view and in the decoded text.
#[test]
fn s36_import_dbc() {
    let tmp = TempDir::new("s36");
    let dbc = tmp.0.join("level.dbc");
    std::fs::write(
        &dbc,
        "VERSION \"\"\n\nBU_: Node\n\nBO_ 928 Level: 4 Node\n SG_ depth : 8|16@1+ (1,0) [0|65535] \"mm\" Node\n SG_ count : 7|8@0+ (1,0) [0|255] \"\" Node\n\nBO_ 2147484672 Ext: 8 Node\n SG_ Mux M : 0|8@1+ (1,0) [0|0] \"\" Node\n",
    )
    .unwrap();
    let mut h = build_sized(&tmp.data(), 1500.0, 1000.0);
    only(&mut h, vec![Tab::Messages]);
    analysis_frames(&mut h, 40);
    step2(&mut h);
    assert!(has_exact(&h, "Import DBC..."), "toolbar button\nUI:\n{}", dump(&h));
    let before = buttons(&h, "Add");
    h.state_mut().open_dbc(&dbc);
    step2(&mut h);
    assert!(has_exact(&h, "Import DBC"), "dialog\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "Replace all signals of these IDs"));
    click_new_button(&mut h, "Add", &before);
    assert!(h.state().an.dbc.is_none(), "dialog still open\nUI:\n{}", dump(&h));
    assert_eq!(h.state().an.usigs.len(), 2);
    assert!(has(&h, "DBC level.dbc: "), "status\nUI:\n{}", dump(&h));
    // The decoded text of a new frame (depth = 0x2710, count = byte 0).
    let mut f = crate::can::CanFrame::new(0x3A0, &[5, 0x10, 0x27, 0]);
    f.timestamp_us = 3_000_000;
    let d = crate::canopen::decode::decode(&f, &h.state().decode_ctx);
    assert!(d.text.contains("depth=10000 mm") && d.text.contains("count=5"), "decoded {:?}", d.text);
    // The Messages detail (newest depth = 1000 + 39 * 7 = 1273).
    let r = top_by_label(&h, "3A0", None);
    pointer_click(&mut h, r);
    assert!(has_exact(&h, "depth = 1273 mm"), "value list\nUI:\n{}", dump(&h));
}

/// The PDO view checks the mapping in the editor: 4 x 16 + 8 = 72 bits is
/// an error that shows under the editor.
#[test]
fn s37_pdo_check_finding() {
    let tmp = TempDir::new("s37");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Pdo");
    connect(&mut h);
    only(&mut h, vec![Tab::Pdo]);
    click_containing(&mut h, "Read PDOs");
    wait_for(&mut h, "PDOs read", |h| has(h, "PDO(s) read"));
    let t1 = top_by_label(&h, "TPDO1", Some(Role::Label));
    let edit = button_on_row(&h, t1, "Edit");
    click_id(&mut h, edit);
    step2(&mut h);
    assert!(has_exact(&h, "PDO check: no finding"), "clean PDO\nUI:\n{}", dump(&h));
    let mut m = vec![crate::canopen::pdo::MapEntry { index: 0x6401, sub: 1, bits: 16 }; 4];
    m.push(crate::canopen::pdo::MapEntry { index: 0x6000, sub: 1, bits: 8 });
    h.state_mut().pdo.editor.as_mut().expect("editor").new.mapping = m;
    step2(&mut h);
    assert!(has_exact(&h, "PDO check: 1 error(s), 0 warning(s)"), "finding count\nUI:\n{}", dump(&h));
    assert!(has(&h, "72 bits"), "finding text\nUI:\n{}", dump(&h));
}

/// The visible part of the Object dictionary leaf, in accesskit units:
/// the leaf rect without the room of the vertical scroll bar.
fn od_visible(h: &H) -> Rect {
    let d = h.state().dock.as_ref().expect("dock");
    let (s, n, _) = d.find_tab(&Tab::Od).expect("Od tab open");
    let r = d[s][n].rect().expect("leaf rect");
    let ppp = h.ctx.pixels_per_point() as f64;
    let bar = h.ctx.style().spacing.scroll.allocated_width() as f64;
    Rect { x0: r.min.x as f64 * ppp, y0: r.min.y as f64 * ppp, x1: (r.max.x as f64 - bar) * ppp, y1: r.max.y as f64 * ppp }
}

/// Spec 8.2: the actions column of the object dictionary shows both
/// buttons completely, in the default layout at every UI scale and in both
/// row densities: every Write button (the right one) lies inside the
/// visible table, left of the vertical scroll bar.
#[test]
fn s35_od_actions_column_fits() {
    for (scale, density) in [(1.0, crate::settings::RowDensity::Compact), (1.0, crate::settings::RowDensity::Comfortable), (1.25, crate::settings::RowDensity::Compact), (1.5, crate::settings::RowDensity::Comfortable), (2.0, crate::settings::RowDensity::Compact)] {
        let tmp = TempDir::new("s35");
        preset(&tmp.data(), Settings { ui_scale: scale, row_density: density, ..Settings::default() });
        let s = Settings::default();
        let mut h = build_sized(&tmp.data(), s.window_width, s.window_height);
        add_eds_and_device(&mut h, "Bench");
        only(&mut h, vec![Tab::Od]);
        connect(&mut h);
        let width = h.ctx.screen_rect().width();
        h.state_mut().reset_layout(width);
        step2(&mut h);
        step2(&mut h);
        let vis = od_visible(&h);
        let writes = buttons(&h, "Write");
        assert!(!writes.is_empty(), "no Write button at scale {scale}\nUI:\n{}", dump(&h));
        let mut seen_rw = false;
        for id in writes {
            let r = rect_id(&h, id);
            seen_rw |= !disabled(&h, id);
            assert!(r.x0 >= vis.x0 && r.x1 <= vis.x1 + 0.01, "Write button {r:?} not inside the visible table {vis:?} at scale {scale}, {density:?}");
        }
        assert!(seen_rw, "no enabled Write button (rw row) at scale {scale}\nUI:\n{}", dump(&h));
    }
}

/// Help > Copy display diagnostics: the text goes to the clipboard and the
/// status bar says so. The harness has no Win32 window.
#[test]
fn s36_copy_display_diagnostics() {
    let tmp = TempDir::new("s36");
    let mut h = build(&tmp.data());
    click_role(&mut h, Role::Button, "Help");
    click(&mut h, "Copy display diagnostics");
    step2(&mut h);
    assert!(has(&h, "Display diagnostics copied"), "status\nUI:\n{}", dump(&h));
    let t = h.state().diag_text.clone().expect("diagnostics text");
    println!("{t}");
    for k in ["app_version: ", "pixels_per_point: ", "native_pixels_per_point: ", "screen_rect_physical: ", "inner_rect_points: ", "settings_ui_scale: ", "win32: not available", "mismatch: no"] {
        assert!(t.contains(k), "{k} missing\n{t}");
    }
    let home = std::env::var("USERNAME").unwrap_or_default();
    assert!(!t.contains(":\\") && (home.len() < 3 || !t.contains(&home)), "no path, no user name\n{t}");
}

/// The object dictionary at the default window size, at 100 % and 150 %,
/// for a look at the actions column: `cargo test screenshots_od_actions -- --ignored`.
#[test]
#[ignore]
fn screenshots_od_actions() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screens");
    std::fs::create_dir_all(&out).unwrap();
    for (name, scale) in [("od_actions_100_dark", 1.0), ("od_actions_150_dark", 1.5)] {
        let tmp = TempDir::new(name);
        preset(&tmp.data(), Settings { ui_scale: scale, theme: Theme::Dark, ..Settings::default() });
        let s = Settings::default();
        let boot = Boot::load_dir(tmp.data());
        let mut h: H = Harness::builder().with_size(egui::Vec2::new(s.window_width, s.window_height)).wgpu().build_eframe(move |cc| App::new(cc, boot));
        add_eds_and_device(&mut h, "Bench");
        only(&mut h, vec![Tab::Od]);
        connect(&mut h);
        let width = h.ctx.screen_rect().width();
        h.state_mut().reset_layout(width);
        step2(&mut h);
        step2(&mut h);
        let img = h.render().unwrap_or_else(|e| panic!("wgpu render of {name} failed: {e}"));
        img.save(out.join(format!("{name}.png"))).unwrap_or_else(|e| panic!("{name}.png not written: {e}"));
    }
}

// ------------------------------------- v0.5: EMCY, EDS check, parameters

/// The View menu offers Emergencies (after Network) and EDS check (after
/// DCF save / restore).
#[test]
fn s37_view_menu_offers_emergencies_and_eds_check() {
    let tmp = TempDir::new("s37");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Trace(1)]);
    click_role(&mut h, Role::Button, "View");
    let y = |h: &H, name: &str| cy(&rect_id(h, top_by_label(h, name, Some(Role::CheckBox))));
    let (net, em, dcf, ec) = (y(&h, "Network"), y(&h, "Emergencies"), y(&h, "DCF save / restore"), y(&h, "EDS check"));
    assert!(net < em && dcf < ec, "menu order: Network {net}, Emergencies {em}, DCF {dcf}, EDS check {ec}\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::CheckBox, "Emergencies");
    assert!(h.state().is_open(Tab::Emergencies));
    view_menu(&mut h, "EDS check");
    assert!(h.state().is_open(Tab::EdsCheck));
}

/// Emergencies: a raised simulator EMCY is listed with its text; the error
/// history 0x1003 of the node is read and cleared.
#[test]
fn s38_emergencies_raise_read_clear() {
    let tmp = TempDir::new("s38");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Emergencies]);
    assert!(has(&h, "No emergency received yet."), "\nUI:\n{}", dump(&h));
    all_disabled(&h, "Read error history (0x1003)");
    connect(&mut h);
    click(&mut h, "Raise EMCY");
    wait_for(&mut h, "EMCY listed", |h| has_exact(h, "Life guard or heartbeat error") && has_exact(h, "1 EMCY"));
    assert!(has_exact(&h, "8130"), "\nUI:\n{}", dump(&h));
    // The simulator starts with 2 entries; the raised EMCY is the third.
    click(&mut h, "Read error history (0x1003)");
    wait_for(&mut h, "0x1003 read", |h| has_exact(h, "Node 1: 3 entries (1 = newest)"));
    // Clear asks first (confirm before write is on by default).
    let before = buttons(&h, "Clear");
    click(&mut h, "Clear error history");
    assert!(has_exact(&h, "Clear error history") && has(&h, "Write 0 to 0x1003:00 of node 1"), "no question\nUI:\n{}", dump(&h));
    click_new_button(&mut h, "Clear", &before);
    wait_for(&mut h, "0x1003 cleared", |h| has_exact(h, "Error history of node 1 cleared"));
    click(&mut h, "Read error history (0x1003)");
    wait_for(&mut h, "0x1003 empty", |h| has_exact(h, "Node 1: the error history is empty."));
    // Reset EMCY: code 0 in the ok colour; the list clear sends nothing.
    click(&mut h, "Reset EMCY");
    wait_for(&mut h, "reset listed", |h| has_exact(h, "Error reset / no error") && has_exact(h, "2 EMCY"));
    click(&mut h, &format!("{} Clear", icon::TRASH));
    assert!(has(&h, "No emergency received yet."), "\nUI:\n{}", dump(&h));
}

/// EDS check: disabled before the connection; on the simulator (built-in
/// EDS) the summary shows no failure.
#[test]
fn s39_eds_check_on_the_simulator() {
    let tmp = TempDir::new("s39");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::EdsCheck]);
    let start = format!("{} Start", icon::PLAY);
    all_disabled(&h, &start);
    connect(&mut h);
    click(&mut h, &start);
    wait_for(&mut h, "summary", |h| has(h, " 0 failed") && has(h, "sim_builtin.eds, node 1"));
    assert!(has_exact(&h, "Every entry is ok.") || has_exact(&h, "Index:sub"), "\nUI:\n{}", dump(&h));
    click(&mut h, "show ok entries");
    assert!(has_exact(&h, "1018:01"), "ok entries not shown\nUI:\n{}", dump(&h));
}

/// Network view, Parameters group: store and restore defaults on the
/// simulator, each after its question.
#[test]
fn s40_parameters_store_and_restore() {
    let tmp = TempDir::new("s40");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Network]);
    all_disabled_containing(&h, "Store (0x1010)");
    connect(&mut h);
    let before = buttons(&h, "Store");
    click_containing(&mut h, "Store (0x1010)");
    assert!(has_exact(&h, "Store parameters"), "no question\nUI:\n{}", dump(&h));
    click_new_button(&mut h, "Store", &before);
    wait_for(&mut h, "stored", |h| has_exact(h, "Store parameters (0x1010:01, All parameters) on node 1 done"));
    let before = buttons(&h, "Restore defaults");
    click_containing(&mut h, "Restore defaults (0x1011)");
    assert!(has_exact(&h, "Restore default parameters"), "no question\nUI:\n{}", dump(&h));
    click_new_button(&mut h, "Restore defaults", &before);
    wait_for(&mut h, "restored", |h| has(h, "The defaults are active after a reset node or a power cycle.") && has(h, "Restore defaults (0x1011:01, All parameters) on node 1 done"));
    click(&mut h, "Read supported");
    wait_for(&mut h, "caps read", |h| has(h, "0x1010: 1 on command, 2 on command, 3 on command"));
}

/// Listen only: the new send controls are disabled.
#[test]
fn s41_new_controls_listen_only() {
    let tmp = TempDir::new("s41");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Network]);
    click_role(&mut h, Role::CheckBox, "Listen only");
    connect(&mut h);
    all_disabled_containing(&h, "Store (0x1010)");
    all_disabled_containing(&h, "Restore defaults (0x1011)");
    only(&mut h, vec![Tab::Emergencies]);
    all_disabled(&h, "Read error history (0x1003)");
    all_disabled(&h, "Clear error history");
    only(&mut h, vec![Tab::EdsCheck]);
    all_disabled(&h, &format!("{} Start", icon::PLAY));
}

/// Program download wizard: file, size and SHA-256, the question, then the
/// sequence runs to the end on the simulator.
#[test]
fn s42_program_download_wizard() {
    let tmp = TempDir::new("s42");
    let mut h = build(&tmp.data());
    let file = tmp.0.join("app.bin");
    let data: Vec<u8> = (0..2000u32).map(|i| (i * 13 + 5) as u8).collect();
    std::fs::write(&file, &data).unwrap();
    only(&mut h, vec![Tab::Network]);
    connect(&mut h);
    click(&mut h, "Program download...");
    assert!(has_exact(&h, "Program download"), "wizard\nUI:\n{}", dump(&h));
    assert!(has(&h, "Never download to a machine in operation"), "warning\nUI:\n{}", dump(&h));
    let dl = top_by_label(&h, "Download program", Some(Role::Button));
    assert!(disabled(&h, dl), "no file yet: the button is disabled");
    let path = right_of(&h, "File", Role::TextInput);
    set_text(&mut h, path, &file.display().to_string());
    step2(&mut h);
    assert!(has_exact(&h, "2000 bytes"), "size\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, &crate::progdl::sha256_hex(&data)), "SHA-256\nUI:\n{}", dump(&h));
    let dl = top_by_label(&h, "Download program", Some(Role::Button));
    assert!(!disabled(&h, dl));
    click_id(&mut h, dl);
    // The question first, as for Restore defaults.
    assert!(has(&h, "as program 1 to node 1?"), "question\nUI:\n{}", dump(&h));
    let before: Vec<NodeId> = Vec::new();
    click_new_button(&mut h, "Download", &before);
    // The status line follows the wizard text by a frame: wait for both.
    wait_for(&mut h, "download done", |h| has_exact(h, "Download done") && has(h, "Program download to node 1 done"));
    assert!(has(&h, "2000 / 2000 bytes"), "progress\nUI:\n{}", dump(&h));
    let crc = crate::progdl::crc32(&data);
    assert!(has(&h, &format!("0x{crc:08X} (CRC-32")), "new identification\nUI:\n{}", dump(&h));
}

/// Drive panel: the polled state, the motion group locked until the check
/// box is ticked.
#[test]
fn s43_drive_panel() {
    let tmp = TempDir::new("s43");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Drive]);
    assert!(has_exact(&h, "Not connected: the commands are disabled."), "UI:\n{}", dump(&h));
    connect(&mut h);
    click_role(&mut h, Role::CheckBox, "Poll");
    wait_for(&mut h, "state", |h| has_exact(h, "State: Switch on disabled"));
    // Quick stop and Disable voltage are always enabled while connected.
    assert!(!disabled(&h, control_containing(&h, "  Quick stop")));
    assert!(!disabled(&h, control_containing(&h, "  Disable voltage")));
    // Switch on is not allowed in Switch on disabled.
    assert!(disabled(&h, top_by_label(&h, "Switch on", Some(Role::Button))));
    for b in ["Enable operation", "Write 0x60FF", "Write 0x607A", "Write 0x6071"] {
        assert!(disabled(&h, top_by_label(&h, b, Some(Role::Button))), "{b} locked\nUI:\n{}", dump(&h));
    }
    click_role(&mut h, Role::CheckBox, super::drive_view::DRIVE_CHECK);
    assert!(!disabled(&h, top_by_label(&h, "Write 0x60FF", Some(Role::Button))), "unlocked\nUI:\n{}", dump(&h));
    // Shutdown, then the state follows.
    click_role(&mut h, Role::Button, "Shutdown");
    wait_for(&mut h, "ready", |h| has_exact(h, "State: Ready to switch on"));
    // The tick is cleared on disconnect.
    click(&mut h, "Disconnect");
    wait_for(&mut h, "disconnected", |h| has_exact(h, "Not connected: the commands are disabled."));
    assert!(!h.state().drv.checked, "tick cleared on disconnect");
}

/// Dashboard: Add signal..., the widget dialog, then the widget shows a
/// value.
#[test]
fn s44_dashboard_widget() {
    let tmp = TempDir::new("s44");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Dash");
    connect(&mut h);
    start_node(&mut h);
    only(&mut h, vec![Tab::Dashboard]);
    assert!(has_exact(&h, "No widgets."), "UI:\n{}", dump(&h));
    click_containing(&mut h, "Add signal...");
    assert!(has_exact(&h, "Add signal"), "dialog\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Add");
    assert!(has_exact(&h, "Dashboard widget"), "widget dialog\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Save");
    assert!(!has_exact(&h, "No widgets."), "widget added\nUI:\n{}", dump(&h));
    assert_eq!(h.state().dash.board.widgets.len(), 1);
    wait_for(&mut h, "a value", |h| !has_exact(h, "No value yet"));
    assert!(tmp.data().join("dashboard.json").exists(), "saved");
}

// ------------------------------------------------- 0.5 + dashboard canvas

/// A raw-frame-bits signal of the made-up COB-ID 3A1.
fn raw_def(name: &str, start_bit: u32) -> crate::signals::SignalDef {
    crate::signals::SignalDef::new(name, crate::signals::SignalSource::Raw { cob_id: 0x3A1, extended: false, start_bit, bits: 16, raw_kind: crate::signals::RawKind::Unsigned, big_endian: false })
}

/// Add a widget of `kind` through the widget dialog (Save), as Add
/// signal... does.
fn dash_widget(h: &mut H, def: crate::signals::SignalDef, kind: crate::dashboard::WidgetKind) {
    h.state_mut().dash_add_widget(def);
    h.state_mut().dash.edit.as_mut().unwrap().w.kind = kind;
    step2(h);
    click_role(h, Role::Button, "Save");
    assert!(h.state().dash.edit.is_none(), "widget dialog still open\nUI:\n{}", dump(h));
}

/// A value for every widget (straight into the signal store).
fn dash_values(h: &mut H, v: f64) {
    let ids: Vec<u64> = h.state().dash.board.widgets.iter().filter_map(|w| w.signal_id).collect();
    if let Ok(mut s) = h.state().bus.signals.lock() {
        for id in ids {
            s.push_raw(id, 0.0, v);
        }
    }
    step2(h);
}

fn inside(outer: &egui::Rect, r: &Rect) -> bool {
    r.x0 as f32 >= outer.left() - 0.5 && r.y0 as f32 >= outer.top() - 0.5 && r.x1 as f32 <= outer.right() + 0.5 && r.y1 as f32 <= outer.bottom() + 0.5
}

fn card_of(h: &H, title: &str) -> (usize, egui::Rect) {
    let i = h.state().dash.board.widgets.iter().position(|w| w.title == title).unwrap_or_else(|| panic!("no widget {title}"));
    (i, h.state().dash.cards[i])
}

/// The dashboard cards are laid out top down in their own rect (the 0.5.0
/// fault: one character per line, the title and the buttons outside the
/// card): the value is one line, the title and Edit / Remove lie in the
/// card, cards of one size have one size.
#[test]
fn s45_dashboard_card_geometry() {
    use crate::dashboard::WidgetKind;
    let tmp = TempDir::new("s45");
    let mut h = build_sized(&tmp.data(), 1400.0, 900.0);
    only(&mut h, vec![Tab::Dashboard]);
    let names = ["Battery voltage", "Motor speed", "Coolant temperature", "Pump on"];
    for (i, k) in WidgetKind::ALL.into_iter().enumerate() {
        dash_widget(&mut h, raw_def(names[i], i as u32 * 16), k);
    }
    dash_values(&mut h, 1400.5);
    let cards = h.state().dash.cards.clone();
    assert_eq!(cards.len(), 4);
    for (i, c) in cards.iter().enumerate() {
        assert_eq!(c.size(), cards[0].size(), "card {i} size");
        assert!(c.width() >= 200.0 && c.height() >= 150.0, "card {i} {c:?}");
        // The title.
        let t = h.query_all_by_label(names[i]).find(|n| n.role() == Role::Label).map(|n| rect_of(&n)).unwrap_or_else(|| panic!("title {}\nUI:\n{}", names[i], dump(&h)));
        assert!(inside(c, &t), "title {} {t:?} outside {c:?}", names[i]);
        assert!(t.y1 - t.y0 < 30.0, "title {} on more than one line {t:?}", names[i]);
        // Edit and Remove: one of each in this card.
        for b in ["Edit", "Remove"] {
            let n = h.query_all_by_label(b).filter(|n| n.role() == Role::Button && inside(c, &rect_of(n))).count();
            assert_eq!(n, 1, "{b} in card {i}\nUI:\n{}", dump(&h));
        }
        // The value: one line, wider than high, in the card.
        let v = h.query_all_by_label_contains("1400.5").map(|n| rect_of(&n)).find(|r| inside(c, r)).unwrap_or_else(|| panic!("value in card {i}\nUI:\n{}", dump(&h)));
        let (w, ht) = (v.x1 - v.x0, v.y1 - v.y0);
        assert!(ht < 110.0 && w > 1.5 * ht, "value of card {i} not one line: {w} x {ht}");
        assert!(ht > 15.0, "value of card {i} not scaled up: {ht}");
    }
    // No two cards overlap (placed in a row).
    for i in 1..4 {
        assert!(cards[i].left() >= cards[i - 1].right() + 9.0, "cards {i} {:?}", cards);
    }
    // A larger card: a larger value text.
    let small = h.query_all_by_label_contains("1400.5").map(|n| rect_of(&n)).find(|r| inside(&cards[0], r)).unwrap();
    h.state_mut().dash.board.widgets[0].rect = Some(crate::dashboard::WidgetRect { x: 0.0, y: 300.0, w: 600.0, h: 400.0 });
    step2(&mut h);
    let big_card = h.state().dash.cards[0];
    let big = h.query_all_by_label_contains("1400.5").map(|n| rect_of(&n)).find(|r| inside(&big_card, r)).unwrap();
    assert!(big.y1 - big.y0 > 1.5 * (small.y1 - small.y0), "value text does not scale: {small:?} {big:?}");
}

/// Press at `from`, move to `to` in steps, release (unless `release` is
/// false).
fn drag_to(h: &mut H, from: egui::Pos2, to: egui::Pos2, release: bool) {
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    for i in 1..=8 {
        let p = from + (to - from) * (i as f32 / 8.0);
        h.input_mut().events.push(egui::Event::PointerMoved(p));
        h.step();
    }
    if release {
        h.input_mut().events.push(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        step2(h);
    }
}

fn wrect(h: &H, title: &str) -> crate::dashboard::WidgetRect {
    let i = card_of(h, title).0;
    h.state().dash.board.widgets[i].rect.expect("rect")
}

/// Move by the title row and resize by the corner grip, with the pointer:
/// snap to 10, the minimum size, x / y not below 0, dashboard.json written
/// when the drag ends (not during it), the moved card on top; Arrange.
#[test]
fn s46_dashboard_move_resize_arrange() {
    use crate::dashboard::{WidgetKind, WidgetRect, MIN_H, MIN_W};
    let tmp = TempDir::new("s46");
    let mut h = build_sized(&tmp.data(), 1400.0, 900.0);
    only(&mut h, vec![Tab::Dashboard]);
    dash_widget(&mut h, raw_def("Battery voltage", 0), WidgetKind::Value);
    dash_widget(&mut h, raw_def("Motor speed", 16), WidgetKind::Bar);
    let file = tmp.data().join(crate::dashboard::DASHBOARD_FILE);
    assert_eq!(wrect(&h, "Battery voltage"), WidgetRect { x: 0.0, y: 0.0, w: 240.0, h: 180.0 });
    assert_eq!(wrect(&h, "Motor speed"), WidgetRect { x: 250.0, y: 0.0, w: 240.0, h: 180.0 });
    // Move the first card by (+123, +347): it snaps to (120, 350).
    let before = std::fs::read_to_string(&file).unwrap();
    let c = card_of(&h, "Battery voltage").1;
    let from = c.min + egui::vec2(40.0, 18.0);
    drag_to(&mut h, from, from + egui::vec2(123.0, 347.0), false);
    assert!(h.state().dash.dragging(), "no drag running");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "written during the drag");
    assert_eq!(wrect(&h, "Battery voltage").x, 120.0);
    h.input_mut().events.push(egui::Event::PointerButton { pos: from + egui::vec2(123.0, 347.0), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    step2(&mut h);
    assert!(!h.state().dash.dragging());
    assert_eq!(wrect(&h, "Battery voltage"), WidgetRect { x: 120.0, y: 350.0, w: 240.0, h: 180.0 });
    // The moved card is on top (the end of the list); the file has it.
    assert_eq!(h.state().dash.board.widgets.last().unwrap().title, "Battery voltage");
    let t = std::fs::read_to_string(&file).unwrap();
    assert!(t != before && t.contains("\"x\": 120.0") && t.contains("\"y\": 350.0"), "{t}");
    // Resize the second card far down to the left: the minimum size.
    let c = card_of(&h, "Motor speed").1;
    let g = c.max - egui::vec2(5.0, 5.0);
    drag_to(&mut h, g, g - egui::vec2(400.0, 400.0), true);
    assert_eq!(wrect(&h, "Motor speed"), WidgetRect { x: 250.0, y: 0.0, w: MIN_W, h: MIN_H });
    assert_eq!(h.state().dash.board.widgets.last().unwrap().title, "Motor speed");
    // Larger by (+33, +26): snapped to 150 x 120.
    let c = card_of(&h, "Motor speed").1;
    let g = c.max - egui::vec2(5.0, 5.0);
    drag_to(&mut h, g, g + egui::vec2(33.0, 26.0), true);
    assert_eq!(wrect(&h, "Motor speed"), WidgetRect { x: 250.0, y: 0.0, w: 150.0, h: 120.0 });
    // Moved far up to the left: x and y stop at 0.
    let c = card_of(&h, "Motor speed").1;
    let from = c.min + egui::vec2(30.0, 15.0);
    drag_to(&mut h, from, from - egui::vec2(600.0, 600.0), true);
    assert_eq!(wrect(&h, "Motor speed"), WidgetRect { x: 0.0, y: 0.0, w: 150.0, h: 120.0 });
    let t = std::fs::read_to_string(&file).unwrap();
    assert!(t.contains("\"w\": 150.0") && t.contains("\"h\": 120.0"), "{t}");
    // Arrange: rows in list order, gap 10, the sizes kept.
    click(&mut h, "Arrange");
    let ws = &h.state().dash.board.widgets;
    assert_eq!((ws[0].title.as_str(), ws[0].rect), ("Battery voltage", Some(WidgetRect { x: 0.0, y: 0.0, w: 240.0, h: 180.0 })));
    assert_eq!((ws[1].title.as_str(), ws[1].rect), ("Motor speed", Some(WidgetRect { x: 250.0, y: 0.0, w: 150.0, h: 120.0 })));
    let t = std::fs::read_to_string(&file).unwrap();
    assert!(t.contains("\"x\": 250.0"), "arrange not saved {t}");
    // The file loads in a new start with the same places (round trip).
    let board = h.state().dash.board.clone();
    drop(h);
    let mut h = build_sized(&tmp.data(), 1400.0, 900.0);
    only(&mut h, vec![Tab::Dashboard]);
    let back: Vec<_> = h.state().dash.board.widgets.iter().map(|w| (w.title.clone(), w.rect)).collect();
    let was: Vec<_> = board.widgets.iter().map(|w| (w.title.clone(), w.rect)).collect();
    assert_eq!(back, was);
    assert_eq!(h.state().dash.cards.len(), 2);
}

/// A dashboard.json of 0.5.0 (sizes, no rectangles) loads: the cards are
/// placed by Arrange at a width of 1000 and drawn.
#[test]
fn s47_dashboard_old_file() {
    let tmp = TempDir::new("s47");
    std::fs::create_dir_all(tmp.data()).unwrap();
    let one = r#"{"title":"TITLE","kind":"value","size":"SIZE","signal":{"name":"TITLE","scale":1.0,"offset":0.0,"source":{"kind":"raw","cob_id":929,"extended":false,"start_bit":0,"bits":16,"raw_kind":"unsigned","big_endian":false}}}"#;
    let ws: Vec<String> = [("Battery voltage", "medium"), ("Motor speed", "large"), ("Pump on", "small"), ("Coolant temperature", "medium")].iter().map(|(t, s)| one.replace("TITLE", t).replace("SIZE", s)).collect();
    std::fs::write(tmp.data().join(crate::dashboard::DASHBOARD_FILE), format!(r#"{{"widgets":[{}]}}"#, ws.join(","))).unwrap();
    let mut h = build_sized(&tmp.data(), 1400.0, 900.0);
    only(&mut h, vec![Tab::Dashboard]);
    let r: Vec<(f32, f32, f32, f32)> = h.state().dash.board.widgets.iter().map(|w| w.rect.map(|r| (r.x, r.y, r.w, r.h)).expect("rect")).collect();
    assert_eq!(r, vec![(0.0, 0.0, 240.0, 180.0), (250.0, 0.0, 340.0, 250.0), (600.0, 0.0, 170.0, 130.0), (0.0, 260.0, 240.0, 180.0)]);
    assert_eq!(h.state().dash.cards.len(), 4);
    assert!(has_exact(&h, "Coolant temperature"), "UI:\n{}", dump(&h));
}

// ------------------------------------------------ 0.5 + Messages bit view

/// Select row 3A0 of the analysis frames in the Messages view.
fn messages_3a0(h: &mut H) {
    only(h, vec![Tab::Messages]);
    analysis_frames(h, 40);
    step2(h);
    let r = top_by_label(h, "3A0", None);
    pointer_click(h, r);
    assert!(has_exact(h, "Define signal..."), "detail\nUI:\n{}", dump(h));
}

/// A press and release at `pos` with Shift held (or not).
fn click_at(h: &mut H, pos: egui::Pos2, shift: bool) {
    let m = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
    h.input_mut().modifiers = m;
    h.input_mut().events.push(egui::Event::PointerMoved(pos));
    step2(h);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: m });
        h.step();
    }
    step2(h);
    h.input_mut().modifiers = egui::Modifiers::NONE;
    step2(h);
}

fn click_byte(h: &mut H, byte: usize, shift: bool) {
    let r = rect_id(h, top_by_label(h, &format!("Byte {byte}"), Some(Role::Button)));
    click_at(h, egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32), shift);
}

fn click_bit(h: &mut H, byte: usize, bit: u32, shift: bool) {
    let p = bit_cell(h, byte, bit);
    click_at(h, p, shift);
}

#[track_caller]
fn selected(h: &H, s: &str) {
    assert!(has_exact(h, s), "want {s:?}\nUI:\n{}", dump(h));
}

/// Shift + click extends the selection from its anchor: bits and whole
/// bytes, both directions, LE and BE; Define signal is filled in from it;
/// a user signal with bits in the selection is marked.
#[test]
fn s48_messages_shift_click() {
    let tmp = TempDir::new("s48");
    let mut h = build_sized(&tmp.data(), 1500.0, 1000.0);
    h.state_mut().an.usigs.push(crate::usersig::UserSignal::new("Battery voltage", 0x3A0, false, 8, 16));
    messages_3a0(&mut h);
    // LE bits, forward and back.
    click_bit(&mut h, 0, 4, false);
    selected(&h, "Selected: bit 4, 1 bit");
    click_bit(&mut h, 1, 2, true);
    selected(&h, "Selected: bit 4, 7 bit");
    click_bit(&mut h, 1, 2, false);
    click_bit(&mut h, 0, 4, true);
    selected(&h, "Selected: bit 4, 7 bit");
    // Without Shift a click starts again.
    click_bit(&mut h, 2, 0, false);
    selected(&h, "Selected: bit 16, 1 bit");
    // LE bytes, forward and back.
    click_byte(&mut h, 1, false);
    selected(&h, "Selected: bit 8, 8 bit");
    click_byte(&mut h, 3, true);
    selected(&h, "Selected: bit 8, 24 bit");
    click_byte(&mut h, 3, false);
    click_byte(&mut h, 1, true);
    selected(&h, "Selected: bit 8, 24 bit");
    // The overlap mark: bytes 1-3 hold the user signal (bits 8..23).
    assert!(has_exact(&h, "In the selection"), "no overlap mark\nUI:\n{}", dump(&h));
    click_byte(&mut h, 0, false);
    assert!(!has_exact(&h, "In the selection"), "overlap mark for byte 0");
    // BE bits: grid reading order (byte 0 bit 4 .. byte 1 bit 2 = 11 cells).
    click(&mut h, "BE");
    click_bit(&mut h, 0, 4, false);
    click_bit(&mut h, 1, 2, true);
    selected(&h, "Selected: bit 4 (MSB), 11 bit, big endian");
    click_bit(&mut h, 1, 2, false);
    click_bit(&mut h, 0, 4, true);
    selected(&h, "Selected: bit 4 (MSB), 11 bit, big endian");
    // BE bytes, back and forward: bytes 1-2, MSB = bit 15.
    click_byte(&mut h, 2, false);
    selected(&h, "Selected: bit 23 (MSB), 8 bit, big endian");
    click_byte(&mut h, 1, true);
    selected(&h, "Selected: bit 15 (MSB), 16 bit, big endian");
    click_byte(&mut h, 1, false);
    click_byte(&mut h, 2, true);
    selected(&h, "Selected: bit 15 (MSB), 16 bit, big endian");
    // Define signal is filled in from the extended selection.
    click(&mut h, "Define signal...");
    let d = h.state().an.draft.as_ref().expect("draft");
    assert_eq!((d.sig.cob_id, d.sig.start_bit, d.sig.bits, d.sig.big_endian), (0x3A0, 15, 16, true));
}

/// The bit view at the bottom, left, right or top: the table and the bit
/// view both show; the choice is saved in settings.json and loads again.
#[test]
fn s49_messages_bit_view_position() {
    use crate::settings::DetailPos;
    let tmp = TempDir::new("s49");
    let mut h = build_sized(&tmp.data(), 1500.0, 1000.0);
    messages_3a0(&mut h);
    for pos in [DetailPos::Left, DetailPos::Right, DetailPos::Top, DetailPos::Bottom] {
        click(&mut h, &format!("Bit view {}", pos.label()));
        assert_eq!(h.state().settings.msg_detail_pos, pos);
        // The table (its header and the row) and the bit view.
        for t in ["COB-ID", "3A0", "Reset statistics", "Byte 0", "Define signal..."] {
            assert!(has_exact(&h, t), "{pos:?}: no {t:?}\nUI:\n{}", dump(&h));
        }
        let table = rect_id(&h, top_by_label(&h, "3A0", None));
        let detail = rect_id(&h, top_by_label(&h, "Byte 0", Some(Role::Button)));
        let ok = match pos {
            DetailPos::Bottom => detail.y0 > table.y1,
            DetailPos::Top => detail.y1 < table.y0,
            DetailPos::Left => detail.x1 < table.x0,
            DetailPos::Right => detail.x0 > table.x1,
        };
        assert!(ok, "{pos:?}: table {table:?} bit view {detail:?}");
    }
    click(&mut h, "Bit view Right");
    h.state_mut().save_settings();
    drop(h);
    let (s, err) = Settings::load(&tmp.data());
    assert!(err.is_none(), "{err:?}");
    assert_eq!(s.msg_detail_pos, DetailPos::Right);
    let h = build_sized(&tmp.data(), 1500.0, 1000.0);
    assert_eq!(h.state().settings.msg_detail_pos, DetailPos::Right);
}

fn graph_count(h: &H) -> usize {
    h.state().bus.signals.lock().map(|s| s.graph_signals().count()).unwrap_or(0)
}

/// The Graph and Dashboard buttons of a user signal and of a mapped PDO
/// signal in the bit view; no second Graph signal for the same source.
#[test]
fn s50_messages_signal_buttons() {
    let tmp = TempDir::new("s50");
    let mut h = build_sized(&tmp.data(), 1500.0, 1000.0);
    h.state_mut().an.usigs.push(crate::usersig::UserSignal::new("Battery voltage", 0x3A0, false, 8, 16));
    messages_3a0(&mut h);
    click_role(&mut h, Role::Button, "Add Battery voltage to the Graph");
    assert_eq!(graph_count(&h), 1);
    assert!(has(&h, "Signal Battery voltage added to the Graph"), "UI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Add Battery voltage to the Graph");
    assert_eq!(graph_count(&h), 1, "a second Graph signal");
    assert!(has(&h, "Signal Battery voltage is already in the Graph"), "UI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Add Battery voltage to the Dashboard");
    assert_eq!(graph_count(&h), 1, "a second Graph signal for the dashboard");
    assert!(leaves(&h).iter().flatten().any(|t| *t == Tab::Dashboard), "Dashboard not open");
    assert!(has_exact(&h, "Dashboard widget"), "widget dialog\nUI:\n{}", dump(&h));
    click_role(&mut h, Role::Button, "Save");
    assert_eq!(h.state().dash.board.widgets.len(), 1);
    assert!(has(&h, "Dashboard widget Battery voltage added"), "UI:\n{}", dump(&h));
    // A mapped PDO signal (TPDO1 of node 1 from test.eds).
    drop(h);
    let mut h2 = build_sized(&tmp.data(), 1500.0, 1000.0);
    add_eds_and_device(&mut h2, "Pdo");
    only(&mut h2, vec![Tab::Messages]);
    for i in 0..10u64 {
        msg_frame(&mut h2, 0x181, &[i as u8, 1, 2, 3, 4, 5, 6, 7], 1_000_000 + i * 20_000);
    }
    step2(&mut h2);
    let r = top_by_label(&h2, "181", None);
    pointer_click(&mut h2, r);
    let n0 = graph_count(&h2);
    let g = h2.query_all_by_label_contains("to the Graph").find(|n| n.role() == Role::Button && text_of(n).starts_with("Add N1 ")).map(|n| (n.id(), text_of(&n))).unwrap_or_else(|| panic!("no PDO Graph button\nUI:\n{}", dump(&h2)));
    click_id(&mut h2, g.0);
    assert_eq!(graph_count(&h2), n0 + 1);
    let src = h2.state().bus.signals.lock().unwrap().graph_signals().last().map(|s| s.def.source.clone());
    assert!(matches!(src, Some(crate::signals::SignalSource::Pdo { node: 1, .. })), "{src:?}");
    click_id(&mut h2, g.0);
    assert_eq!(graph_count(&h2), n0 + 1, "a second PDO Graph signal");
    let d = g.1.replace("to the Graph", "to the Dashboard");
    click_role(&mut h2, Role::Button, &d);
    assert_eq!(graph_count(&h2), n0 + 1);
    assert!(has_exact(&h2, "Dashboard widget"), "widget dialog\nUI:\n{}", dump(&h2));
}

// ------------------------------------------------- toolbar layout sweep

/// The widget nodes of the tree with their rectangles: not the text runs
/// of a text field or a label, and not the containers.
fn widget_rects(h: &H) -> Vec<(String, Role, Rect)> {
    h.query_all_by(|n| !matches!(n.role(), Role::TextRun | Role::Unknown | Role::GenericContainer | Role::Window | Role::Group))
        .filter_map(|n| n.raw_bounds().map(|r| (text_of(&n), n.role(), r)))
        .collect()
}

/// The faults of the toolbar rows of the last frame: two widgets of one row
/// whose rectangles overlap by more than 1 point in both axes, or a widget
/// that reaches outside the view.
fn toolbar_faults(h: &H, what: &str) -> Vec<String> {
    row_faults(h, theme::toolbar_rects::last(&h.ctx), what)
}

/// The faults of these (view room, row rect) rows; see `toolbar_faults`.
fn row_faults(h: &H, rows: Vec<(egui::Rect, egui::Rect)>, what: &str) -> Vec<String> {
    let nodes = widget_rects(h);
    let mut out = Vec::new();
    for (panel, row) in rows {
        // The widgets of the row: their centre is in the row, and not below
        // the view (for example the status bar under a row that is cut off).
        let inside = |r: &Rect| {
            let (cx, cy) = (((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32);
            row.expand(1.0).contains(egui::pos2(cx, cy)) && cy <= panel.bottom() + 1.0
        };
        let ws: Vec<&(String, Role, Rect)> = nodes.iter().filter(|(_, _, r)| inside(r)).collect();
        for (t, role, r) in &ws {
            if r.x0 < panel.left() as f64 - 1.0 || r.x1 > panel.right() as f64 + 1.0 {
                out.push(format!("{what}: {role:?} {t:?} x {:.0}..{:.0} outside the view x {:.0}..{:.0}", r.x0, r.x1, panel.left(), panel.right()));
            }
        }
        // A label that egui wraps onto the next row allocates each text row
        // on its own; its node has the bounding box of all of them, which
        // covers the widgets before it. Such a label is checked only for
        // the view bounds.
        let one_row = |r: &Rect, role: Role| role != Role::Label || r.y1 - r.y0 <= h.ctx.style().spacing.interact_size.y as f64 + 4.0;
        for i in 0..ws.len() {
            if !one_row(&ws[i].2, ws[i].1) {
                continue;
            }
            for j in i + 1..ws.len() {
                if !one_row(&ws[j].2, ws[j].1) {
                    continue;
                }
                let (a, b) = (&ws[i].2, &ws[j].2);
                let ox = a.x1.min(b.x1) - a.x0.max(b.x0);
                let oy = a.y1.min(b.y1) - a.y0.max(b.y0);
                if ox > 1.0 && oy > 1.0 {
                    out.push(format!(
                        "{what}: {:?} {:?} ({:.0},{:.0})..({:.0},{:.0}) overlaps {:?} {:?} ({:.0},{:.0})..({:.0},{:.0})",
                        ws[i].1, ws[i].0, a.x0, a.y0, a.x1, a.y1, ws[j].1, ws[j].0, b.x0, b.y0, b.x1, b.y1
                    ));
                }
            }
        }
    }
    out
}

/// Show `tab` alone at window widths 300..=1400 (every 20 points) and
/// collect the toolbar faults of each width.
fn sweep_toolbars(h: &mut H, tab: Tab, name: &str, expect_rows: usize) -> Vec<String> {
    only(h, vec![tab]);
    let mut out = Vec::new();
    for w in (300..=1400).step_by(20) {
        h.set_size(egui::Vec2::new(w as f32, 900.0));
        step2(h);
        theme::toolbar_rects::clear(&h.ctx);
        step2(h);
        let n = theme::toolbar_rects::last(&h.ctx).len();
        assert!(n >= expect_rows, "{name} at {w}: {n} toolbar rows, expected at least {expect_rows}\nUI:\n{}", dump(h));
        out.extend(toolbar_faults(h, &format!("{name} at {w}")));
    }
    out
}

/// The toolbar rows wrap instead of drawing widgets over each other (the
/// Trace counters over the buttons, the Manual SDO combo boxes under the
/// next button), and no widget of a toolbar reaches outside the view, at
/// any width of the view.
#[test]
fn toolbar_rows_do_not_overlap() {
    let tmp = TempDir::new("toolbars");
    let mut h = build(&tmp.data());
    // An active device with an EDS: the Manual SDO row shows Type from EDS.
    add_eds_and_device(&mut h, "Bench");
    // The first faults of each view (all of them would be too long).
    let mut faults = Vec::new();
    let mut n = 0;
    let mut add = |v: Vec<String>| {
        n += v.len();
        faults.extend(v.into_iter().take(12));
    };
    add(sweep_toolbars(&mut h, Tab::Trace(1), "Trace", 1));
    add(sweep_toolbars(&mut h, Tab::Od, "Object dictionary", 2));
    assert!(has_exact(&h, "Type from EDS"), "Manual SDO row without Type from EDS\nUI:\n{}", dump(&h));
    add(sweep_toolbars(&mut h, Tab::Messages, "Messages", 1));
    add(sweep_toolbars(&mut h, Tab::Graph, "Graph", 1));
    add(sweep_toolbars(&mut h, Tab::Network, "Network", 5));
    add(sweep_toolbars(&mut h, Tab::TxList, "Transmit list", 3));
    assert!(n == 0, "{n} toolbar fault(s), the first of each view:\n{}", faults.join("\n"));
}

/// The connection bar and the status bar draw no widget over another one
/// and none outside the window, at window widths from the smallest one of
/// the app (`MIN_WINDOW`) to 2000 points: with the control of each adapter
/// kind (with a long driver note), disconnected and connected (Simulator,
/// listen only), and with a long status message.
///
/// The adapter kinds other than the Simulator are set in the settings of
/// the running app, not picked in the list: the app then sends no
/// `ListChannels`, so no driver is loaded and no COM port is listed, and
/// Connect is pressed only with the Simulator.
#[test]
fn window_bars_do_not_overlap() {
    let tmp = TempDir::new("bars");
    let mut h = build(&tmp.data());
    only(&mut h, vec![Tab::Trace(1)]);
    let long = "Device Bench saved; the object dictionary was read again and 214 entries were compared with the EDS of the device";
    let note = "No channel list reported: the driver answered with an error, choose a channel by hand";
    let min = super::MIN_WINDOW[0] as u32;
    let mut faults = Vec::new();
    let mut n = 0;
    let mut sweep = |h: &mut H, name: &str| {
        let mut v = Vec::new();
        for w in (min..=2000).step_by(20) {
            h.set_size(egui::Vec2::new(w as f32, 800.0));
            step2(h);
            theme::toolbar_rects::clear(&h.ctx);
            step2(h);
            let rows = theme::toolbar_rects::last_bars(&h.ctx);
            assert_eq!(rows.len(), 2, "{name} at {w}: the connection bar and the status bar are not both recorded");
            v.extend(row_faults(h, rows, &format!("{name} at {w}")));
        }
        n += v.len();
        faults.extend(v.into_iter().take(12));
    };
    let kinds = [AdapterKind::Simulator, AdapterKind::Peak, AdapterKind::Canfox, AdapterKind::Kvaser, AdapterKind::SlcanSerial, AdapterKind::SlcanTcp, AdapterKind::Replay];
    for kind in kinds {
        {
            let app = h.state_mut();
            app.settings.adapter = kind;
            app.settings.slcan_tcp = "192.168.100.200:20001".to_string();
            app.settings.replay_file = tmp.data().join("a_rather_long_name_of_a_bench_log_file.trc").display().to_string();
            app.channel_note = Some(note.to_string());
            app.set_status(long, true);
        }
        sweep(&mut h, &format!("{kind:?}"));
    }
    {
        let app = h.state_mut();
        app.settings.adapter = AdapterKind::Simulator;
        app.channel_note = None;
        app.settings.listen_only = true;
    }
    h.set_size(egui::Vec2::new(1600.0, 800.0));
    step2(&mut h);
    connect(&mut h);
    h.state_mut().set_status(long, false);
    sweep(&mut h, "Simulator connected");
    assert!(n == 0, "{n} bar fault(s), the first of each case:\n{}", faults.join("\n"));
}
