//! Object dictionary tab: EDS table with Read / Write per entry, Read all,
//! and a manual SDO box that works without an EDS (with the transfer mode,
//! upload to a file, download from a file, progress and cancel).

use super::{theme, App};
use crate::bus::BusCommand;
use crate::canopen::sdo::{SdoOutcome, SdoRequest, TransferMode};
use crate::canopen::types::{check_limits, format_value, parse_int, parse_text, type_name, DataType, DisplayMode};
use crate::eds::{eval_int, OdEntry, ObjectDictionary};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as icon;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) enum Pending {
    Row { index: u16, sub: u8, write: Option<Vec<u8>> },
    Manual { write: bool },
    /// Upload into this file.
    ToFile(PathBuf),
    /// Download of a file (its size).
    FromFile(usize),
}

enum RowValue {
    Value(Vec<u8>),
    Error(String),
    Busy,
}

pub(crate) struct OdView {
    filter: String,
    expanded: HashSet<u16>,
    selected: Option<u16>,
    values: HashMap<(u16, u8), RowValue>,
    edits: HashMap<(u16, u8), String>,
    pub(crate) pending: HashMap<u64, Pending>,
    m_index: String,
    m_sub: String,
    m_type: Option<DataType>,
    m_value: String,
    m_result: String,
    m_error: bool,
    m_data: Option<Vec<u8>>,
}

impl Default for OdView {
    fn default() -> Self {
        OdView {
            filter: String::new(),
            expanded: HashSet::new(),
            selected: None,
            values: HashMap::new(),
            edits: HashMap::new(),
            pending: HashMap::new(),
            m_index: "1000".to_string(),
            m_sub: "0".to_string(),
            m_type: Some(DataType::Unsigned32),
            m_value: String::new(),
            m_result: String::new(),
            m_error: false,
            m_data: None,
        }
    }
}

impl OdView {
    /// Forget the read values (another device or node).
    pub(crate) fn reset_values(&mut self) {
        self.values.clear();
        self.edits.clear();
    }
}

#[derive(Clone, Copy)]
enum RowRef {
    /// Header row of an ARRAY / RECORD.
    Object(u16),
    Entry(u16, u8),
}

fn matches(od: &ObjectDictionary, index: u16, f: &str) -> bool {
    if f.is_empty() {
        return true;
    }
    let o = &od.objects[&index];
    format!("{index:04x}").contains(f)
        || format!("0x{index:04x}").contains(f)
        || o.name.to_lowercase().contains(f)
        || o.subs.values().any(|e| e.name.to_lowercase().contains(f))
}

/// Parse "0x1018", "1018" (hex) for an index.
fn parse_index(s: &str) -> Option<u16> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u16::from_str_radix(t, 16).ok()
}

fn parse_sub(s: &str) -> Option<u8> {
    parse_int(s.trim(), false).and_then(|v| u8::try_from(v).ok())
}

/// A limit text with `$NODEID` evaluated; other texts stay as they are.
fn limit_text(t: Option<&str>, node: u8) -> Option<String> {
    let t = t?;
    if t.to_ascii_uppercase().contains("$NODEID") {
        eval_int(t, node).map(|v| v.to_string())
    } else {
        Some(t.to_string())
    }
}

/// Text from the user to bytes, checked against type and limits.
fn entry_bytes(e: &OdEntry, text: &str, node: u8) -> Result<Vec<u8>, String> {
    let dt = e.dtype().unwrap_or(DataType::OctetString);
    let b = parse_text(dt, text, false).map_err(|x| x.to_string())?;
    let lo = limit_text(e.low.as_deref(), node);
    let hi = limit_text(e.high.as_deref(), node);
    check_limits(dt, &b, lo.as_deref(), hi.as_deref()).map_err(|x| x.to_string())?;
    Ok(b)
}

impl App {
    pub(crate) fn on_sdo_done(&mut self, tag: u64, o: SdoOutcome) {
        let Some(p) = self.od.pending.remove(&tag) else { return };
        let what = format!("node {} 0x{:04X}:{:02X}", o.node, o.index, o.sub);
        match p {
            Pending::Row { index, sub, write } => match o.result {
                Ok(d) => {
                    let v = write.clone().unwrap_or(d);
                    self.od.values.insert((index, sub), RowValue::Value(v));
                    let verb = if write.is_some() { "Written" } else { "Read" };
                    self.set_status(format!("{verb} {what}"), false);
                }
                Err(e) => {
                    let verb = if write.is_some() { "Write" } else { "Read" };
                    self.od.values.insert((index, sub), RowValue::Error(e.to_string()));
                    self.set_status(format!("{verb} {what}: {e}"), true);
                }
            },
            Pending::Manual { write } => match o.result {
                Ok(d) => {
                    self.od.m_error = false;
                    if write {
                        self.od.m_result = format!("Written {what}");
                        self.od.m_data = None;
                    } else {
                        self.od.m_result = format!("{what}: {} byte(s)", d.len());
                        self.od.m_data = Some(d);
                    }
                    let r = self.od.m_result.clone();
                    self.set_status(r, false);
                }
                Err(e) => {
                    self.od.m_error = true;
                    self.od.m_data = None;
                    self.od.m_result = format!("{what}: {e}");
                    let r = self.od.m_result.clone();
                    self.set_status(r, true);
                }
            },
            Pending::ToFile(path) => {
                let r = o.result.map_err(|e| e.to_string()).and_then(|d| crate::library::write_atomic(&path, &d).map(|_| d.len()).map_err(|e| e.0));
                match r {
                    Ok(n) => {
                        self.od.m_error = false;
                        self.od.m_result = format!("{what}: {n} byte(s) saved to {}", path.display());
                    }
                    Err(e) => {
                        self.od.m_error = true;
                        self.od.m_result = format!("{what} to file: {e}");
                    }
                }
                let r = self.od.m_result.clone();
                let err = self.od.m_error;
                self.set_status(r, err);
            }
            Pending::FromFile(n) => {
                match o.result {
                    Ok(_) => {
                        self.od.m_error = false;
                        self.od.m_result = format!("Written {what}: {n} byte(s) from file");
                    }
                    Err(e) => {
                        self.od.m_error = true;
                        self.od.m_result = format!("{what} from file: {e}");
                    }
                }
                let r = self.od.m_result.clone();
                let err = self.od.m_error;
                self.set_status(r, err);
            }
        }
    }

    /// An SDO write, after the question of the Safety settings.
    fn write_sdo(&mut self, node: u8, index: u16, sub: u8, data: Vec<u8>, pending: Pending) {
        let text = format!("Write {} byte(s) to node {node} object 0x{index:04X}:{sub:02X}?", data.len());
        let ask = self.settings.confirm_before_write;
        self.confirm_then(ask, "Write object", text, "Write", false, move |app| {
            if let Pending::Row { index, sub, .. } = &pending {
                app.od.values.insert((*index, *sub), RowValue::Busy);
            }
            app.send_sdo(node, SdoRequest::Download { index, sub, data }, pending);
        });
    }

    fn read_entry(&mut self, node: u8, index: u16, sub: u8) {
        self.od.values.insert((index, sub), RowValue::Busy);
        self.send_sdo(node, SdoRequest::Upload { index, sub }, Pending::Row { index, sub, write: None });
    }

    pub(crate) fn od_tab(&mut self, ui: &mut egui::Ui) {
        let node = self.active_node();
        let can_send = self.can_send();
        let p = theme::pal(ui);
        let od = match self.active_od() {
            Some(Ok(od)) => Some(od),
            Some(Err(e)) => {
                ui.colored_label(p.error, e);
                None
            }
            None => None,
        };
        let dev = self.active_device().map(|d| d.name.clone());

        // Toolbar.
        ui.horizontal_wrapped(|ui| {
            match &dev {
                Some(n) => ui.strong(format!("{n}, node {node}")),
                None => ui.strong(format!("No active device, node {node}")),
            };
            ui.separator();
            ui.label("Filter");
            ui.add(egui::TextEdit::singleline(&mut self.od.filter).desired_width(140.0).hint_text("index or name"));
            ui.separator();
            ui.label("Values");
            let before = self.settings.value_display;
            for m in DisplayMode::ALL {
                ui.selectable_value(&mut self.settings.value_display, m, m.label());
            }
            if before != self.settings.value_display {
                self.mark_settings();
            }
            ui.separator();
            let sel = self.od.selected;
            let label = match sel {
                Some(i) => format!("Read all 0x{i:04X}"),
                None => "Read all (select an object)".to_string(),
            };
            if theme::icon_button(ui, can_send && sel.is_some() && od.is_some(), icon::DOWNLOAD_SIMPLE, &label, &label).clicked() {
                if let (Some(i), Some(od)) = (sel, &od) {
                    self.read_all(od, node, i);
                }
            }
            if theme::icon_button(ui, od.is_some(), icon::CARET_DOUBLE_DOWN, "Expand all", "Expand all objects").clicked() {
                if let Some(od) = &od {
                    self.od.expanded = od.objects.keys().copied().collect();
                }
            }
            if theme::icon_button(ui, od.is_some(), icon::CARET_DOUBLE_UP, "Collapse all", "Collapse all objects").clicked() {
                self.od.expanded.clear();
            }
            if !can_send {
                ui.colored_label(p.warning, if self.listen_only_active { "listen only: SDO disabled" } else { "not connected" });
            }
        });

        self.manual_box(ui, node, can_send, od.as_ref());
        self.sdo_progress_line(ui);
        ui.separator();

        let Some(od) = od else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::LIST_MAGNIFYING_GLASS).size(32.0).color(p.text_secondary));
                ui.label("No object dictionary. Select a device with an EDS file in the Devices view, or add an EDS file in the EDS library.");
                ui.label(egui::RichText::new("The manual SDO box above works without an EDS.").color(p.text_secondary));
            });
            return;
        };
        self.od_table(ui, &od, node, can_send);
    }

    fn read_all(&mut self, od: &Arc<ObjectDictionary>, node: u8, index: u16) {
        let Some(o) = od.objects.get(&index) else { return };
        let subs: Vec<u8> = o.subs.values().filter(|e| e.access.readable()).map(|e| e.sub).collect();
        for s in &subs {
            self.read_entry(node, index, *s);
        }
        self.set_status(format!("Reading {} sub-object(s) of 0x{index:04X}", subs.len()), false);
    }

    fn manual_box(&mut self, ui: &mut egui::Ui, node: u8, can_send: bool, od: Option<&Arc<ObjectDictionary>>) {
        egui::CollapsingHeader::new("Manual SDO").default_open(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Node {node}"));
                ui.label("Index 0x");
                ui.add(egui::TextEdit::singleline(&mut self.od.m_index).desired_width(50.0));
                ui.label("Sub");
                ui.add(egui::TextEdit::singleline(&mut self.od.m_sub).desired_width(36.0));
                ui.label("Type");
                let cur = self.od.m_type.map(|t| t.name().to_string()).unwrap_or_else(|| "raw bytes".to_string());
                egui::ComboBox::from_id_salt("m_type").selected_text(cur).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.od.m_type, None, "raw bytes");
                    for t in DataType::ALL {
                        ui.selectable_value(&mut self.od.m_type, Some(t), t.name());
                    }
                });
                if let (Some(od), Some(i), Some(s)) = (od, parse_index(&self.od.m_index), parse_sub(&self.od.m_sub)) {
                    if let Some(e) = od.entry(i, s) {
                        if ui.small_button("Type from EDS").on_hover_text(e.name.clone()).clicked() {
                            self.od.m_type = e.dtype();
                        }
                    }
                }
                ui.label("Value");
                ui.add(egui::TextEdit::singleline(&mut self.od.m_value).desired_width(140.0).hint_text("dec, 0x hex, text"));
                let ok = parse_index(&self.od.m_index).zip(parse_sub(&self.od.m_sub));
                if ui.add_enabled(can_send && ok.is_some(), egui::Button::new("Read")).clicked() {
                    if let Some((index, sub)) = ok {
                        self.od.m_result = "reading...".to_string();
                        self.od.m_error = false;
                        self.send_sdo(node, SdoRequest::Upload { index, sub }, Pending::Manual { write: false });
                    }
                }
                if ui.add_enabled(can_send && ok.is_some(), egui::Button::new("Write")).clicked() {
                    if let Some((index, sub)) = ok {
                        let dt = self.od.m_type.unwrap_or(DataType::OctetString);
                        match parse_text(dt, &self.od.m_value, false) {
                            Ok(data) => {
                                self.od.m_result = "writing...".to_string();
                                self.od.m_error = false;
                                self.write_sdo(node, index, sub, data, Pending::Manual { write: true });
                            }
                            Err(e) => {
                                self.od.m_result = format!("Not sent: {e}");
                                self.od.m_error = true;
                            }
                        }
                    }
                }
                ui.separator();
                ui.label("Transfer");
                let before = self.settings.sdo_transfer_mode;
                egui::ComboBox::from_id_salt("od_mode").width(100.0).selected_text(self.settings.sdo_transfer_mode.label()).show_ui(ui, |ui| {
                    for m in TransferMode::ALL {
                        ui.selectable_value(&mut self.settings.sdo_transfer_mode, m, m.label());
                    }
                });
                if before != self.settings.sdo_transfer_mode {
                    self.mark_settings();
                }
                if ui.add_enabled(can_send && ok.is_some(), egui::Button::new("Upload to file...")).on_hover_text("Read the object (any size) into a file").clicked() {
                    if let Some((index, sub)) = ok {
                        if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{index:04X}_{sub:02X}.bin")).save_file() {
                            self.od.m_result = "reading to file...".to_string();
                            self.od.m_error = false;
                            self.send_sdo(node, SdoRequest::Upload { index, sub }, Pending::ToFile(path));
                        }
                    }
                }
                if ui.add_enabled(can_send && ok.is_some(), egui::Button::new("Download from file...")).on_hover_text("Write the bytes of a file into the object").clicked() {
                    if let Some((index, sub)) = ok {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            match std::fs::read(&path) {
                                Ok(data) => {
                                    let n = data.len();
                                    self.od.m_result = "writing from file...".to_string();
                                    self.od.m_error = false;
                                    self.write_sdo(node, index, sub, data, Pending::FromFile(n));
                                }
                                Err(e) => {
                                    self.od.m_result = format!("{}: {e}", path.display());
                                    self.od.m_error = true;
                                }
                            }
                        }
                    }
                }
                if ok.is_none() {
                    ui.colored_label(theme::pal(ui).warning, "index: hex 0..FFFF, sub: 0..255");
                }
            });
            if !self.od.m_result.is_empty() {
                let c = if self.od.m_error { theme::pal(ui).error } else { theme::pal(ui).text_primary };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(c, &self.od.m_result);
                    if let Some(d) = &self.od.m_data {
                        let v = format_value(self.od.m_type, d, self.settings.value_display);
                        ui.label(egui::RichText::new(v).monospace());
                    }
                });
            }
        });
    }

    /// Progress of a running SDO transfer, with Cancel.
    fn sdo_progress_line(&mut self, ui: &mut egui::Ui) {
        let prog = self.bus.shared.lock().ok().and_then(|s| s.sdo_progress);
        let Some((n, i, s, pr)) = prog else { return };
        ui.horizontal(|ui| {
            let text = match pr.total {
                Some(t) if t > 0 => format!("SDO node {n} 0x{i:04X}:{s:02X}: {} / {t} bytes", pr.done),
                _ => format!("SDO node {n} 0x{i:04X}:{s:02X}: {} bytes", pr.done),
            };
            let frac = pr.total.filter(|t| *t > 0).map(|t| pr.done as f32 / t as f32).unwrap_or(0.0);
            ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(200.0).text(text));
            if ui.button("Cancel transfer").clicked() {
                self.bus.send(BusCommand::CancelSdo);
            }
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    fn od_table(&mut self, ui: &mut egui::Ui, od: &Arc<ObjectDictionary>, node: u8, can_send: bool) {
        let f = self.od.filter.trim().to_lowercase();
        let mut rows: Vec<RowRef> = Vec::new();
        for (&index, o) in &od.objects {
            if !matches(od, index, &f) {
                continue;
            }
            if o.is_var() {
                if let Some(e) = o.subs.values().next() {
                    rows.push(RowRef::Entry(index, e.sub));
                }
            } else {
                rows.push(RowRef::Object(index));
                if self.od.expanded.contains(&index) || !f.is_empty() {
                    for &s in o.subs.keys() {
                        rows.push(RowRef::Entry(index, s));
                    }
                }
            }
        }
        let mode = self.settings.value_display;
        let row_h = self.settings.row_density.row_height().max(ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Body)) + 2.0);
        let zebra = self.settings.zebra;
        let err_c = theme::pal(ui).error;
        let mut reads: Vec<(u16, u8)> = Vec::new();
        let mut writes: Vec<(u16, u8)> = Vec::new();
        let mut read_all: Option<u16> = None;
        let mut toggle: Option<u16> = None;
        let mut select: Option<u16> = None;

        let dis_c = theme::pal(ui).text_disabled;
        // Column widths from the width of the view (spec 8.2): the actions
        // column has a fixed width and is always inside the view; Name
        // takes the rest.
        const ACTIONS: f32 = 64.0;
        let gap = ui.spacing().item_spacing.x;
        let fixed = 76.0 + 30.0 + 48.0 + ACTIONS + gap * 9.0;
        let rest = (ui.available_width() - fixed).max(420.0);
        let w = |f: f32, min: f32| (rest * f).max(min);
        TableBuilder::new(ui)
            .id_salt("od_table")
            .striped(zebra)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            // The user drags the column borders; the widths above are only
            // the start values. Name takes what is left; the actions stay fixed.
            .resizable(true)
            .column(Column::initial(76.0).at_least(60.0).clip(true))
            .column(Column::initial(30.0).at_least(26.0).clip(true))
            .column(Column::remainder().at_least(80.0).clip(true))
            .column(Column::initial(w(0.13, 56.0)).at_least(40.0).clip(true))
            .column(Column::initial(48.0).at_least(36.0).clip(true))
            .column(Column::initial(w(0.22, 70.0)).at_least(40.0).clip(true))
            .column(Column::initial(w(0.13, 50.0)).at_least(40.0).clip(true))
            .column(Column::initial(w(0.18, 64.0)).at_least(50.0).clip(true))
            .column(Column::exact(ACTIONS).resizable(false))
            .min_scrolled_height(0.0)
            .header(row_h, |mut h| {
                for t in ["Index", "Sub", "Name", "Type", "Access", "Value", "Default", "New value", ""] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, rows.len(), |mut row| {
                    let r = rows[row.index()];
                    match r {
                        RowRef::Object(index) => {
                            let o = &od.objects[&index];
                            let open = self.od.expanded.contains(&index) || !f.is_empty();
                            let sel = self.od.selected == Some(index);
                            row.set_selected(sel);
                            row.col(|ui| {
                                let (arrow, name) = if open { (icon::CARET_DOWN, "Collapse") } else { (icon::CARET_RIGHT, "Expand") };
                                let r = ui.add(egui::Button::new(arrow).frame(false).small());
                                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
                                if r.clicked() {
                                    toggle = Some(index);
                                }
                                if ui.selectable_label(sel, format!("{index:04X}")).clicked() {
                                    select = Some(index);
                                }
                            });
                            row.col(|ui| {
                                ui.label(format!("{}", o.subs.len()));
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(egui::RichText::new(&o.name).strong()).truncate());
                            });
                            row.col(|ui| {
                                ui.label(o.object_type.label());
                            });
                            row.col(|_| {});
                            row.col(|_| {});
                            row.col(|_| {});
                            row.col(|_| {});
                            row.col(|ui| {
                                if theme::icon_button(ui, can_send, icon::DOWNLOAD_SIMPLE, "Read all", "Read all sub-objects from the device").clicked() {
                                    read_all = Some(index);
                                }
                            });
                        }
                        RowRef::Entry(index, sub) => {
                            let o = &od.objects[&index];
                            let e = &o.subs[&sub];
                            let is_var = o.is_var();
                            row.set_selected(self.od.selected == Some(index));
                            row.col(|ui| {
                                if is_var {
                                    if ui.selectable_label(self.od.selected == Some(index), format!("{index:04X}")).clicked() {
                                        select = Some(index);
                                    }
                                } else {
                                    ui.weak(format!("{index:04X}"));
                                }
                            });
                            row.col(|ui| {
                                ui.label(format!("{sub:02X}"));
                            });
                            row.col(|ui| {
                                let name = if is_var { o.name.as_str() } else { e.name.as_str() };
                                ui.add(egui::Label::new(name).truncate()).on_hover_text(name);
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(type_name(e.data_type)).truncate());
                            });
                            row.col(|ui| {
                                ui.label(e.access.label());
                            });
                            row.col(|ui| match self.od.values.get(&(index, sub)) {
                                Some(RowValue::Value(b)) => {
                                    let t = format_value(e.dtype(), b, mode);
                                    ui.add(egui::Label::new(egui::RichText::new(&t).monospace()).truncate()).on_hover_text(t);
                                }
                                Some(RowValue::Error(t)) => {
                                    ui.add(egui::Label::new(egui::RichText::new(t).color(err_c)).truncate()).on_hover_text(t);
                                }
                                Some(RowValue::Busy) => {
                                    ui.weak("...");
                                }
                                None => {
                                    ui.label(egui::RichText::new("\u{2014}").color(dis_c)).on_hover_text("not read yet");
                                }
                            });
                            row.col(|ui| {
                                let d = e.effective_text().unwrap_or("");
                                ui.add(egui::Label::new(d).truncate()).on_hover_text(d);
                            });
                            row.col(|ui| {
                                if e.access.writable() {
                                    let buf = self.od.edits.entry((index, sub)).or_default();
                                    ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY).hint_text("dec / 0x hex"));
                                }
                            });
                            row.col(|ui| {
                                if theme::icon_button(ui, can_send && e.access.readable(), icon::DOWNLOAD_SIMPLE, "Read", "Read from the device").clicked() {
                                    reads.push((index, sub));
                                }
                                if theme::icon_button(ui, can_send && e.access.writable(), icon::UPLOAD_SIMPLE, "Write", "Write the new value to the device").clicked() {
                                    writes.push((index, sub));
                                }
                            });
                        }
                    }
                });
            });

        if let Some(i) = toggle {
            if !self.od.expanded.remove(&i) {
                self.od.expanded.insert(i);
            }
        }
        if let Some(i) = select {
            self.od.selected = if self.od.selected == Some(i) { None } else { Some(i) };
        }
        if let Some(i) = read_all {
            self.od.selected = Some(i);
            self.read_all(od, node, i);
        }
        for (i, s) in reads {
            self.read_entry(node, i, s);
        }
        for (i, s) in writes {
            let Some(e) = od.entry(i, s) else { continue };
            let text = self.od.edits.get(&(i, s)).cloned().unwrap_or_default();
            match entry_bytes(e, &text, node) {
                Ok(data) => {
                    self.write_sdo(node, i, s, data.clone(), Pending::Row { index: i, sub: s, write: Some(data) });
                }
                Err(err) => {
                    self.od.values.insert((i, s), RowValue::Error(format!("not sent: {err}")));
                    self.set_status(format!("0x{i:04X}:{s:02X} not written: {err}"), true);
                }
            }
        }
    }
}
