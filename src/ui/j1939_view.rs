//! J1939 view (read only): the parameter groups by PGN and source address,
//! the source addresses with the NAME of their address claim, and the
//! DM1 faults. Fed by the bus worker (`Shared::j1939`) with the received
//! frames while Settings > Extended IDs are J1939. Nothing is sent.

use super::theme;
use super::App;
use crate::j1939::{self, AddrRow, FaultRow, PgRow, TpCounters, MAX_FAULT_ROWS, MAX_PG_ROWS};
use eframe::egui;
use egui_extras::{Column, TableBuilder};

/// Bytes of the "Last data" column before the ellipsis.
const DATA_CUT: usize = 8;

#[derive(Default)]
pub(crate) struct J1939View {
    /// The selected parameter group row: (PGN, SA).
    pub selected: Option<(u32, u8)>,
}

/// The data bytes as hex, cut after `DATA_CUT` bytes with an ellipsis.
pub(crate) fn data_cut(d: &[u8]) -> String {
    let t = crate::canopen::types::hex_bytes(&d[..d.len().min(DATA_CUT)]);
    if d.len() > DATA_CUT {
        format!("{t} \u{2026}")
    } else {
        t
    }
}

/// A 16-byte-per-line hex dump with the offset.
pub(crate) fn hex_dump(d: &[u8]) -> String {
    d.chunks(16).enumerate().map(|(i, c)| format!("{:04X}  {}", i * 16, crate::canopen::types::hex_bytes(c))).collect::<Vec<_>>().join("\n")
}

fn age_text(now: u64, last: u64) -> String {
    format!("{:.1} s", now.saturating_sub(last) as f64 / 1e6)
}

fn lamps_text(l: u8) -> String {
    let v = j1939::lamps_on(l);
    if v.is_empty() {
        "-".to_string()
    } else {
        v.join(", ")
    }
}

impl App {
    pub(crate) fn j1939_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let (pgs, addrs, faults, tp, pg_dropped, fault_dropped, now, empty) = {
            let s = self.bus.shared.lock().unwrap_or_else(|p| p.into_inner());
            let m = &s.j1939;
            let pgs: Vec<PgRow> = m.pgs().cloned().collect();
            let addrs: Vec<AddrRow> = m.addrs().cloned().collect();
            let faults: Vec<FaultRow> = m.faults().cloned().collect();
            (pgs, addrs, faults, m.counters(), m.pg_dropped, m.fault_dropped, s.now_us, m.is_empty())
        };
        let on = self.decode_ctx.j1939();
        egui::ScrollArea::vertical().id_salt("j1939_scroll").show(ui, |ui| {
            self.j1939_toolbar(ui, &tp, !empty, pgs.len());
            if !on {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(egui_phosphor::regular::TRUCK).size(28.0).color(p.text_secondary));
                    ui.label("J1939 is off.");
                    ui.label(egui::RichText::new("Settings > Extended IDs are: Raw frames. Choose J1939 to decode the 29-bit frames here.").color(p.text_secondary));
                });
                return;
            }
            if empty {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(egui_phosphor::regular::TRUCK).size(28.0).color(p.text_secondary));
                    ui.label("No J1939 frame received yet.");
                    ui.label(egui::RichText::new("Each received 29-bit frame is listed here by its parameter group (PGN) and source address. Nothing is sent.").color(p.text_secondary));
                });
                return;
            }
            ui.strong(format!("Parameter groups ({})", pgs.len()));
            if pg_dropped > 0 {
                ui.label(egui::RichText::new(format!("{pg_dropped} frame(s) of new groups not listed (at most {MAX_PG_ROWS} rows)")).color(p.warning));
            }
            self.j1939_pg_table(ui, &pgs, now);
            if let Some(r) = self.j1v.selected.and_then(|k| pgs.iter().find(|r| (r.pgn, r.sa) == k)) {
                j1939_detail(ui, r);
            }
            ui.separator();
            ui.strong(format!("Addresses ({})", addrs.len()));
            j1939_addr_table(ui, &addrs, now, self.settings.zebra, self.settings.row_density.row_height());
            ui.separator();
            let active = faults.iter().filter(|f| f.active).count();
            ui.strong(format!("Active faults ({active} active, {} listed)", faults.len()));
            if fault_dropped > 0 {
                ui.label(egui::RichText::new(format!("{fault_dropped} fault(s) not listed (at most {MAX_FAULT_ROWS} rows)")).color(p.warning));
            }
            if faults.is_empty() {
                ui.label(egui::RichText::new("No DM1 fault received.").color(p.text_secondary));
            } else {
                j1939_fault_table(ui, &faults, now, self.settings.zebra, self.settings.row_density.row_height());
            }
        });
    }

    fn j1939_toolbar(&mut self, ui: &mut egui::Ui, tp: &TpCounters, any: bool, rows: usize) {
        let p = theme::pal(ui);
        theme::toolbar(ui, |ui| {
            if ui.add_enabled(any, egui::Button::new(format!("{} Clear", egui_phosphor::regular::TRASH))).on_hover_text("Clear the tables and the transport counters (nothing is sent)").clicked() {
                self.bus.shared.lock().unwrap_or_else(|p| p.into_inner()).j1939.clear();
                self.j1v.selected = None;
            }
            ui.label(egui::RichText::new(tp.text()).color(p.text_secondary)).on_hover_text(
                "Transport protocol (TP.CM / TP.DT, BAM and RTS/CTS): messages reassembled, sessions that ended before all data (replaced by a new BAM or RTS, or an EOM too early), and errors (timeouts of 750 ms BAM / 1250 ms RTS/CTS, sequence numbers out of order, aborts)",
            );
            if ui.add_enabled(rows > 0, egui::Button::new("Export CSV...")).on_hover_text("Save the parameter group table as CSV").clicked() {
                if let Some(path) = rfd::FileDialog::new().set_file_name("j1939.csv").add_filter("CSV", &["csv"]).save_file() {
                    let csv = self.bus.shared.lock().unwrap_or_else(|p| p.into_inner()).j1939.csv();
                    match crate::library::write_atomic(&path, csv.as_bytes()) {
                        Ok(()) => self.set_status(format!("Saved {rows} J1939 row(s) to {}", path.display()), false),
                        Err(e) => self.set_status(format!("CSV not written: {}", e.0), true),
                    }
                }
            }
        });
    }

    fn j1939_pg_table(&mut self, ui: &mut egui::Ui, pgs: &[PgRow], now: u64) {
        let row_h = self.settings.row_density.row_height();
        let mut clicked: Option<(u32, u8)> = None;
        let sel = self.j1v.selected;
        egui::ScrollArea::horizontal().id_salt("j1939_pg_h").show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt("j1939_pg_table")
                .striped(self.settings.zebra)
                .sense(egui::Sense::click())
                .vscroll(true)
                .max_scroll_height(320.0)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(56.0))
                .column(Column::initial(70.0).clip(true))
                .column(Column::exact(40.0))
                .column(Column::exact(44.0))
                .column(Column::exact(32.0))
                .column(Column::exact(48.0))
                .column(Column::exact(64.0))
                .column(Column::exact(64.0))
                .column(Column::initial(220.0).at_least(120.0).clip(true))
                .column(Column::exact(60.0))
                .header(row_h, |mut h| {
                    for t in ["PGN", "Name", "SA", "Dest", "Prio", "Length", "Count", "Cycle ms", "Last data", "Age"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, pgs.len(), |mut row| {
                        let r = &pgs[row.index()];
                        row.set_selected(sel == Some((r.pgn, r.sa)));
                        row.col(|ui| {
                            ui.monospace(r.pgn.to_string());
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(j1939::pgn_name(r.pgn).unwrap_or("")).truncate());
                        });
                        row.col(|ui| {
                            ui.monospace(r.sa.to_string());
                        });
                        row.col(|ui| {
                            ui.monospace(j1939::dest_text(r.da));
                        });
                        row.col(|ui| {
                            ui.monospace(r.priority.to_string());
                        });
                        row.col(|ui| {
                            ui.monospace(r.data.len().to_string());
                        });
                        row.col(|ui| {
                            ui.monospace(r.count.to_string());
                        });
                        row.col(|ui| {
                            ui.monospace(r.period_us.map(|p| format!("{:.1}", p as f64 / 1000.0)).unwrap_or_default());
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(data_cut(&r.data)).monospace()).truncate());
                        });
                        row.col(|ui| {
                            ui.monospace(age_text(now, r.last_us));
                        });
                        if row.response().clicked() {
                            clicked = Some((r.pgn, r.sa));
                        }
                    });
                });
        });
        if let Some(k) = clicked {
            self.j1v.selected = if self.j1v.selected == Some(k) { None } else { Some(k) };
        }
    }
}

/// The detail box of the selected parameter group: a hex dump of the last
/// data and, for DM1 / DM2, the fault list.
fn j1939_detail(ui: &mut egui::Ui, r: &PgRow) {
    let p = theme::pal(ui);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        let multi = if r.multi { ", reassembled" } else { "" };
        ui.strong(format!("{}  SA {}  -> {}  ({} bytes{multi})", j1939::pgn_text(r.pgn), r.sa, j1939::dest_text(r.da), r.data.len()));
        ui.label(egui::RichText::new(hex_dump(&r.data)).monospace());
        let label = match r.pgn {
            j1939::PGN_DM1 => Some("DM1"),
            j1939::PGN_DM2 => Some("DM2"),
            _ => None,
        };
        if let Some(l) = label {
            match j1939::decode_dm(&r.data) {
                Some(m) => {
                    ui.label(format!("Lamps: {}", lamps_text(m.lamps)));
                    if m.faults.is_empty() {
                        ui.label(format!("{l}: no active fault"));
                    }
                    for f in &m.faults {
                        ui.monospace(format!("SPN {}  FMI {}  OC {}  CM {}", f.spn, f.fmi, f.oc, f.cm as u8));
                    }
                }
                None => {
                    ui.colored_label(p.warning, format!("{l}: too short"));
                }
            }
        }
    });
}

fn j1939_addr_table(ui: &mut egui::Ui, addrs: &[AddrRow], now: u64, zebra: bool, row_h: f32) {
    egui::ScrollArea::horizontal().id_salt("j1939_addr_h").show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("j1939_addr_table")
            .striped(zebra)
            .vscroll(true)
            .max_scroll_height(200.0)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(40.0))
            .column(Column::initial(280.0).at_least(160.0).clip(true))
            .column(Column::exact(72.0))
            .column(Column::exact(60.0))
            .header(row_h, |mut h| {
                for t in ["SA", "Address claim", "Frames", "Age"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, addrs.len(), |mut row| {
                    let a = &addrs[row.index()];
                    row.col(|ui| {
                        ui.monospace(a.sa.to_string());
                    });
                    row.col(|ui| {
                        let t = match &a.name {
                            Some(n) => n.short_text(),
                            None => "-".to_string(),
                        };
                        let r = ui.add(egui::Label::new(t).truncate());
                        if let Some(n) = &a.name {
                            r.on_hover_text(format!(
                                "NAME {:016X}\nidentity {}, manufacturer {}, ECU instance {}, function instance {}, function {}, vehicle system {} (instance {}), industry group {}, arbitrary address {}",
                                n.raw, n.identity, n.manufacturer, n.ecu_instance, n.function_instance, n.function, n.vehicle_system, n.vehicle_system_instance, n.industry_group, n.arbitrary_address
                            ));
                        }
                    });
                    row.col(|ui| {
                        ui.monospace(a.frames.to_string());
                    });
                    row.col(|ui| {
                        ui.monospace(age_text(now, a.last_us));
                    });
                });
            });
    });
}

fn j1939_fault_table(ui: &mut egui::Ui, faults: &[FaultRow], now: u64, zebra: bool, row_h: f32) {
    let p = theme::pal(ui);
    egui::ScrollArea::horizontal().id_salt("j1939_fault_h").show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("j1939_fault_table")
            .striped(zebra)
            .vscroll(true)
            .max_scroll_height(200.0)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(40.0))
            .column(Column::exact(72.0))
            .column(Column::exact(40.0))
            .column(Column::exact(40.0))
            .column(Column::initial(140.0).clip(true))
            .column(Column::exact(72.0))
            .column(Column::exact(60.0))
            .header(row_h, |mut h| {
                for t in ["SA", "SPN", "FMI", "OC", "Lamps", "State", "Age"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, faults.len(), |mut row| {
                    let f = &faults[row.index()];
                    let c = if f.active { p.text_primary } else { p.text_secondary };
                    let cell = |ui: &mut egui::Ui, t: String| {
                        ui.add(egui::Label::new(egui::RichText::new(t).monospace().color(c)).truncate());
                    };
                    row.col(|ui| cell(ui, f.sa.to_string()));
                    row.col(|ui| cell(ui, f.spn.to_string()));
                    row.col(|ui| cell(ui, f.fmi.to_string()));
                    row.col(|ui| cell(ui, f.oc.to_string()));
                    row.col(|ui| cell(ui, lamps_text(f.lamps)));
                    row.col(|ui| cell(ui, if f.active { "active".to_string() } else { "cleared".to_string() }));
                    row.col(|ui| cell(ui, age_text(now, f.last_us)));
                });
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_cut_and_dump() {
        assert_eq!(data_cut(&[1, 2, 3]), "01 02 03");
        let d: Vec<u8> = (0..20).collect();
        assert_eq!(data_cut(&d), "00 01 02 03 04 05 06 07 \u{2026}");
        let h = hex_dump(&d);
        assert_eq!(h.lines().count(), 2);
        assert!(h.starts_with("0000  00 01") && h.contains("\n0010  10 11 12 13"), "{h}");
    }
}
