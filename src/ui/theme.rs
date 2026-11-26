//! Colour tokens, fonts, text styles and egui `Visuals` of the v0.2 design
//! (docs/UI_DESIGN.md sections 1 to 4). All colours come from the two
//! palettes below; views ask `pal(ui)` for the palette of the theme in use.

use crate::canopen::decode::MsgClass;
use crate::settings::{Settings, Theme};
use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke, TextStyle};
use std::sync::Arc;

/// Colour tokens of one theme (spec section 3).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // every token of the spec; some have no egui use yet
pub(crate) struct Palette {
    pub dark: bool,
    pub bg_frame: Color32,
    pub bg_panel: Color32,
    pub bg_bar: Color32,
    pub bg_row_alt: Color32,
    pub bg_row_hover: Color32,
    pub bg_selection: Color32,
    pub bg_input: Color32,
    pub bg_popup: Color32,
    pub bg_button: Color32,
    pub bg_button_hover: Color32,
    pub bg_button_active: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_disabled: Color32,
    pub text_accent: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,
    pub border_hover: Color32,
    pub accent_fill: Color32,
    pub accent_fill_hover: Color32,
    pub accent_fill_active: Color32,
    pub on_accent: Color32,
    pub danger_fill: Color32,
    pub pending_fill: Color32,
    pub pending_border: Color32,
    pub success: Color32,
    pub success_tint: Color32,
    pub warning: Color32,
    pub warning_tint: Color32,
    pub error: Color32,
    pub error_tint: Color32,
    pub info: Color32,
    pub info_tint: Color32,
    pub shadow: Color32,
}

const fn c(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub(crate) const DARK: Palette = Palette {
    dark: true,
    bg_frame: c(0x101215),
    bg_panel: c(0x181B1F),
    bg_bar: c(0x1E2227),
    bg_row_alt: c(0x1B1E23),
    bg_row_hover: c(0x232830),
    bg_selection: c(0x1D3A5C),
    bg_input: c(0x111316),
    bg_popup: c(0x21252B),
    bg_button: c(0x2A3038),
    bg_button_hover: c(0x343B45),
    bg_button_active: c(0x3E4652),
    text_primary: c(0xE6E9ED),
    text_secondary: c(0xA9B2BD),
    text_disabled: c(0x6E7783),
    text_accent: c(0x79B8FF),
    border_subtle: c(0x262A30),
    border_strong: c(0x363D46),
    border_hover: c(0x363D46),
    accent_fill: c(0x2A64BA),
    accent_fill_hover: c(0x3270CC),
    accent_fill_active: c(0x24579F),
    on_accent: c(0xFFFFFF),
    danger_fill: c(0xC93636),
    pending_fill: c(0x3A3320),
    pending_border: c(0xE5B04B),
    success: c(0x4CC38A),
    success_tint: c(0x203630),
    warning: c(0xE5B04B),
    warning_tint: c(0x393326),
    error: c(0xFF7A7A),
    error_tint: c(0x3D2A2E),
    info: c(0x79B8FF),
    info_tint: c(0x283443),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 89),
};

pub(crate) const LIGHT: Palette = Palette {
    dark: false,
    bg_frame: c(0xE3E7EC),
    bg_panel: c(0xFFFFFF),
    bg_bar: c(0xF3F5F8),
    bg_row_alt: c(0xF7F9FB),
    bg_row_hover: c(0xEDF2F8),
    bg_selection: c(0xD4E5FB),
    bg_input: c(0xFFFFFF),
    bg_popup: c(0xFFFFFF),
    bg_button: c(0xE7EBF0),
    bg_button_hover: c(0xDCE2E9),
    bg_button_active: c(0xCDD5DF),
    text_primary: c(0x14181D),
    text_secondary: c(0x4A5461),
    text_disabled: c(0x8F98A3),
    text_accent: c(0x1458B8),
    border_subtle: c(0xDDE2E8),
    border_strong: c(0xC3CAD3),
    border_hover: c(0xAEB7C2),
    accent_fill: c(0x1458B8),
    accent_fill_hover: c(0x0F4C9F),
    accent_fill_active: c(0x0B3F86),
    on_accent: c(0xFFFFFF),
    danger_fill: c(0xB42323),
    pending_fill: c(0xFFF4D6),
    pending_border: c(0x8A5300),
    success: c(0x16703F),
    success_tint: c(0xE3EEE8),
    warning: c(0x8A5300),
    warning_tint: c(0xF1EAE0),
    error: c(0xB42323),
    error_tint: c(0xF6E5E5),
    info: c(0x1458B8),
    info_tint: c(0xE3EBF6),
    // #0B1320 at 18 %, premultiplied.
    shadow: Color32::from_rgba_premultiplied(2, 3, 6, 46),
};

/// The palette of the theme a `Ui` is drawn with.
pub(crate) fn pal(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

/// Text colour and fill of a message type chip (spec 3.5).
pub(crate) fn class_colours(p: &Palette, class: MsgClass) -> (Color32, Color32) {
    use MsgClass::*;
    let (dt, df, lt, lf) = match class {
        Nmt => (0xC3A6FF, 0x333143, 0x6A3DB8, 0xEDE8F6),
        Sync => (0x5CCFE0, 0x23383E, 0x0A6E7F, 0xE2EEF0),
        Emcy => (0xFFA25C, 0x3D3129, 0xA24A00, 0xF4E9E0),
        Time => (0xA9B2BD, 0x2F3338, 0x4A5461, 0xE9EAEC),
        Tpdo | Rpdo => (0x6FD08A, 0x263830, 0x1C6E33, 0xE4EEE7),
        SdoReq | SdoResp => (0x79B8FF, 0x283443, 0x1458B8, 0xE3EBF6),
        Heartbeat => (0xF49AD0, 0x3B2F3B, 0xA1286A, 0xF4E5ED),
        Lss => (0xE3D36B, 0x38382B, 0x6E6000, 0xEEECE0),
        // The palette's info tokens (no new colour).
        J1939 => return (p.info, p.info_tint),
        Other => return (p.text_secondary, p.bg_button),
    };
    if p.dark {
        (c(dt), c(df))
    } else {
        (c(lt), c(lf))
    }
}

/// Node state pill colours (spec 3.7): (text, fill), by the state label.
pub(crate) fn state_colours(p: &Palette, label: &str) -> (Color32, Color32, &'static str) {
    let l = label.to_ascii_lowercase();
    let (dt, df, lt, lf, icon) = if l.contains("boot") {
        (0x5CCFE0, 0x23383E, 0x0A6E7F, 0xE2EEF0, egui_phosphor::regular::CIRCLE_DASHED)
    } else if l.contains("pre") {
        (0xE5B04B, 0x393326, 0x8A5300, 0xF1EAE0, egui_phosphor::regular::CIRCLE_HALF)
    } else if l.contains("oper") {
        (0x4CC38A, 0x203630, 0x16703F, 0xE3EEE8, egui_phosphor::regular::RECORD)
    } else if l.contains("stop") {
        (0xA9B2BD, 0x2F3338, 0x4A5461, 0xE9EAEC, egui_phosphor::regular::STOP_CIRCLE)
    } else if l.contains("lost") {
        (0xFF7A7A, 0x3D2A2E, 0xB42323, 0xF6E5E5, egui_phosphor::regular::X_CIRCLE)
    } else {
        return (p.text_secondary, Color32::TRANSPARENT, egui_phosphor::regular::QUESTION);
    };
    if p.dark {
        (c(dt), c(df), icon)
    } else {
        (c(lt), c(lf), icon)
    }
}

/// Graph series colours (spec 3.8), in assignment order.
pub(crate) fn series_colour(p: &Palette, i: usize) -> Color32 {
    const D: [u32; 8] = [0x5AA9F0, 0xF0A030, 0x3FC1A6, 0xE86FB2, 0xE8D44D, 0xA98BF5, 0xF07858, 0xAAB6C2];
    const L: [u32; 8] = [0x1F6FC0, 0xB86400, 0x0E8A70, 0xB83C86, 0x857400, 0x6B4FC8, 0xC3401E, 0x5A6B7C];
    c(if p.dark { D[i % 8] } else { L[i % 8] })
}

/// Messages view: the full change tint of a data byte (blue: the value went
/// up, red: down). The tint fades towards the row background; primary text
/// keeps the AA contrast on every mix (tested).
pub(crate) fn change_tint(p: &Palette, up: bool) -> Color32 {
    match (p.dark, up) {
        (true, true) => c(0x1F4E8C),
        (true, false) => c(0x7A2430),
        (false, true) => c(0xBFD8F8),
        (false, false) => c(0xF6C4C4),
    }
}

/// Messages view: the heat colour of a bit cell by its toggle rate (0..1).
pub(crate) fn bit_heat(p: &Palette, rate: f32) -> Color32 {
    let hot = if p.dark { c(0x7A4A10) } else { c(0xF5C98A) };
    mix(p.bg_button, hot, rate)
}

/// Messages bit view: the change flash of a bit or Hex cell over the cell
/// fill `base` with the strength `k` (1 = full): (fill, text colour). The
/// full flash is the warning colour; the text turns to the panel colour
/// while the flash is strong, so that it stays readable on it.
pub(crate) fn bit_flash(p: &Palette, base: Color32, k: f32) -> (Color32, Color32) {
    let fill = mix(base, p.warning, k);
    let text = if k > 0.5 { p.bg_panel } else { p.text_primary };
    (fill, text)
}

/// `a` mixed with `b` by `t` (0: `a`, 1: `b`), opaque.
pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// A rounded label with a tint fill (pill / chip). With `outlined` (the
/// "reduce colour" setting) the fill is replaced by a 1 px border.
pub(crate) fn pill(ui: &mut egui::Ui, text: impl Into<String>, fg: Color32, fill: Color32, outlined: bool) -> egui::Response {
    let (fill, stroke) = if outlined || fill == Color32::TRANSPARENT { (Color32::TRANSPARENT, Stroke::new(1.0_f32, if outlined { fg } else { pal(ui).border_strong })) } else { (fill, Stroke::NONE) };
    let text = text.into();
    // A frame takes the rest of the row: in a toolbar row the pill goes
    // to a new row first when it does not fit (see `wrap_for`).
    let w = text_width(ui, &text, TextStyle::Small) + 2.0 * (8.0 + stroke.width);
    wrap_for(ui, w);
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(8, 1))
        .show(ui, |ui| ui.label(egui::RichText::new(text).text_style(TextStyle::Small).color(fg)))
        .inner
}

/// Semantic tone of a result pill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tone {
    Success,
    Neutral,
    Info,
    Warning,
    Error,
}

/// A result pill (spec 5.6) in a semantic tone.
pub(crate) fn tone_pill(ui: &mut egui::Ui, tone: Tone, text: impl Into<String>, outlined: bool) -> egui::Response {
    let p = pal(ui);
    let (fg, bg) = match tone {
        Tone::Success => (p.success, p.success_tint),
        Tone::Neutral => (p.text_secondary, p.bg_button),
        Tone::Info => (p.info, p.info_tint),
        Tone::Warning => (p.warning, p.warning_tint),
        Tone::Error => (p.error, p.error_tint),
    };
    pill(ui, text, fg, bg, outlined)
}

/// A caution note under a control (spec 8.8): small, warning colour, with
/// the warning glyph.
pub(crate) fn caution_note(ui: &mut egui::Ui, text: &str) {
    let p = pal(ui);
    ui.label(egui::RichText::new(format!("{}  {text}", egui_phosphor::regular::WARNING)).small().color(p.warning));
}

/// Ghost icon button (spec 5.1): the glyph only, the action as tooltip and
/// as the accessible name (`name`).
pub(crate) fn icon_button(ui: &mut egui::Ui, enabled: bool, glyph: &str, name: &str, tip: &str) -> egui::Response {
    let r = ui.add_enabled(enabled, egui::SelectableLabel::new(false, egui::RichText::new(glyph).size(16.0))).on_hover_text(tip).on_disabled_hover_text(tip);
    let name = name.to_string();
    r.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    r
}

/// The width of an [`icon_button`] with this glyph in this ui: the glyph
/// galley and the button padding on both sides, as the widget lays it out.
pub(crate) fn icon_button_width(ui: &egui::Ui, glyph: &str) -> f32 {
    let g = egui::WidgetText::from(egui::RichText::new(glyph).size(16.0)).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
    g.size().x + 2.0 * ui.spacing().button_padding.x
}

/// Toggle icon button (spec 5.1: Pause, Autoscroll): ON has the accent
/// tint. Accessible as a check box named `name`.
pub(crate) fn icon_toggle(ui: &mut egui::Ui, on: &mut bool, glyph: &str, name: &str, tip: &str) -> egui::Response {
    let mut r = ui.add(egui::SelectableLabel::new(*on, egui::RichText::new(glyph).size(16.0))).on_hover_text(tip);
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let (v, name) = (*on, name.to_string());
    r.widget_info(move || egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, v, &name));
    r
}

/// Caution button (spec 5.1: actions that can take a device off the bus):
/// warning tint, warning border, a leading warning glyph.
pub(crate) fn caution_button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let p = pal(ui);
    let (fill, border) = if p.dark { (c(0x393326), c(0x745E33)) } else { (c(0xF1EAE0), c(0xCAB28C)) };
    let label = format!("{}  {text}", egui_phosphor::regular::WARNING);
    let b = if enabled {
        egui::Button::new(egui::RichText::new(label).color(p.warning)).fill(fill).stroke(Stroke::new(1.0_f32, border))
    } else {
        egui::Button::new(label)
    };
    let r = ui.add_enabled(enabled, b);
    let name = text.to_string();
    r.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    r
}

/// Danger button (spec 5.1: NMT all, LSS Activate bit timing).
pub(crate) fn danger_button(ui: &mut egui::Ui, enabled: bool, glyph: &str, text: &str) -> egui::Response {
    let p = pal(ui);
    let label = format!("{glyph}  {text}");
    let b = if enabled { egui::Button::new(egui::RichText::new(label).color(Color32::WHITE)).fill(p.danger_fill) } else { egui::Button::new(label) };
    let r = ui.add_enabled(enabled, b);
    let name = text.to_string();
    r.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    r
}

/// A count badge (spec 5.6) painted at the top right of a widget.
pub(crate) fn count_badge(ui: &egui::Ui, rect: egui::Rect, n: usize) {
    if n == 0 {
        return;
    }
    let p = pal(ui);
    let c = egui::pos2(rect.right() - 3.0, rect.top() + 3.0);
    ui.painter().circle_filled(c, 7.0, p.accent_fill);
    ui.painter().text(c, egui::Align2::CENTER_CENTER, n.to_string(), FontId::proportional(10.0), p.on_accent);
}

/// A toolbar row: the widgets from left to right, on as many rows as the
/// width of the view needs (`horizontal_wrapped`). Every toolbar of the
/// views uses it, so that the layout tests can check each one.
pub(crate) fn toolbar<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    #[cfg(test)]
    let panel = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    let r = ui.horizontal_wrapped(add_contents);
    #[cfg(test)]
    toolbar_rects::push(ui.ctx(), panel, r.response.rect);
    r
}

/// In a toolbar row: start a new row when an item `width` points wide does
/// not fit in the rest of the current row (elsewhere: nothing).
///
/// egui wraps a widget that asks for its size (a button, a label, a text
/// field). A combo box, a frame or any nested layout instead gets the rest
/// of the current row as its room, whatever its own width: it never wraps,
/// it runs past the end of the row, and the row (and each row after it)
/// becomes wider than the view. Call this before such an item.
pub(crate) fn wrap_for(ui: &mut egui::Ui, width: f32) {
    let l = ui.layout();
    if l.main_wrap() && l.main_dir() == egui::Direction::LeftToRight && ui.available_size_before_wrap().x < width && ui.cursor().left() > ui.max_rect().left() {
        ui.end_row();
    }
}

/// The width of `text` on one line in `style`.
pub(crate) fn text_width(ui: &egui::Ui, text: &str, style: TextStyle) -> f32 {
    egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, style).size().x
}

/// A combo box in a toolbar row (`egui::ComboBox` with these settings and
/// a side `label`, if any), on a new row when it does not fit in the rest
/// of the current one. Its width is measured as egui lays it out: the
/// selected text with the icon, at least `width` (default
/// `Spacing::combo_width`), the button padding, then the label.
pub(crate) fn toolbar_combo<R>(ui: &mut egui::Ui, key: impl std::hash::Hash, label: Option<&str>, selected: &str, width: Option<f32>, menu: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<Option<R>> {
    let s = ui.spacing();
    let (pad, icon, gap, min) = (s.button_padding.x, s.icon_spacing + s.icon_width, s.item_spacing.x, width.unwrap_or(s.combo_width));
    let mut w = (text_width(ui, selected, TextStyle::Button) + icon).max(min - 2.0 * pad) + 2.0 * pad;
    if let Some(l) = label {
        w += gap + text_width(ui, l, TextStyle::Body);
    }
    wrap_for(ui, w);
    let mut c = match label {
        Some(l) => egui::ComboBox::new(key, l),
        None => egui::ComboBox::from_id_salt(key),
    }
    .selected_text(selected)
    .wrap_mode(egui::TextWrapMode::Extend);
    if let Some(w) = width {
        c = c.width(w);
    }
    c.show_ui(ui, menu)
}

/// Items that stay together in a toolbar row (for example a label and its
/// control in an `add_enabled_ui`), on a new row when they do not fit in
/// the rest of the current one. Their width is the one measured in the
/// last pass: when it is new or has changed, egui runs the pass again
/// (`request_discard`), so that no shown frame has the group at a wrong
/// place. `key` names the group; it must be unique in the window.
pub(crate) fn toolbar_group<R>(ui: &mut egui::Ui, key: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let id = egui::Id::new(("toolbar_group", key));
    let last: Option<f32> = ui.ctx().data(|d| d.get_temp(id));
    wrap_for(ui, last.unwrap_or(0.0));
    let r = ui.horizontal(add_contents);
    let w = r.response.rect.width();
    if last.is_none_or(|l| (l - w).abs() > 0.5) {
        ui.ctx().data_mut(|d| d.insert_temp(id, w));
        ui.ctx().request_discard("toolbar group width");
    }
    r.inner
}

/// Right-aligned items at the end of a toolbar row, when `width` (their
/// width with the gaps) fits in the rest of the current row; returns false
/// and adds nothing when it does not. (`available_width` is no test for
/// this: in a wrapping row it is the width of a whole row.)
pub(crate) fn toolbar_end(ui: &mut egui::Ui, width: f32, add_contents: impl FnOnce(&mut egui::Ui)) -> bool {
    if ui.available_size_before_wrap().x < width {
        return false;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add_contents);
    true
}

/// The toolbar rows of the last pass (tests): the room of the view at the
/// place of each row, and the rectangle that the row used.
#[cfg(test)]
pub(crate) mod toolbar_rects {
    use eframe::egui;

    #[derive(Clone, Default)]
    struct Rows(u64, Vec<(egui::Rect, egui::Rect)>);

    fn id() -> egui::Id {
        egui::Id::new("canoxide_test_toolbar_rects")
    }

    fn bar_id() -> egui::Id {
        egui::Id::new("canoxide_test_bar_rects")
    }

    fn push_to(ctx: &egui::Context, id: egui::Id, panel: egui::Rect, row: egui::Rect) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| {
            let r = d.get_temp_mut_or_default::<Rows>(id);
            if r.0 != pass {
                *r = Rows(pass, Vec::new());
            }
            r.1.push((panel, row));
        });
    }

    pub(crate) fn push(ctx: &egui::Context, panel: egui::Rect, row: egui::Rect) {
        push_to(ctx, id(), panel, row);
    }

    /// A bar of the window (the connection bar, the status bar).
    pub(crate) fn push_bar(ctx: &egui::Context, panel: egui::Rect, row: egui::Rect) {
        push_to(ctx, bar_id(), panel, row);
    }

    /// (view room, row rect) of each toolbar row of the last pass that drew one.
    pub(crate) fn last(ctx: &egui::Context) -> Vec<(egui::Rect, egui::Rect)> {
        ctx.data(|d| d.get_temp::<Rows>(id())).map(|r| r.1).unwrap_or_default()
    }

    /// The same for the bars of the window.
    pub(crate) fn last_bars(ctx: &egui::Context) -> Vec<(egui::Rect, egui::Rect)> {
        ctx.data(|d| d.get_temp::<Rows>(bar_id())).map(|r| r.1).unwrap_or_default()
    }

    pub(crate) fn clear(ctx: &egui::Context) {
        ctx.data_mut(|d| {
            d.remove::<Rows>(id());
            d.remove::<Rows>(bar_id());
        });
    }
}

/// A full-width banner (spec 5.8) with a semantic tint; returns true when
/// its action button (if any) was clicked.
pub(crate) fn banner(ui: &mut egui::Ui, error: bool, text: &str, action: Option<&str>) -> bool {
    let p = pal(ui);
    let (fill, fg, glyph) = if error { (p.error_tint, p.error, egui_phosphor::regular::WARNING_CIRCLE) } else { (p.warning_tint, p.warning, egui_phosphor::regular::WARNING) };
    let mut clicked = false;
    egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(8, 4)).corner_radius(CornerRadius::same(4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        toolbar(ui, |ui| {
            ui.label(egui::RichText::new(glyph).color(fg));
            ui.label(egui::RichText::new(text).color(p.text_primary));
            if let Some(a) = action {
                clicked = ui.button(a).clicked();
            }
        });
    });
    clicked
}

/// Fonts: Inter for text, JetBrains Mono for data, Phosphor icons as a
/// fallback of both (spec section 1). The two text fonts in assets/fonts
/// have their Private Use Area code points (U+E000..U+F8FF) removed:
/// otherwise their own glyphs there hide the Phosphor icons.
pub(crate) fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    f.font_data.insert("Inter".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf"))));
    f.font_data.insert("JetBrainsMono".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"))));
    f.font_data.insert("phosphor".into(), Arc::new(egui_phosphor::Variant::Regular.font_data()));
    if let Some(v) = f.families.get_mut(&FontFamily::Proportional) {
        v.insert(0, "Inter".into());
        v.insert(1, "phosphor".into());
    }
    if let Some(v) = f.families.get_mut(&FontFamily::Monospace) {
        v.insert(0, "JetBrainsMono".into());
        v.insert(1, "phosphor".into());
    }
    f
}

/// Is the theme of the settings dark? `os_dark` is the OS theme (System).
pub(crate) fn is_dark(theme: Theme, os_dark: Option<bool>) -> bool {
    match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => os_dark.unwrap_or(true),
    }
}

/// egui visuals from a palette (spec 3.9).
pub(crate) fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg_panel;
    v.window_fill = p.bg_popup;
    v.extreme_bg_color = p.bg_input;
    v.faint_bg_color = p.bg_row_alt;
    v.code_bg_color = p.bg_bar;
    v.hyperlink_color = p.text_accent;
    v.selection.bg_fill = p.bg_selection;
    v.selection.stroke = Stroke::new(1.0_f32, p.text_accent);
    v.override_text_color = None;
    v.window_stroke = Stroke::new(1.0_f32, p.border_strong);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(4);
    v.window_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: p.shadow };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 8, spread: 0, color: p.shadow };
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.text_cursor.stroke = Stroke::new(2.0_f32, p.text_accent);
    v.striped = true;
    let w = &mut v.widgets;
    let r = CornerRadius::same(4);
    w.noninteractive.bg_fill = p.bg_panel;
    w.noninteractive.weak_bg_fill = p.bg_panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border_subtle);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    w.inactive.bg_fill = p.bg_button;
    w.inactive.weak_bg_fill = p.bg_button;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    w.hovered.bg_fill = p.bg_button_hover;
    w.hovered.weak_bg_fill = p.bg_button_hover;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.border_hover);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    w.active.bg_fill = p.bg_button_active;
    w.active.weak_bg_fill = p.bg_button_active;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.text_accent);
    w.active.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    w.open = w.hovered;
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = r;
        s.expansion = 0.0;
    }
    v
}

/// Apply theme, text sizes, spacing and zoom from the settings.
pub(crate) fn apply(ctx: &egui::Context, s: &Settings, os_dark: Option<bool>) {
    let p = if is_dark(s.theme, os_dark) { &DARK } else { &LIGHT };
    let o = s.font_offset.clamp(-2, 4) as f32;
    // The app decides the theme (System follows the OS theme it reads);
    // egui gets a fixed preference so that it does not switch by itself.
    let theme = if p.dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |st| {
        st.visuals = visuals(p);
        st.text_styles = [
            (TextStyle::Heading, FontId::proportional(15.0 + o)),
            (TextStyle::Body, FontId::proportional(14.0 + o)),
            (TextStyle::Button, FontId::proportional(14.0 + o)),
            (TextStyle::Monospace, FontId::monospace(13.0 + o)),
            (TextStyle::Small, FontId::proportional(12.0 + o)),
            (TextStyle::Name("title".into()), FontId::proportional(17.0 + o)),
            (TextStyle::Name("table".into()), FontId::proportional(13.0 + o)),
            (TextStyle::Name("mono_small".into()), FontId::monospace(12.0 + o)),
        ]
        .into();
        st.spacing.item_spacing = egui::vec2(8.0, 4.0);
        st.spacing.button_padding = egui::vec2(10.0, 4.0);
        st.spacing.interact_size.y = 26.0;
        st.spacing.indent = 16.0;
        st.spacing.scroll = egui::style::ScrollStyle::thin();
    });
    if (ctx.zoom_factor() - s.ui_scale).abs() > 0.001 {
        ctx.set_zoom_factor(s.ui_scale);
    }
}

/// WCAG 2 relative luminance.
#[cfg(test)]
pub(crate) fn luminance(c: Color32) -> f64 {
    let ch = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
}

/// WCAG 2 contrast ratio of two colours.
#[cfg(test)]
pub(crate) fn contrast(a: Color32, b: Color32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pairs of the spec contrast table (section 3.2).
    fn backgrounds(p: &Palette, with_input: bool) -> Vec<(&'static str, Color32)> {
        let mut v = vec![
            ("bg.panel", p.bg_panel),
            ("bg.bar", p.bg_bar),
            ("bg.frame", p.bg_frame),
            ("bg.row.alt", p.bg_row_alt),
            ("bg.row.hover", p.bg_row_hover),
            ("bg.selection", p.bg_selection),
            ("bg.popup", p.bg_popup),
            ("bg.button", p.bg_button),
        ];
        if with_input {
            v.push(("bg.input", p.bg_input));
        }
        v
    }

    #[test]
    fn text_contrast_meets_the_spec_table() {
        for p in [&DARK, &LIGHT] {
            let mut prim = backgrounds(p, p.dark);
            prim.push(("bg.button.hover", p.bg_button_hover));
            prim.push(("bg.button.active", p.bg_button_active));
            for (name, bg) in prim {
                let r = contrast(p.text_primary, bg);
                assert!(r >= 7.0, "text.primary on {name} (dark {}): {r:.2}", p.dark);
            }
            for (name, bg) in backgrounds(p, false) {
                let r = contrast(p.text_secondary, bg);
                assert!(r >= 4.5, "text.secondary on {name} (dark {}): {r:.2}", p.dark);
            }
        }
    }

    #[test]
    fn contrast_formula_known_values() {
        assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 1e-9);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-9);
    }

    /// The change tints and the bit heat keep AA text contrast (4.5) with
    /// primary text at each fade step, on the plain, striped and selected
    /// row.
    #[test]
    fn message_tints_are_readable() {
        for p in [&DARK, &LIGHT] {
            for bg in [p.bg_panel, p.bg_row_alt, p.bg_selection] {
                for up in [true, false] {
                    for i in 0..=10 {
                        let t = mix(bg, change_tint(p, up), i as f32 / 10.0);
                        let r = contrast(p.text_primary, t);
                        assert!(r >= 4.5, "tint up {up} step {i} dark {}: {r:.2}", p.dark);
                    }
                }
            }
            for i in 0..=10 {
                let r = contrast(p.text_primary, bit_heat(p, i as f32 / 10.0));
                assert!(r >= 4.5, "heat step {i} dark {}: {r:.2}", p.dark);
            }
        }
    }

    #[test]
    fn chips_are_readable() {
        for p in [&DARK, &LIGHT] {
            for cl in MsgClass::ALL {
                let (fg, bg) = class_colours(p, cl);
                assert!(contrast(fg, bg) >= 4.5, "{:?} dark {}", cl, p.dark);
            }
        }
    }
}
