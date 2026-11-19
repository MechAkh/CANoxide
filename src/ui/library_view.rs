//! Devices and EDS library views, and their dialogs (add / edit / remove).

use super::{theme, App, Dialog};
use crate::can::Bitrate;
use crate::library::Device;
use eframe::egui;

impl App {
    pub(crate) fn devices_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let have_lib = self.lib.is_some();
        let names: Vec<(String, u8, Option<String>)> = match &self.lib {
            Some(l) => l.devices.iter().map(|d| (d.name.clone(), d.node_id, d.eds_file.clone())).collect(),
            None => Vec::new(),
        };
        let active = self.active_idx();
        ui.horizontal(|ui| {
            if ui.add_enabled(have_lib, egui::Button::new("Add")).clicked() {
                let dev = Device { name: String::new(), node_id: self.active_node(), eds_file: self.selected_eds.clone(), bitrate_kbit: self.settings.bitrate_kbit, notes: String::new() };
                self.dialog = Some(Dialog::Device { idx: None, dev, error: None });
            }
            if ui.add_enabled(active.is_some(), egui::Button::new("Edit")).clicked() {
                if let (Some(i), Some(d)) = (active, self.active_device().cloned()) {
                    self.dialog = Some(Dialog::Device { idx: Some(i), dev: d, error: None });
                }
            }
            if ui.add_enabled(active.is_some(), egui::Button::new("Remove")).clicked() {
                if let Some(i) = active {
                    self.dialog = Some(Dialog::RemoveDevice(i));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Node ID");
            match active {
                Some(i) => {
                    let mut n = names[i].1;
                    if ui.add(egui::DragValue::new(&mut n).range(1..=127)).changed() {
                        if let Some(lib) = self.lib.as_mut() {
                            let mut d = lib.devices[i].clone();
                            d.node_id = n;
                            if let Err(e) = lib.update_device(i, d) {
                                self.set_status(format!("Device not saved: {e}"), true);
                            }
                        }
                        self.od.reset_values();
                        self.rebuild_decode();
                    }
                }
                None => {
                    if ui.add(egui::DragValue::new(&mut self.free_node).range(1..=127)).on_hover_text("Node for SDO and NMT when no device is active").changed() {
                        self.od.reset_values();
                    }
                }
            }
        });
        ui.separator();
        if names.is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::CPU).size(28.0).color(p.text_secondary));
                ui.label("No devices yet.");
                ui.label(egui::RichText::new("A device is a name, a node ID and an EDS file. Add one with Add.").color(p.text_secondary));
            });
            return;
        }
        egui::ScrollArea::vertical().id_salt("devices").show(ui, |ui| {
            for (i, (name, node, eds)) in names.iter().enumerate() {
                let text = format!("{name}  (node {node}){}", if eds.is_none() { ", no EDS" } else { "" });
                if ui.selectable_label(active == Some(i), text).on_hover_text(eds.clone().unwrap_or_default()).clicked() {
                    self.settings.active_device = if active == Some(i) { None } else { Some(name.clone()) };
                    self.mark_settings();
                    self.od.reset_values();
                }
            }
        });
    }

    pub(crate) fn eds_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let files = self.lib.as_ref().map(|l| l.list_eds()).unwrap_or_default();
        ui.horizontal(|ui| {
            if ui.add_enabled(self.lib.is_some(), egui::Button::new("Add...")).clicked() {
                self.open_eds_dialog();
            }
            if ui.add_enabled(self.selected_eds.is_some(), egui::Button::new("Remove")).clicked() {
                if let Some(f) = self.selected_eds.clone() {
                    self.dialog = Some(Dialog::RemoveEds(f));
                }
            }
        });
        ui.separator();
        if files.is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::BOOKS).size(28.0).color(p.text_secondary));
                ui.label("No EDS files.");
                ui.label(egui::RichText::new("Add one, or drop .eds / .dcf files onto the window.").color(p.text_secondary));
            });
            return;
        }
        egui::ScrollArea::vertical().id_salt("eds_list").show(ui, |ui| {
            for f in &files {
                let sel = self.selected_eds.as_deref() == Some(f.as_str());
                if ui.selectable_label(sel, f).clicked() {
                    self.selected_eds = if sel { None } else { Some(f.clone()) };
                }
            }
            let Some(sel) = self.selected_eds.clone() else { return };
            ui.separator();
            match self.od_for(&sel) {
                Err(e) => {
                    ui.colored_label(p.error, e);
                }
                Ok(od) => {
                    ui.strong(&sel);
                    egui::Grid::new("eds_info").num_columns(2).striped(true).show(ui, |ui| {
                        for (k, v) in od.file_info.iter().chain(od.device_info.iter()) {
                            if v.is_empty() {
                                continue;
                            }
                            ui.label(egui::RichText::new(k).color(p.text_secondary));
                            ui.add(egui::Label::new(v).wrap());
                            ui.end_row();
                        }
                        ui.label(egui::RichText::new("Objects").color(p.text_secondary));
                        ui.label(format!("{} ({} entries){}", od.objects.len(), od.entry_count(), if od.is_dcf { ", DCF" } else { "" }));
                        ui.end_row();
                        if let Some(n) = od.node_id {
                            ui.label(egui::RichText::new("DCF node ID").color(p.text_secondary));
                            ui.label(n.to_string());
                            ui.end_row();
                        }
                    });
                    if od.warnings.is_empty() {
                        ui.label("No parse warnings.");
                    } else {
                        ui.colored_label(p.warning, format!("{} parse warning(s):", od.warnings.len()));
                        for w in &od.warnings {
                            ui.add(egui::Label::new(egui::RichText::new(w).small()).wrap());
                        }
                    }
                }
            }
        });
    }

    pub(crate) fn library_dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let eds_files = self.lib.as_ref().map(|l| l.list_eds()).unwrap_or_default();
        let err_c = if self.is_dark() { theme::DARK.error } else { theme::LIGHT.error };
        match &mut dialog {
            Dialog::Device { idx, dev, error } => {
                let title = if idx.is_some() { "Edit device" } else { "Add device" };
                let mut save = false;
                egui::Modal::new(egui::Id::new("device_dialog")).show(ctx, |ui| {
                    ui.heading(title);
                    egui::Grid::new("dev_grid").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut dev.name);
                        ui.end_row();
                        ui.label("Node ID");
                        ui.add(egui::DragValue::new(&mut dev.node_id).range(1..=127));
                        ui.end_row();
                        ui.label("EDS file");
                        egui::ComboBox::from_id_salt("dev_eds").selected_text(dev.eds_file.clone().unwrap_or_else(|| "(none)".to_string())).show_ui(ui, |ui| {
                            ui.selectable_value(&mut dev.eds_file, None, "(none)");
                            for f in &eds_files {
                                ui.selectable_value(&mut dev.eds_file, Some(f.clone()), f);
                            }
                        });
                        ui.end_row();
                        ui.label("Bitrate");
                        let cur = Bitrate::from_kbit(dev.bitrate_kbit).map(|b| b.label()).unwrap_or_else(|| "(none)".to_string());
                        egui::ComboBox::from_id_salt("dev_br").selected_text(cur).show_ui(ui, |ui| {
                            for b in Bitrate::ALL {
                                ui.selectable_value(&mut dev.bitrate_kbit, b.kbit(), b.label());
                            }
                        });
                        ui.end_row();
                        ui.label("Notes");
                        ui.add(egui::TextEdit::multiline(&mut dev.notes).desired_rows(3));
                        ui.end_row();
                    });
                    if let Some(e) = error {
                        ui.colored_label(err_c, e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            save = true;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if save {
                    if let Some(lib) = self.lib.as_mut() {
                        dev.name = dev.name.trim().to_string();
                        let r = match idx {
                            Some(i) => lib.update_device(*i, dev.clone()),
                            None => lib.add_device(dev.clone()).map(|_| ()),
                        };
                        match r {
                            Ok(()) => {
                                keep = false;
                                self.settings.active_device = Some(dev.name.clone());
                                self.mark_settings();
                                self.od.reset_values();
                                let msg = format!("Device {} saved", dev.name);
                                self.set_status(msg, false);
                            }
                            Err(e) => *error = Some(e.to_string()),
                        }
                    }
                    self.rebuild_decode();
                }
            }
            Dialog::RemoveEds(name) => {
                let mut yes = false;
                egui::Modal::new(egui::Id::new("rm_eds")).show(ctx, |ui| {
                    ui.label(format!("Remove the EDS file {name} from the library? Devices that use it keep no EDS."));
                    ui.horizontal(|ui| {
                        yes = ui.button("Remove").clicked();
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if yes {
                    keep = false;
                    let name = name.clone();
                    if let Some(lib) = self.lib.as_mut() {
                        match lib.remove_eds(&name) {
                            Ok(()) => {
                                self.eds_cache.remove(&name);
                                self.selected_eds = None;
                                self.set_status(format!("Removed {name}"), false);
                            }
                            Err(e) => self.set_status(e.to_string(), true),
                        }
                    }
                    self.rebuild_decode();
                }
            }
            Dialog::RemoveDevice(i) => {
                let i = *i;
                let name = self.lib.as_ref().and_then(|l| l.devices.get(i)).map(|d| d.name.clone()).unwrap_or_default();
                let mut yes = false;
                egui::Modal::new(egui::Id::new("rm_dev")).show(ctx, |ui| {
                    ui.label(format!("Remove the device {name}? The EDS file stays in the library."));
                    ui.horizontal(|ui| {
                        yes = ui.button("Remove").clicked();
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if yes {
                    keep = false;
                    if let Some(lib) = self.lib.as_mut() {
                        match lib.remove_device(i) {
                            Ok(d) => {
                                self.settings.active_device = None;
                                self.mark_settings();
                                self.set_status(format!("Removed device {}", d.name), false);
                            }
                            Err(e) => self.set_status(e.to_string(), true),
                        }
                    }
                    self.od.reset_values();
                    self.rebuild_decode();
                }
            }
        }
        if keep {
            self.dialog = Some(dialog);
        }
    }
}
