//! Signal analysis UI (0.3): the Define signal dialog and the user signal
//! file, the Find similar bits window, the Find signal tab, the history of
//! a COB-ID and the sparklines. The logic lives in `crate::usersig` and
//! `crate::analysis`; this module only draws.

use super::{messages_view, theme, App};
use crate::analysis::{self, BitMatch, BitRef, CmpOp, FindSettings, FindSignal, FindStep, HistFilter, HistRow, SparkSet, Sparkline};
use crate::bus::BusCommand;
use crate::can::CanFrame;
use crate::msgstats::MsgKey;
use crate::signals::{RawKind, SignalDef};
use crate::usersig::{self, fmt_value, UserSignal, COLOURS, MAX_USER_SIGNALS, USER_SIGNALS_FILE};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most history rows built (the newest).
pub(crate) const MAX_HIST_ROWS: usize = 100_000;
/// Most rows of the similar-bits table.
const MAX_SIMILAR_ROWS: usize = 500;
/// Candidates listed one by one in Find signal.
pub(crate) const LIST_CANDIDATES: usize = 50;

/// The fields of the Define signal dialog.
pub(crate) struct SigDraft {
    pub sig: UserSignal,
    pub cob_text: String,
    /// The index of the signal being edited (`None`: a new one).
    pub edit: Option<usize>,
    pub error: Option<String>,
}

/// A DBC file read for the import dialog.
pub(crate) struct DbcPending {
    pub file: String,
    pub import: crate::dbc::DbcImport,
}

impl SigDraft {
    pub(crate) fn new(sig: UserSignal, edit: Option<usize>) -> SigDraft {
        let cob_text = format!("{:X}", sig.cob_id);
        SigDraft { sig, cob_text, edit, error: None }
    }
}

/// Frames for an analysis run: the loaded log up to the replay cursor, or
/// a copy of the trace buffer.
enum FrameSrc {
    Log(Arc<Vec<CanFrame>>, usize),
    Own(Vec<CanFrame>),
}

impl FrameSrc {
    fn frames(&self) -> &[CanFrame] {
        match self {
            FrameSrc::Log(v, n) => &v[..(*n).min(v.len())],
            FrameSrc::Own(v) => v,
        }
    }
}

/// A Find similar bits run (off the UI thread) and its result.
pub(crate) struct SimilarJob {
    pub reference: BitRef,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
    pub total: usize,
    rx: Option<Receiver<Option<Vec<BitMatch>>>>,
    pub result: Option<Vec<BitMatch>>,
    pub cancelled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpKind {
    Eq,
    Ne,
    Gt,
    Lt,
    Between,
    Increased,
    Decreased,
    Changed,
    Unchanged,
}

impl OpKind {
    const ALL: [OpKind; 9] = [OpKind::Eq, OpKind::Ne, OpKind::Gt, OpKind::Lt, OpKind::Between, OpKind::Increased, OpKind::Decreased, OpKind::Changed, OpKind::Unchanged];

    fn label(self) -> &'static str {
        match self {
            OpKind::Eq => "=",
            OpKind::Ne => "!=",
            OpKind::Gt => ">",
            OpKind::Lt => "<",
            OpKind::Between => "between",
            OpKind::Increased => "increased",
            OpKind::Decreased => "decreased",
            OpKind::Changed => "changed",
            OpKind::Unchanged => "unchanged",
        }
    }

    fn values(self) -> usize {
        match self {
            OpKind::Between => 2,
            OpKind::Eq | OpKind::Ne | OpKind::Gt | OpKind::Lt => 1,
            _ => 0,
        }
    }

    fn step(self, a: f64, b: f64) -> FindStep {
        match self {
            OpKind::Eq => FindStep::Eq(a),
            OpKind::Ne => FindStep::Ne(a),
            OpKind::Gt => FindStep::Gt(a),
            OpKind::Lt => FindStep::Lt(a),
            OpKind::Between => FindStep::Between(a, b),
            OpKind::Increased => FindStep::Increased,
            OpKind::Decreased => FindStep::Decreased,
            OpKind::Changed => FindStep::Changed,
            OpKind::Unchanged => FindStep::Unchanged,
        }
    }
}

pub(crate) struct FindView {
    pub fs: FindSignal,
    pub ids_text: String,
    pub op: OpKind,
    pub a: f64,
    pub b: f64,
    pub error: Option<String>,
}

impl Default for FindView {
    fn default() -> Self {
        FindView { fs: FindSignal::new(FindSettings::default()), ids_text: String::new(), op: OpKind::Changed, a: 0.0, b: 0.0, error: None }
    }
}

/// The history of the selected COB-ID (rows cached between frames).
#[derive(Default)]
pub(crate) struct HistView {
    pub filter_on: bool,
    pub col: usize,
    pub op: Option<CmpOp>,
    pub value_text: String,
    rows: Vec<HistRow>,
    sigs: Vec<UserSignal>,
    /// (key, frames seen, filter) the rows were built from, and when.
    built: Option<(MsgKey, u64, Option<HistFilter>, Instant)>,
}

#[derive(Default)]
pub(crate) struct AnalysisState {
    /// The user signals (also in the decode context as an `Arc`).
    pub usigs: Vec<UserSignal>,
    pub draft: Option<SigDraft>,
    /// A read DBC file waiting for Add / Replace.
    pub dbc: Option<DbcPending>,
    pub similar: Option<SimilarJob>,
    pub find: FindView,
    pub hist: HistView,
    pub sparks: SparkSet,
}

/// Draw a sparkline of the last `window` seconds before `now_s`.
pub(crate) fn sparkline(ui: &mut egui::Ui, line: &Sparkline, now_s: f64, colour: egui::Color32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(84.0, 16.0), egui::Sense::hover());
    let p = theme::pal(ui);
    ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0_f32, p.bg_button), egui::StrokeKind::Inside);
    let t0 = now_s - line.window_s();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut n = 0;
    for (t, v) in line.iter() {
        if t >= t0 && v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
            n += 1;
        }
    }
    if n == 0 {
        return resp;
    }
    let span = if hi > lo { hi - lo } else { 1.0 };
    let r = r.shrink(2.0);
    let pts: Vec<egui::Pos2> = line
        .iter()
        .filter(|(t, v)| *t >= t0 && v.is_finite())
        .map(|(t, v)| {
            let x = r.left() + ((t - t0) / line.window_s()).clamp(0.0, 1.0) as f32 * r.width();
            let y = if hi > lo { r.bottom() - ((v - lo) / span) as f32 * r.height() } else { r.center().y };
            egui::pos2(x, y)
        })
        .collect();
    if pts.len() == 1 {
        ui.painter().circle_filled(pts[0], 1.5, colour);
    } else {
        ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.2_f32, colour)));
    }
    resp.on_hover_text(format!("Last {:.0} s: {} .. {}", line.window_s(), fmt_value(lo), fmt_value(hi)))
}

pub(crate) fn colour32(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

fn key_text(k: MsgKey) -> String {
    if k.extended {
        format!("{:08X}", k.id)
    } else {
        format!("{:03X}", k.id)
    }
}

fn bit_text(bit: u32) -> String {
    format!("B{}.{} (bit {bit})", bit / 8, bit % 8)
}

impl App {
    // ------------------------------------------------------- user signals

    fn user_signals_path(&self) -> Option<std::path::PathBuf> {
        self.lib.as_ref().map(|l| l.root().join(USER_SIGNALS_FILE))
    }

    /// Read `user_signals.json` (at start).
    pub(crate) fn load_user_signals(&mut self) {
        let Some(p) = self.user_signals_path() else { return };
        let (v, note) = usersig::load(&p);
        self.an.usigs = v;
        self.push_user_signals();
        if let Some(n) = note {
            self.set_status(n, true);
        }
    }

    /// Give the user signals to the decoders (UI and worker).
    fn push_user_signals(&mut self) {
        let a = Arc::new(self.an.usigs.clone());
        self.decode_ctx.set_user_signals(a.clone());
        self.bus.send(BusCommand::SetUserSignals(a));
    }

    /// Replace the user signals, save the file and update the decoders.
    pub(crate) fn set_user_signals(&mut self, v: Vec<UserSignal>) {
        self.an.usigs = v;
        self.push_user_signals();
        if self.data_error.is_some() {
            return;
        }
        if let Some(p) = self.user_signals_path() {
            if let Err(e) = usersig::save(&p, &self.an.usigs) {
                self.set_status(format!("User signals not saved: {e}"), true);
            }
        }
    }

    /// Open the Define signal dialog for a new signal.
    pub(crate) fn define_signal(&mut self, key: MsgKey, start_bit: u32, bits: u32, big_endian: bool, signed: bool) {
        let n = self.an.usigs.iter().filter(|s| s.matches(key.id, key.extended)).count();
        let mut s = UserSignal::new(format!("{} sig{}", key_text(key), n + 1), key.id, key.extended, start_bit, bits);
        s.big_endian = big_endian;
        if signed && bits > 1 {
            s.kind = RawKind::Signed;
        }
        s.colour = COLOURS[(self.an.usigs.len() + 1) % COLOURS.len()];
        self.an.draft = Some(SigDraft::new(s, None));
    }

    pub(crate) fn edit_user_signal(&mut self, i: usize) {
        if let Some(s) = self.an.usigs.get(i) {
            self.an.draft = Some(SigDraft::new(s.clone(), Some(i)));
        }
    }

    pub(crate) fn delete_user_signal(&mut self, i: usize) {
        if i < self.an.usigs.len() {
            let mut v = self.an.usigs.clone();
            let s = v.remove(i);
            self.set_user_signals(v);
            self.set_status(format!("User signal {} deleted", s.name), false);
        }
    }

    /// Check and store the dialog's signal. `Err`: the text for the dialog.
    pub(crate) fn save_draft(&mut self, d: &mut SigDraft) -> Result<(), String> {
        let t = d.cob_text.trim();
        let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
        d.sig.cob_id = u32::from_str_radix(t, 16).map_err(|_| "COB-ID: hex value".to_string())?;
        d.sig.name = d.sig.name.trim().to_string();
        d.sig.unit = d.sig.unit.trim().to_string();
        d.sig.validate()?;
        let clash = self.an.usigs.iter().enumerate().any(|(i, s)| Some(i) != d.edit && s.matches(d.sig.cob_id, d.sig.extended) && s.name == d.sig.name);
        if clash {
            return Err("name: already used for this COB-ID".to_string());
        }
        let mut v = self.an.usigs.clone();
        match d.edit {
            Some(i) if i < v.len() => v[i] = d.sig.clone(),
            _ => {
                if v.len() >= MAX_USER_SIGNALS {
                    return Err(format!("at most {MAX_USER_SIGNALS} user signals"));
                }
                v.push(d.sig.clone());
            }
        }
        let name = d.sig.name.clone();
        self.set_user_signals(v);
        self.set_status(format!("User signal {name} saved"), false);
        Ok(())
    }

    pub(crate) fn define_signal_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.an.draft.take() else { return };
        let mut keep = true;
        let mut save = false;
        let mut open = true;
        let err_c = if self.is_dark() { theme::DARK.error } else { theme::LIGHT.error };
        let screen = ctx.screen_rect();
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.memory(|m| m.any_popup_open());
        let title = if d.edit.is_some() { "Edit signal" } else { "Define signal" };
        egui::Window::new(title)
            .id(egui::Id::new("define_signal"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(true)
            .constrain(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                egui::Grid::new("sigdef_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut d.sig.name).desired_width(200.0));
                    ui.end_row();
                    ui.label("COB-ID 0x");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut d.cob_text).desired_width(80.0));
                        ui.checkbox(&mut d.sig.extended, "Ext");
                    });
                    ui.end_row();
                    ui.label("Start bit");
                    ui.add(egui::DragValue::new(&mut d.sig.start_bit).range(0..=63));
                    ui.end_row();
                    ui.label("Length");
                    ui.add(egui::DragValue::new(&mut d.sig.bits).range(1..=64).suffix(" bit"));
                    ui.end_row();
                    ui.label("Byte order");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut d.sig.big_endian, false, "Little endian");
                        ui.selectable_value(&mut d.sig.big_endian, true, "Big endian");
                    });
                    ui.end_row();
                    ui.label("Type");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut d.sig.kind, RawKind::Unsigned, "Unsigned");
                        ui.selectable_value(&mut d.sig.kind, RawKind::Signed, "Signed");
                        ui.selectable_value(&mut d.sig.kind, RawKind::Float, "Float32");
                    });
                    ui.end_row();
                    ui.label("Factor");
                    ui.add(egui::DragValue::new(&mut d.sig.factor).speed(0.01));
                    ui.end_row();
                    ui.label("Offset");
                    ui.add(egui::DragValue::new(&mut d.sig.offset).speed(0.1));
                    ui.end_row();
                    ui.label("Unit");
                    ui.add(egui::TextEdit::singleline(&mut d.sig.unit).desired_width(60.0));
                    ui.end_row();
                    ui.label("Colour");
                    ui.horizontal(|ui| {
                        for (i, c) in COLOURS.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                            ui.painter().rect_filled(r.shrink(2.0), 3.0, colour32(*c));
                            if d.sig.colour == *c {
                                ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(2.0_f32, ui.visuals().strong_text_color()), egui::StrokeKind::Inside);
                            }
                            if resp.on_hover_text(format!("Colour {}", i + 1)).clicked() {
                                d.sig.colour = *c;
                            }
                        }
                    });
                    ui.end_row();
                });
                ui.label(egui::RichText::new("Start bit: the lowest bit (little endian) or the highest bit (big endian, as in DBC files); bit = byte × 8 + bit in the byte.").small());
                if let Some(e) = &d.error {
                    ui.colored_label(err_c, e);
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
        if !open || escape {
            keep = false;
        }
        if save {
            match self.save_draft(&mut d) {
                Ok(()) => keep = false,
                Err(e) => d.error = Some(e),
            }
        }
        if keep {
            self.an.draft = Some(d);
        }
    }

    // --------------------------------------------------------- DBC import

    /// "Import DBC...": pick a file, then the merge dialog.
    pub(crate) fn import_dbc_dialog(&mut self) {
        let picked = rfd::FileDialog::new().set_title("DBC file to import as user signals").add_filter("DBC", &["dbc"]).add_filter("All files", &["*"]).pick_file();
        if let Some(p) = picked {
            self.open_dbc(&p);
        }
    }

    /// Read a DBC file and open the merge dialog (or show the error).
    pub(crate) fn open_dbc(&mut self, path: &std::path::Path) {
        match crate::dbc::load(path) {
            Ok(import) => {
                let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.an.dbc = Some(DbcPending { file, import });
            }
            Err(e) => self.set_status(format!("DBC not read: {e}"), true),
        }
    }

    /// Merge the pending import into the user signals.
    pub(crate) fn apply_dbc(&mut self, replace: bool) {
        let Some(d) = self.an.dbc.take() else { return };
        let (v, rep) = crate::dbc::merge(&self.an.usigs, &d.import, replace);
        self.set_user_signals(v);
        self.set_status(format!("DBC {}: {}", d.file, rep.text()), false);
    }

    pub(crate) fn dbc_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = &self.an.dbc else { return };
        let warn_c = if self.is_dark() { theme::DARK.warning } else { theme::LIGHT.warning };
        let screen = ctx.screen_rect();
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.memory(|m| m.any_popup_open());
        let mut open = true;
        let mut action: Option<Option<bool>> = None;
        let n_sig = d.import.signal_count();
        let n_msg = d.import.messages.iter().filter(|m| !m.signals.is_empty()).count();
        egui::Window::new("Import DBC")
            .id(egui::Id::new("import_dbc"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .constrain(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&d.file).strong());
                ui.label(format!("{n_msg} message(s) with {n_sig} signal(s) to import ({} message(s) in the file)", d.import.messages.len()));
                if d.import.skipped.is_empty() {
                    ui.label("Nothing skipped.");
                } else {
                    let mux = if d.import.multiplexed > 0 { format!(", {} multiplexed", d.import.multiplexed) } else { String::new() };
                    ui.colored_label(warn_c, format!("Skipped: {}{mux}", d.import.skipped.len()));
                    egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                        for s in &d.import.skipped {
                            ui.label(egui::RichText::new(s).small());
                        }
                    });
                }
                ui.label(egui::RichText::new("Add keeps the user signals you have and skips a signal whose name is already used on the same ID. Replace first removes every user signal of the IDs in this file.").small());
                ui.horizontal(|ui| {
                    if ui.add_enabled(n_sig > 0, egui::Button::new("Add")).clicked() {
                        action = Some(Some(false));
                    }
                    if ui.add_enabled(n_sig > 0, egui::Button::new("Replace all signals of these IDs")).clicked() {
                        action = Some(Some(true));
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(None);
                    }
                });
            });
        if !open || escape {
            action = Some(None);
        }
        match action {
            Some(Some(replace)) => self.apply_dbc(replace),
            Some(None) => self.an.dbc = None,
            None => {}
        }
    }

    // -------------------------------------------------------- frames source

    /// The analysis frames: the loaded log up to the replay cursor, else
    /// the trace buffer.
    fn frame_src(&self) -> FrameSrc {
        let log = self.bus.shared.lock().ok().and_then(|s| Some((s.replay_frames.clone()?, s.replay.as_ref()?.cursor)));
        match log {
            Some((v, n)) => FrameSrc::Log(v, n),
            None => FrameSrc::Own(self.trace.frames().copied().collect()),
        }
    }

    /// A count that changes when new analysis frames arrive.
    fn frames_mark(&self) -> u64 {
        let cur = self.bus.shared.lock().ok().and_then(|s| s.replay.as_ref().map(|r| r.cursor as u64));
        cur.unwrap_or_else(|| self.trace.total())
    }

    // ------------------------------------------------------- similar bits

    /// Start Find similar bits for this reference (off the UI thread).
    pub(crate) fn start_similar(&mut self, reference: BitRef) {
        if let Some(j) = &self.an.similar {
            j.cancel.store(true, Ordering::Relaxed);
        }
        let src = self.frame_src();
        let total = src.frames().len();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let (c, p) = (cancel.clone(), progress.clone());
        // The window polls the result while the run is busy.
        let spawned = std::thread::Builder::new().name("similar-bits".to_string()).spawn(move || {
            let r = analysis::similar_bits(src.frames(), reference, &c, Some(&p));
            let _ = tx.send(r);
        });
        if spawned.is_err() {
            self.set_status("Find similar bits: no thread", true);
            return;
        }
        self.an.similar = Some(SimilarJob { reference, cancel, progress, total, rx: Some(rx), result: None, cancelled: false });
    }

    pub(crate) fn similar_window(&mut self, ctx: &egui::Context) {
        let Some(mut j) = self.an.similar.take() else { return };
        if let Some(rx) = &j.rx {
            match rx.try_recv() {
                Ok(r) => {
                    j.cancelled = r.is_none();
                    j.result = r;
                    j.rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => j.rx = None,
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        let mut open = true;
        let mut pick: Option<(MsgKey, u32)> = None;
        let p = if self.is_dark() { &theme::DARK } else { &theme::LIGHT };
        let screen = ctx.screen_rect();
        egui::Window::new("Similar bits")
            .id(egui::Id::new("similar_bits"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .constrain(true)
            .default_size(egui::vec2(520.0, 380.0))
            .default_pos(screen.center() - egui::vec2(260.0, 190.0))
            .show(ctx, |ui| {
                ui.label(format!("Reference: {} {}, over {} frames", key_text(j.reference.key), bit_text(j.reference.bit), j.total));
                if j.rx.is_some() {
                    ui.horizontal(|ui| {
                        let done = j.progress.load(Ordering::Relaxed);
                        ui.add(egui::ProgressBar::new(if j.total == 0 { 0.0 } else { done as f32 / j.total as f32 }).desired_width(240.0));
                        if ui.button("Cancel").clicked() {
                            j.cancel.store(true, Ordering::Relaxed);
                        }
                    });
                    return;
                }
                if j.cancelled {
                    ui.label(egui::RichText::new("Cancelled.").color(p.text_secondary));
                    return;
                }
                let Some(res) = &j.result else { return };
                if res.is_empty() {
                    ui.label(egui::RichText::new("No other bit toggled in these frames.").color(p.text_secondary));
                    return;
                }
                let shown = res.len().min(MAX_SIMILAR_ROWS);
                ui.label(egui::RichText::new(format!("{} bits toggled{}; click a row to select the bit.", res.len(), if shown < res.len() { format!(", best {shown} shown") } else { String::new() })).color(p.text_secondary));
                ui.style_mut().interaction.selectable_labels = false;
                TableBuilder::new(ui)
                    .id_salt("similar_table")
                    .striped(true)
                    .sense(egui::Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::initial(80.0))
                    .column(Column::initial(110.0))
                    .column(Column::initial(70.0))
                    .column(Column::initial(90.0))
                    .column(Column::remainder().at_least(60.0))
                    .min_scrolled_height(0.0)
                    .header(20.0, |mut h| {
                        for t in ["COB-ID", "Bit", "Match", "Inverted", "Toggles"] {
                            h.col(|ui| {
                                ui.strong(t);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(20.0, shown, |mut row| {
                            let m = res[row.index()];
                            row.col(|ui| {
                                ui.monospace(key_text(m.key));
                            });
                            row.col(|ui| {
                                ui.monospace(bit_text(m.bit));
                            });
                            row.col(|ui| {
                                ui.monospace(format!("{:.1} %", m.match_pct()));
                            });
                            row.col(|ui| {
                                ui.monospace(format!("{:.1} %", m.inverted_pct()));
                            });
                            row.col(|ui| {
                                ui.monospace(m.toggles.to_string());
                            });
                            if row.response().clicked() {
                                pick = Some((m.key, m.bit));
                            }
                        });
                    });
            });
        if let Some((key, bit)) = pick {
            self.select_message_bit(key, bit);
        }
        if open {
            self.an.similar = Some(j);
        } else {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    // -------------------------------------------------------- find signal

    pub(crate) fn findsig_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        ui.label(egui::RichText::new("Narrow down an unknown signal: start with every field of the chosen sizes, then keep the ones whose newest value (live or at the replay cursor) passes each step.").color(p.text_secondary));
        ui.add_space(4.0);
        let fv = &mut self.an.find;
        theme::toolbar(ui, |ui| {
            ui.label("Sizes");
            for (bits, label) in [(1u32, "1 bit"), (8, "8 bit"), (16, "16 bit"), (32, "32 bit")] {
                let mut on = fv.fs.settings.sizes.contains(&bits);
                if ui.checkbox(&mut on, label).changed() {
                    if on {
                        fv.fs.settings.sizes.push(bits);
                    } else {
                        fv.fs.settings.sizes.retain(|b| *b != bits);
                    }
                }
            }
            ui.checkbox(&mut fv.fs.settings.byte_aligned, "Byte aligned").on_hover_text("Multi-bit fields start at a byte border only (off: at every bit)");
            ui.separator();
            ui.selectable_value(&mut fv.fs.settings.big_endian, false, "Little endian");
            ui.selectable_value(&mut fv.fs.settings.big_endian, true, "Big endian");
            ui.separator();
            ui.checkbox(&mut fv.fs.settings.signed, "Signed");
        });
        let mut start = false;
        ui.horizontal(|ui| {
            ui.label("COB-IDs");
            ui.add(egui::TextEdit::singleline(&mut fv.ids_text).hint_text("all (or hex: 181, 281)").desired_width(200.0));
            let t = if fv.fs.started() { "Reset" } else { "Start" };
            let tip = "Take every field of every COB-ID seen with its newest value";
            if ui.add_enabled(!self.msgs.is_empty(), egui::Button::new(t)).on_hover_text(tip).on_disabled_hover_text("No frames yet").clicked() {
                start = true;
            }
        });
        if let Some(e) = &fv.error {
            ui.colored_label(p.error, e);
        }
        if start {
            self.find_reset();
        }
        let fv = &mut self.an.find;
        if !fv.fs.started() {
            return;
        }
        ui.separator();
        let mut apply = false;
        let mut undo = false;
        theme::toolbar(ui, |ui| {
            theme::toolbar_combo(ui, "Step", Some("Step"), fv.op.label(), None, |ui| {
                for o in OpKind::ALL {
                    ui.selectable_value(&mut fv.op, o, o.label());
                }
            });
            if fv.op.values() >= 1 {
                ui.add(egui::DragValue::new(&mut fv.a).speed(1.0));
            }
            if fv.op.values() == 2 {
                ui.label("and");
                ui.add(egui::DragValue::new(&mut fv.b).speed(1.0));
            }
            if ui.button("Apply step").clicked() {
                apply = true;
            }
            if ui.add_enabled(fv.fs.can_undo(), egui::Button::new("Undo step")).clicked() {
                undo = true;
            }
        });
        if apply {
            let step = fv.op.step(fv.a, fv.b);
            let msgs = &self.msgs;
            fv.fs.step(step, |k| msgs.get(k).filter(|e| !e.rtr).map(|e| e.data[..e.dlc.min(8) as usize].to_vec()));
        }
        if undo {
            fv.fs.undo();
        }
        let n = fv.fs.candidates().len();
        theme::toolbar(ui, |ui| {
            ui.strong(format!("{n} candidates"));
            if !fv.fs.steps().is_empty() {
                ui.label(egui::RichText::new(format!("after {}", fv.fs.steps().join(", then "))).color(p.text_secondary));
            }
        });
        if fv.fs.cut {
            ui.colored_label(p.warning, format!("The start set was cut at {} candidates: choose fewer sizes or COB-IDs.", analysis::MAX_CANDIDATES));
        }
        if n == 0 {
            ui.label(egui::RichText::new("No candidate is left. Undo the step or reset.").color(p.text_secondary));
            return;
        }
        if n > LIST_CANDIDATES {
            ui.label(egui::RichText::new(format!("The candidates are listed at {LIST_CANDIDATES} or fewer.")).color(p.text_secondary));
            return;
        }
        let cands: Vec<analysis::Candidate> = fv.fs.candidates().to_vec();
        let (be, signed) = (fv.fs.settings.big_endian, fv.fs.settings.signed);
        let mut define = None;
        // A candidate that already is a defined signal gets the Graph and
        // Dashboard buttons.
        let known: Vec<Option<SignalDef>> = cands
            .iter()
            .map(|c| {
                self.an
                    .usigs
                    .iter()
                    .find(|s| s.cob_id == c.key.id && s.extended == c.key.extended && s.start_bit == c.start_bit && s.bits == c.bits && s.big_endian == be)
                    .map(|s| s.to_def())
            })
            .collect();
        let mut to_graph = None;
        let mut to_dash = None;
        TableBuilder::new(ui)
            .id_salt("findsig_table")
            .striped(self.settings.zebra)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(80.0))
            .column(Column::initial(80.0))
            .column(Column::initial(60.0))
            .column(Column::initial(110.0))
            .column(Column::remainder().at_least(100.0))
            .min_scrolled_height(0.0)
            .header(20.0, |mut h| {
                for t in ["COB-ID", "Start bit", "Bits", "Value", ""] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|mut body| {
                for (c, def) in cands.iter().zip(known.iter()) {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            ui.monospace(key_text(c.key));
                        });
                        row.col(|ui| {
                            ui.monospace(c.start_bit.to_string());
                        });
                        row.col(|ui| {
                            ui.monospace(c.bits.to_string());
                        });
                        row.col(|ui| {
                            ui.monospace(fmt_value(c.value));
                        });
                        row.col(|ui| {
                            if ui.small_button("Define signal...").clicked() {
                                define = Some(*c);
                            }
                            if let Some(d) = def {
                                let (g, b) = messages_view::signal_buttons(ui, &d.name);
                                if g {
                                    to_graph = Some(d.clone());
                                }
                                if b {
                                    to_dash = Some(d.clone());
                                }
                            }
                        });
                    });
                }
            });
        if let Some(c) = define {
            self.define_signal(c.key, c.start_bit, c.bits, be, signed);
        }
        if let Some(d) = to_graph {
            self.signal_to_graph(d);
        }
        if let Some(d) = to_dash {
            self.signal_to_dashboard(d);
        }
    }

    fn find_reset(&mut self) {
        let mut ids = Vec::new();
        for part in self.an.find.ids_text.split([',', ' ', ';']).filter(|s| !s.trim().is_empty()) {
            let t = part.trim();
            let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
            match u32::from_str_radix(t, 16) {
                Ok(id) => {
                    ids.push(MsgKey { id, extended: false });
                    ids.push(MsgKey { id, extended: true });
                }
                Err(_) => {
                    self.an.find.error = Some(format!("COB-ID {part}: hex value"));
                    return;
                }
            }
        }
        if self.an.find.fs.settings.sizes.is_empty() {
            self.an.find.error = Some("Choose at least one size".to_string());
            return;
        }
        self.an.find.error = None;
        let mut settings = self.an.find.fs.settings.clone();
        settings.ids = ids;
        let mut fs = FindSignal::new(settings);
        fs.reset(self.msgs.entries().filter(|e| !e.rtr).map(|e| (e.key, &e.data[..e.dlc.min(8) as usize])));
        fs.settings.ids.clear();
        self.an.find.fs = fs;
    }

    // ------------------------------------------------------------ history

    /// The history section of the Messages detail pane for `key` with the
    /// signal columns `sigs`.
    pub(crate) fn history_section(&mut self, ui: &mut egui::Ui, key: MsgKey, sigs: Vec<UserSignal>) {
        let p = theme::pal(ui);
        let hv = &mut self.an.hist;
        if hv.col >= sigs.len() {
            hv.col = 0;
        }
        let mut export = false;
        theme::toolbar(ui, |ui| {
            ui.strong("History");
            ui.checkbox(&mut hv.filter_on, "Filter").on_hover_text("Show only the frames whose signal value passes the test");
            ui.add_enabled_ui(hv.filter_on && !sigs.is_empty(), |ui| {
                let cur = sigs.get(hv.col).map(|s| s.name.clone()).unwrap_or_else(|| "(no signal)".to_string());
                theme::toolbar_combo(ui, "hist_sig", None, &cur, Some(160.0), |ui| {
                    for (i, s) in sigs.iter().enumerate() {
                        ui.selectable_value(&mut hv.col, i, &s.name);
                    }
                });
                let op = hv.op.unwrap_or(CmpOp::Eq);
                theme::toolbar_combo(ui, "hist_op", None, op.label(), Some(48.0), |ui| {
                    for o in CmpOp::ALL {
                        if ui.selectable_label(op == o, o.label()).clicked() {
                            hv.op = Some(o);
                        }
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut hv.value_text).hint_text("value").desired_width(70.0));
            });
            if ui.button("Export CSV...").on_hover_text("Write the rows shown (after the filter) as CSV").clicked() {
                export = true;
            }
        });
        let value = hv.value_text.trim().parse::<f64>().ok();
        let filter = if hv.filter_on && !sigs.is_empty() { value.map(|v| HistFilter { col: hv.col, op: hv.op.unwrap_or(CmpOp::Eq), value: v }) } else { None };
        let mark = self.frames_mark();
        let hv = &self.an.hist;
        let stale = match &hv.built {
            Some((k, m, f, at)) => *k != key || *f != filter || hv.sigs != sigs || (*m != mark && at.elapsed() >= Duration::from_millis(300)),
            None => true,
        };
        if stale {
            let rows = {
                let log = self.bus.shared.lock().ok().and_then(|s| Some((s.replay_frames.clone()?, s.replay.as_ref()?.cursor)));
                match log {
                    Some((v, n)) => analysis::history(v[..n.min(v.len())].iter(), key, &sigs, filter.as_ref(), MAX_HIST_ROWS),
                    None => analysis::history(self.trace.frames(), key, &sigs, filter.as_ref(), MAX_HIST_ROWS),
                }
            };
            let hv = &mut self.an.hist;
            hv.rows = rows;
            hv.sigs = sigs;
            hv.built = Some((key, mark, filter, Instant::now()));
        } else if hv.built.as_ref().is_some_and(|b| b.1 != mark) {
            ui.ctx().request_repaint_after(Duration::from_millis(300));
        }
        if export {
            if let Some(path) = rfd::FileDialog::new().set_file_name(format!("history_{}.csv", key_text(key))).add_filter("CSV", &["csv"]).save_file() {
                let hv = &self.an.hist;
                let r = std::fs::File::create(&path).and_then(|f| {
                    let mut w = std::io::BufWriter::new(f);
                    analysis::write_history_csv(&mut w, &hv.rows, &hv.sigs)?;
                    std::io::Write::flush(&mut w)
                });
                match r {
                    Ok(()) => self.set_status(format!("Saved {} rows to {}", self.an.hist.rows.len(), path.display()), false),
                    Err(e) => self.set_status(format!("CSV not saved: {e}"), true),
                }
            }
        }
        let hv = &self.an.hist;
        if hv.rows.is_empty() {
            let t = if filter.is_some() { "No frame passes the filter." } else { "No frames of this COB-ID in the buffer." };
            ui.label(egui::RichText::new(t).color(p.text_secondary));
            return;
        }
        let row_h = self.settings.row_density.row_height();
        // Fill the pane down to its bottom edge (the clip rect), so that the
        // rows do not grow the panel.
        let max_h = (ui.clip_rect().bottom() - ui.cursor().top() - row_h - 8.0).max(80.0);
        let mut tb = TableBuilder::new(ui)
            .id_salt(("hist_table", key))
            .striped(self.settings.zebra)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).at_least(70.0))
            .column(Column::initial(80.0).at_least(50.0))
            .column(Column::initial(180.0).at_least(80.0).clip(true));
        for _ in &hv.sigs {
            tb = tb.column(Column::initial(100.0).at_least(50.0).clip(true));
        }
        tb.column(Column::remainder().at_least(10.0))
            .min_scrolled_height(0.0)
            .max_scroll_height(max_h)
            .header(row_h, |mut h| {
                for t in ["Time (s)", "Δt (ms)", "Data"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
                for s in &hv.sigs {
                    h.col(|ui| {
                        ui.strong(if s.unit.is_empty() { s.name.clone() } else { format!("{} [{}]", s.name, s.unit) });
                    });
                }
                h.col(|_| {});
            })
            .body(|body| {
                body.rows(row_h, hv.rows.len(), |mut row| {
                    let r = &hv.rows[row.index()];
                    row.col(|ui| {
                        ui.monospace(format!("{:.6}", r.t_us as f64 / 1e6));
                    });
                    row.col(|ui| {
                        ui.monospace(r.dt_us.map(|d| format!("{:.3}", d as f64 / 1000.0)).unwrap_or_default());
                    });
                    row.col(|ui| {
                        ui.monospace(if r.rtr { "RTR".to_string() } else { crate::canopen::types::hex_bytes(&r.data[..r.len as usize]) });
                    });
                    for v in &r.values {
                        row.col(|ui| {
                            ui.monospace(v.map(fmt_value).unwrap_or_default());
                        });
                    }
                    row.col(|_| {});
                });
            });
    }
}
