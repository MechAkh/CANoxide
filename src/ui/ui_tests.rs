//! Headless UI tests (egui_kittest).
//!
//! Each test runs the real `App` (the real `update` code and the real bus
//! worker thread) with the Simulator adapter, in its own temporary data
//! folder, with no window and no GPU. Widgets are found and clicked through
//! the accessibility tree; every assertion reads labels, values, toggle and
//! disabled states from that tree.
//!
//! No test selects the PEAK adapter, so no test loads the PCAN driver, opens
//! a channel or reads the channel list: `build` refuses settings that name
//! the PEAK adapter before the app (and its first `ListChannels`) starts.

use super::{App, Boot};
use crate::bus::BusCommand;
use crate::settings::{AdapterKind, Settings};
use eframe::egui;
use egui::accesskit::{NodeId, Rect, Role, Toggled};
use egui_kittest::kittest::{Key, Node, Queryable};
use egui_kittest::Harness;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type H = Harness<'static, App>;

/// Longest wait for one UI condition.
const WAIT: Duration = Duration::from_secs(6);

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

/// Build the app on a data folder, as `ui::run` does, but without a window.
fn build(dir: &Path) -> H {
    let boot = Boot::load_dir(dir.to_path_buf());
    assert_eq!(boot.settings.adapter, AdapterKind::Simulator, "a UI test must never use the PEAK adapter");
    Harness::builder().with_size(egui::Vec2::new(1600.0, 1200.0)).build_eframe(move |cc| App::new(cc, boot))
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

/// Text of the first node whose text contains `s`.
fn text_containing(h: &H, s: &str) -> Option<String> {
    h.query_all_by_label_contains(s).next().map(|n| text_of(&n))
}

/// Id of the node with this exact label that is highest on the screen.
#[track_caller]
fn top_by_label(h: &H, label: &str, role: Option<Role>) -> NodeId {
    let mut best: Option<(f64, NodeId)> = None;
    for n in h.query_all_by_label(label) {
        if role.is_some_and(|r| n.role() != r) {
            continue;
        }
        let y = rect_of(&n).y0;
        if best.is_none_or(|(by, _)| y < by) {
            best = Some((y, n.id()));
        }
    }
    match best {
        Some((_, id)) => id,
        None => panic!("no node labelled {label:?} ({role:?})\nUI:\n{}", dump(h)),
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
    let a = rect_id(h, top_by_label(h, label, Some(Role::Label)));
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
/// (the header nearest above the anchor).
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
        let d = (r.x0 - hx).abs();
        if (cy(&r) - cy(&a)).abs() < 6.0 && d < 6.0 && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, n.id()));
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

/// Every node with this exact label is disabled (and there is at least one).
#[track_caller]
fn all_disabled(h: &H, label: &str) {
    let v: Vec<bool> = h.query_all_by_label(label).filter(|n| n.role() != Role::Label).map(|n| n.is_disabled()).collect();
    assert!(!v.is_empty(), "no control {label:?}\nUI:\n{}", dump(h));
    assert!(v.iter().all(|d| *d), "control {label:?} is enabled\nUI:\n{}", dump(h));
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

/// (shown, in buffer, received) of the trace tab.
#[track_caller]
fn trace_counts(h: &H) -> (usize, usize, u64) {
    let t = text_containing(h, " shown, ").unwrap_or_else(|| panic!("no trace count line\nUI:\n{}", dump(h)));
    let num = |s: &str| s.trim().split(' ').next().unwrap_or("").parse::<u64>().unwrap_or(u64::MAX);
    let parts: Vec<&str> = t.split(',').collect();
    (num(parts[0]) as usize, num(parts[1]) as usize, num(parts[2]))
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

/// Add test.eds by drag and drop and add the device "Bench" (node 1) with it.
fn add_eds_and_device(h: &mut H, name: &str) {
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

// ================================================================== tests

/// Scenario 1: start state, send controls disabled while not connected.
#[test]
fn s1_start_not_connected() {
    let tmp = TempDir::new("s1");
    let mut h = build(&tmp.data());
    assert!(has_exact(&h, "Not connected"));
    assert!(has_exact(&h, "Connect"));
    // Object dictionary tab (start tab): manual SDO box.
    assert!(has_exact(&h, "not connected"));
    all_disabled(&h, "Read");
    all_disabled(&h, "Write");
    all_disabled(&h, "Read all (select an object)");
    // Network tab.
    click(&mut h, "Network");
    assert!(has_exact(&h, "Not connected: send controls are disabled."));
    for l in ["Start", "Stop", "Enter pre-operational", "Reset node", "Reset communication", "Send SYNC", "Scan nodes", "Send once", "Start cyclic", "Trigger EMCY"] {
        all_disabled(&h, l);
    }
    assert_eq!(count_exact(&h, "Start"), 2, "NMT Start for the node and for all nodes");
    // A command that reaches the worker anyway is refused.
    h.state().bus.send(BusCommand::Sync);
    wait_for(&mut h, "refusal", |h| has(h, "SYNC: not connected"));
}

/// Scenario 2: connect, disconnect, reconnect.
#[test]
fn s2_connect_disconnect_reconnect() {
    let tmp = TempDir::new("s2");
    let mut h = build(&tmp.data());
    connect(&mut h);
    assert!(!has_exact(&h, "Not connected"));
    assert!(has(&h, "Bus: "), "bus status shown");
    assert!(has(&h, "frames/s"));
    assert!(has_exact(&h, "Connected: Simulator at 250 kbit/s"));
    // Adapter, bitrate and listen-only are locked while connected.
    // (The manual SDO box has a type combo box of its own, lower down.)
    let bar: Vec<bool> = h.query_all_by_role(Role::ComboBox).filter(|n| rect_of(n).y0 < 20.0).map(|n| n.is_disabled()).collect();
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
#[test]
fn s3_trace() {
    let tmp = TempDir::new("s3");
    let mut h = build(&tmp.data());
    connect(&mut h);
    click(&mut h, "Trace");
    wait_for(&mut h, "heartbeat row", |h| has_exact(h, "Heartbeat Pre-operational"));
    assert!(has_exact(&h, "Boot-up"), "boot-up row of the simulated node");
    // Type column of the heartbeat rows.
    let hb = last_row(&h, "701").expect("row 701");
    assert_eq!(cell_text(&h, hb, "Type"), "Heartbeat");
    assert_eq!(cell_text(&h, hb, "Node"), "1");
    assert_eq!(cell_text(&h, hb, "Dir"), "Rx");
    let (_, _, rx0) = trace_counts(&h);
    wait_for(&mut h, "received counter rises", |h| trace_counts(h).2 > rx0);

    // Pause: no new rows; the frames are counted as not recorded.
    click_role(&mut h, Role::CheckBox, "Pause");
    let (shown, buf, total) = trace_counts(&h);
    wait_for(&mut h, "paused frames counted", |h| has(h, "frame(s) not recorded"));
    assert_eq!(trace_counts(&h), (shown, buf, total), "trace changed while paused");
    click_role(&mut h, Role::CheckBox, "Pause");
    wait_for(&mut h, "recording again", |h| trace_counts(h).2 > total);

    // Clear.
    click(&mut h, "Clear");
    let (shown, buf, _) = trace_counts(&h);
    assert_eq!((shown, buf), (0, 0), "Clear did not empty the list\nUI:\n{}", dump(&h));

    // A known Tx frame "41 42" on 0x123.
    click(&mut h, "Network");
    let data = right_of(&h, "Data", Role::TextInput);
    set_text(&mut h, data, "41 42");
    click(&mut h, "Send once");
    click(&mut h, "Trace");
    wait_for(&mut h, "Tx row 123", |h| last_row(h, "123").is_some());
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Dir"), "Tx");
    assert_eq!(cell_text(&h, row, "DLC"), "2");
    assert_eq!(cell_text(&h, row, "Data"), "41 42");
    click(&mut h, "Decimal");
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Data"), "65 66");
    click(&mut h, "String");
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Data"), "AB");
    click(&mut h, "Hex");
    let row = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, row, "Data"), "41 42");

    // Fixed: one row per COB-ID, the count rises.
    click(&mut h, "Fixed");
    // (Clear emptied the buffer a moment ago: wait for two heartbeats.)
    let count = |h: &H| last_row(h, "701").map(|r| cell_text(h, r, "Count").parse::<u64>().unwrap_or(0)).unwrap_or(0);
    wait_for(&mut h, "two heartbeats in the fixed view", |h| count(h) >= 2);
    assert_eq!(count_exact(&h, "701"), 1, "fixed view: one row for 0x701\nUI:\n{}", dump(&h));
    assert_eq!(count_exact(&h, "123"), 1, "fixed view: one row for 0x123");
    let c0 = count(&h);
    wait_for(&mut h, "fixed count rises", |h| count(h) > c0);
    assert_eq!(count_exact(&h, "701"), 1, "fixed view: still one row for 0x701");
    click(&mut h, "Chronological");

    // Node filter: node 1 hides the Tx frame (no node), node 5 hides all.
    let (all, _, _) = trace_counts(&h);
    let node_f = right_of(&h, "Node", Role::TextInput);
    set_text(&mut h, node_f, "1");
    let (n1, _, _) = trace_counts(&h);
    assert!(n1 < all, "node filter 1: {n1} of {all} rows");
    assert!(last_row(&h, "123").is_none(), "Tx frame without node shown with node filter 1");
    set_text(&mut h, node_f, "5");
    assert_eq!(trace_counts(&h).0, 0, "node filter 5 shows rows\nUI:\n{}", dump(&h));
    click(&mut h, "Reset filters");
    assert!(trace_counts(&h).0 >= all, "Reset filters");

    // Type filter: no heartbeat rows.
    click(&mut h, "Types (11/11)");
    click_role(&mut h, Role::CheckBox, "Heartbeat");
    assert!(has_exact(&h, "Types (10/11)"), "type count\nUI:\n{}", dump(&h));
    let (shown, buf, _) = trace_counts(&h);
    assert!(shown < buf, "type filter: {shown} of {buf} rows");
    assert!(last_row(&h, "701").is_none(), "heartbeat row shown with the Heartbeat type off\nUI:\n{}", dump(&h));
}

/// Scenario 4: node list and NMT, TPDOs, scan, raw frame, SYNC.
#[test]
fn s4_network() {
    let tmp = TempDir::new("s4");
    // A short scan timeout from the settings file, so a full scan is quick.
    std::fs::create_dir_all(tmp.data()).unwrap();
    Settings { scan_timeout_ms: 20, ..Settings::default() }.save(&tmp.data()).unwrap();
    let mut h = build(&tmp.data());
    connect(&mut h);
    click(&mut h, "Network");
    wait_for(&mut h, "node 1 in the list", |h| has_exact(h, "Pre-operational") && has_exact(h, "Node 1"));
    let start = top_by_label(&h, "Start", Some(Role::Button));
    click_id(&mut h, start);
    wait_for(&mut h, "Operational", |h| has_exact(h, "Operational"));
    assert!(has_exact(&h, "NMT Start node 1 sent"));

    // TPDOs in the trace while operational.
    click(&mut h, "Trace");
    wait_for(&mut h, "TPDO rows", |h| has_exact(h, "TPDO1"));
    click(&mut h, "Network");

    let id = top_by_label(&h, "Stop", Some(Role::Button));

    click_id(&mut h, id);
    wait_for(&mut h, "Stopped", |h| has_exact(h, "Stopped"));
    let id = top_by_label(&h, "Enter pre-operational", Some(Role::Button));
    click_id(&mut h, id);
    wait_for(&mut h, "Pre-operational", |h| has_exact(h, "Pre-operational"));
    let id = top_by_label(&h, "Start", Some(Role::Button));
    click_id(&mut h, id);
    wait_for(&mut h, "Operational again", |h| has_exact(h, "Operational"));
    let id = top_by_label(&h, "Reset node", Some(Role::Button));
    click_id(&mut h, id);
    wait_for(&mut h, "boot-up after reset", |h| has_exact(h, "Boot-up"));
    wait_for(&mut h, "pre-operational after boot-up", |h| has_exact(h, "Pre-operational"));

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

    // Raw frame once, then SYNC.
    click(&mut h, "Send once");
    click(&mut h, "Send SYNC");
    assert!(has_exact(&h, "SYNC sent"));
    click(&mut h, "Trace");
    wait_for(&mut h, "Tx 123 and SYNC rows", |h| last_row(h, "123").is_some() && last_row(h, "080").is_some());
    let r = last_row(&h, "123").unwrap();
    assert_eq!(cell_text(&h, r, "Dir"), "Tx");
    assert_eq!(cell_text(&h, r, "Data"), "01 02");
    let s = last_row(&h, "080").unwrap();
    assert_eq!(cell_text(&h, s, "Type"), "SYNC");
    assert_eq!(cell_text(&h, s, "Decoded"), "SYNC");
    assert_eq!(cell_text(&h, s, "Dir"), "Tx");
}

/// Scenario 5: EDS library, device, object dictionary, decoded trace.
#[test]
fn s5_eds_device_od() {
    let tmp = TempDir::new("s5");
    let mut h = build(&tmp.data());
    add_eds_and_device(&mut h, "Bench");
    assert!(has_exact(&h, "Bench, node 1"));
    // The simulator uses the EDS of the active device: connect after it.
    connect(&mut h);

    // Objects listed.
    for l in ["Device type", "Manufacturer device name", "Identity object", "Mode", "Read analog input 16 bit"] {
        assert!(has_exact(&h, l), "object {l:?} not listed\nUI:\n{}", dump(&h));
    }
    // Filter narrows.
    let filter = right_of(&h, "Filter", Role::TextInput);
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
    wait_for(&mut h, "0x1008 value", |h| !v(h).is_empty() && v(h) != "...");
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
    click(&mut h, "Trace");
    let text = right_of(&h, "Text", Role::TextInput);
    set_text(&mut h, text, "device type");
    assert!(has(&h, "SDO upload req 0x1000:00 (Device type)"), "\nUI:\n{}", dump(&h));
    assert!(has(&h, "SDO upload resp 0x1000:00 = 0x00030191 (Device type)"), "\nUI:\n{}", dump(&h));
    click(&mut h, "Reset filters");
    // PDO rows carry signal names once the node is operational.
    click(&mut h, "Network");
    let id = top_by_label(&h, "Start", Some(Role::Button));
    click_id(&mut h, id);
    click(&mut h, "Trace");
    wait_for(&mut h, "TPDO1 with signal names", |h| has(h, "Read input 8 bit / Inputs 1-8 = "));
    assert!(has(&h, "Read analog input 16 bit / Analog input 1 = "));
}

/// Scenario 6: manual SDO without an EDS.
#[test]
fn s6_manual_sdo() {
    let tmp = TempDir::new("s6");
    let mut h = build(&tmp.data());
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

/// Scenario 7: listen only disables every send control.
#[test]
fn s7_listen_only() {
    let tmp = TempDir::new("s7");
    let mut h = build(&tmp.data());
    click_role(&mut h, Role::CheckBox, "Listen only");
    connect(&mut h);
    assert!(has_exact(&h, "LISTEN ONLY"));
    assert!(has_exact(&h, "Connected: Simulator at 250 kbit/s, listen only"));
    assert!(has_exact(&h, "listen only: SDO disabled"));
    all_disabled(&h, "Read");
    all_disabled(&h, "Write");
    click(&mut h, "Network");
    assert!(has_exact(&h, "Listen only: every send control is disabled."));
    for l in ["Start", "Stop", "Enter pre-operational", "Reset node", "Reset communication", "Send SYNC", "Scan nodes", "Send once", "Start cyclic"] {
        all_disabled(&h, l);
    }
    // The node is still heard.
    wait_for(&mut h, "heartbeat heard", |h| has_exact(h, "Pre-operational"));
    // A send command that reaches the worker is refused.
    h.state().bus.send(BusCommand::Sync);
    wait_for(&mut h, "refusal", |h| has(h, "SYNC: listen-only mode, nothing sent"));
    click(&mut h, "Trace");
    assert!(last_row(&h, "080").is_none(), "a SYNC was sent in listen-only mode");
}

/// Scenario 8: settings and devices survive a restart.
#[test]
fn s8_persistence() {
    let tmp = TempDir::new("s8");
    {
        let mut h = build(&tmp.data());
        click(&mut h, "Decimal"); // OD value display
        click(&mut h, "Trace");
        click(&mut h, "String"); // trace display
        click(&mut h, "Object dictionary");
        let combo = right_of(&h, "Bitrate", Role::ComboBox);
        click_id(&mut h, combo);
        click_role(&mut h, Role::Button, "500 kbit/s");
        add_eds_and_device(&mut h, "Persist");
        // The settings are saved one second after the last change.
        run_for(&mut h, Duration::from_millis(1500));
    }
    let mut h = build(&tmp.data());
    let dev = top_by_label(&h, "Persist  (node 1)", None);
    assert!(toggled(&h, dev), "device not active after restart\nUI:\n{}", dump(&h));
    assert!(has_exact(&h, "Persist, node 1"));
    assert!(toggled(&h, top_by_label(&h, "Decimal", Some(Role::Button))), "value display not restored");
    assert!(has_exact(&h, "test.eds"), "EDS file in the library");
    let combo = right_of(&h, "Bitrate", Role::ComboBox);
    click_id(&mut h, combo);
    assert!(toggled(&h, top_by_label(&h, "500 kbit/s", Some(Role::Button))), "bitrate not restored\nUI:\n{}", dump(&h));
    click_id(&mut h, combo);
    click(&mut h, "Trace");
    assert!(toggled(&h, top_by_label(&h, "String", Some(Role::Button))), "trace display not restored");
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
    assert!(disabled(&h, top_by_label(&h, "Add", Some(Role::Button))), "Add device enabled without a library");
    assert!(disabled(&h, top_by_label(&h, "Add...", Some(Role::Button))), "Add EDS enabled without a library");
    drop_files(&mut h, &[test_eds()]);
    assert!(has_exact(&h, "The library is not available (data folder error)"));
    // A settings change and the autosave time: no panic, nothing written.
    click(&mut h, "Decimal");
    run_for(&mut h, Duration::from_millis(1300));
    assert!(!dir.exists());
    // The app still works.
    connect(&mut h);
}
