# CANoxide 0.5 — UI design specification

This file describes the user interface of CANoxide 0.5.0, built in Rust
with `egui` 0.31, `egui_dock` 0.16 and `egui_plot`. It is complete on its
own; the design canvas only illustrates it. All sizes are egui logical
points at UI scale 100 % (1 pt = 1 px at 100 % Windows scaling). Hex
colours are sRGB, opaque unless an alpha is given.

Design name: **"Bench"**. Cool neutral greys, one blue accent, colour only
where it carries meaning (message type, node state, result). Flat fills,
1 px strokes, 4 px radii. Density first.

---

## 1. Fonts and icons

| Role | Family | File | Licence | Notes |
|---|---|---|---|---|
| UI text | **Inter** Regular (v4.x) | `Inter-Regular.ttf` | SIL OFL 1.1 | Replaces egui's Ubuntu-Light as `FontFamily::Proportional[0]`. |
| Data text | **JetBrains Mono** Regular (v2.304) | `JetBrainsMono-Regular.ttf` | SIL OFL 1.1 | Replaces Hack as `FontFamily::Monospace[0]`. Used for every hex value, COB-ID, index, data byte, number column and timestamp. Tabular figures by design. |
| Icons | **Phosphor** Regular | via crate `egui-phosphor` (the release that targets egui 0.31), variant `regular` | MIT | Add as a fallback font to both families (`egui_phosphor::add_to_fonts(&mut fonts, Variant::Regular)`). |

- Only the Regular weight of each family is embedded (two text font files).
  Hierarchy comes from size and colour (`text.primary` vs `text.secondary`),
  not from weight. egui's `RichText::strong()` maps to `text.primary`.
- Optional size cut: subset both text fonts to Latin-1 + General
  Punctuation + Arrows + Box Drawing with `pyftsubset` (about 90 KB each).
  Keep egui's default emoji fallback out of the build if it is not needed.
- Licence texts go into `licenses/` and the About dialog.
- App icon: `assets/icon/` (see its README). Both exes carry
  `canoxide.ico` as icon group 1 (build.rs, winresource); the window icon
  and the About dialog use the embedded `canoxide_64.png`. The installer
  uses the same `.ico` as its setup icon.

## 2. Typography (at UI scale 100 %)

| Style | Family | Size | Line height | Colour | Use |
|---|---|---|---|---|---|
| `title` | Inter | 17 | 24 | text.primary | Dialog titles |
| `heading` | Inter | 15 | 20 | text.primary | Section titles inside a view, settings section titles |
| `body` | Inter | 14 | 20 | text.primary | Labels, buttons, menus, tab titles, form text |
| `table` | Inter | 13 | 18 | text.primary | Text cells in tables (names, decoded text) |
| `mono` | JetBrains Mono | 13 | 18 | text.primary | Hex, numbers, IDs, timestamps, data bytes |
| `small` | Inter | 12 | 16 | text.secondary | Table headers, captions, hints, status bar, pills, chips |
| `mono.small` | JetBrains Mono | 12 | 16 | text.secondary | Secondary numbers (legend min/max, units) |

- egui `TextStyle` mapping: `Heading`=15, `Body`=14, `Button`=14,
  `Monospace`=13, `Small`=12; add `TextStyle::Name("title")`=17,
  `Name("table")`=13, `Name("mono_small")`=12.
- Setting "Font size" offsets every size by the same amount (−2 … +4).
  Setting "UI scale" uses `ctx.set_zoom_factor`.
- Table headers: `small`, text.secondary, not upper case.
- Numbers right-aligned in numeric columns; hex left-aligned.
- Hex always upper case: `0x1A2B`, bytes `1A 2B 00 FF`. Index shown as
  `1018`, sub as `02` (hex, two digits), both mono.
- Thousands in decimal values use a thin space (U+2009): `1 482`.

## 3. Colour tokens

Contrast = WCAG 2.x ratio of the text colour on the named background.
Targets: body text ≥ 7:1, secondary ≥ 4.5:1 (met on every background the
text is drawn on, including selected rows). Disabled text is exempt by
WCAG; it is shown for reference only.

### 3.1 Backgrounds (by level)

| Token | Dark | Light | Use |
|---|---|---|---|
| `bg.frame` | `#101215` | `#E3E7EC` | Menu bar, connection bar, status bar, dock gaps behind splitters, `ctx.style().visuals.window_fill` of the root panel |
| `bg.panel` | `#181B1F` | `#FFFFFF` | Dock tab content, active tab fill (`panel_fill`) |
| `bg.bar` | `#1E2227` | `#F3F5F8` | Tab bar strip, view toolbars, table header row (`faint_bg_color` for headers) |
| `bg.row.alt` | `#1B1E23` | `#F7F9FB` | Zebra stripe (every second row) |
| `bg.row.hover` | `#232830` | `#EDF2F8` | Row under the pointer |
| `bg.selection` | `#1D3A5C` | `#D4E5FB` | Selected row/cell fill (`selection.bg_fill`) |
| `bg.input` | `#111316` | `#FFFFFF` | Text edit and combo box fill (`extreme_bg_color`) |
| `bg.popup` | `#21252B` | `#FFFFFF` | Menus, combo popups, dialogs, tooltips (`window_fill`) |
| `bg.button` | `#2A3038` | `#E7EBF0` | Secondary button, inactive widget (`widgets.inactive.weak_bg_fill`) |
| `bg.button.hover` | `#343B45` | `#DCE2E9` | `widgets.hovered.weak_bg_fill` |
| `bg.button.active` | `#3E4652` | `#CDD5DF` | Pressed (`widgets.active.weak_bg_fill`) |
| `bg.scrim` | `#000000` @ 55 % | `#0B1320` @ 35 % | Behind a modal dialog |

### 3.2 Text

| Token | Dark | Contrast (dark) | Light | Contrast (light) |
|---|---|---|---|---|
| `text.primary` | `#E6E9ED` | panel 14.19, bar 13.13, frame 15.40, alt 13.72, hover 12.16, selection 9.52, input 15.28, popup 12.64, button 10.92, button.hover 9.28, button.active 7.83 | `#14181D` | panel 17.82, bar 16.32, frame 14.35, alt 16.89, hover 15.84, selection 13.92, popup 17.82, button 14.89, button.hover 13.66, button.active 12.04 |
| `text.secondary` | `#A9B2BD` | panel 8.05, bar 7.45, frame 8.75, alt 7.79, hover 6.91, selection 5.40, popup 7.18, button 6.20 | `#4A5461` | panel 7.69, bar 7.04, frame 6.19, alt 7.28, hover 6.83, selection 6.00, popup 7.69, button 6.42 |
| `text.disabled` | `#6E7783` | panel 3.81, bar 3.53 (exempt) | `#8F98A3` | panel 2.92, bar 2.67 (exempt) |
| `text.accent` (links, focused tab title is NOT accent) | `#79B8FF` | panel 8.32, bar 7.70, selection 5.58 | `#1458B8` | panel 6.74, bar 6.17, selection 5.26 |
| `text.on_accent` | `#FFFFFF` | on accent.fill 5.78 | `#FFFFFF` | on accent.fill 6.74 |
| `text.on_danger` | `#FFFFFF` | on danger.fill 5.17 | `#FFFFFF` | on danger.fill 6.55 |

Placeholder ("hint") text in inputs uses `text.secondary`, never disabled
(the v0.1 hints were too faint).

### 3.3 Borders, accent, focus

| Token | Dark | Light | Use |
|---|---|---|---|
| `border.subtle` | `#262A30` | `#DDE2E8` | Table grid lines (horizontal only), separators inside a view |
| `border.strong` | `#363D46` | `#C3CAD3` | Input/button outline, tab outline, dialog outline, splitter idle line |
| `accent` | `#79B8FF` | `#1458B8` | Focus ring, selection outline, active toggle, slider, plot cursor, drop-zone outline |
| `accent.fill` | `#2A64BA` | `#1458B8` | Primary button fill |
| `accent.fill.hover` | `#3270CC` | `#0F4C9F` | Primary button hover |
| `accent.fill.active` | `#24579F` | `#0B3F86` | Primary button pressed |
| `accent.tint` | `#79B8FF` @ 22 % | `#1458B8` @ 16 % | Dock drop-zone fill, selected segment of a segmented control (with accent text) |
| `danger.fill` | `#C93636` | `#B42323` | Danger button (NMT to all, Write all, LSS Activate bit rate); hover/pressed in 5.1 |
| `pending.fill` | `#3A3320` | `#FFF4D6` | Cell with an unsaved new value / pending PDO change |
| `pending.border` | `#E5B04B` | `#8A5300` | 1 px outline of a pending cell |

Focus ring: 1 px `accent` outline, 1 px outside the widget, on keyboard
focus only (`ctx.memory().focused()`).

### 3.4 Semantic (success / warning / error / info)

Each colour is used as text/icon on `bg.panel`, and as a pill/chip with the
tint fill and the colour as text. Tint = colour mixed over `bg.panel` at
16 % (dark) / 12 % (light); border = mix at 45 %. All ratios below are
"text colour on tint fill".

| Token | Dark text | Dark tint fill | Dark border | ratio | Light text | Light tint fill | Light border | ratio |
|---|---|---|---|---|---|---|---|---|
| `success` | `#4CC38A` | `#203630` | `#2F674F` | 5.81 | `#16703F` | `#E3EEE8` | `#96BFA9` | 5.16 |
| `warning` | `#E5B04B` | `#393326` | `#745E33` | 6.35 | `#8A5300` | `#F1EAE0` | `#CAB28C` | 5.30 |
| `error` | `#FF7A7A` | `#3D2A2E` | `#804648` | 5.30 | `#B42323` | `#F6E5E5` | `#DD9C9C` | 5.38 |
| `info` | `#79B8FF` | `#283443` | `#446284` | 6.08 | `#1458B8` | `#E3EBF6` | `#95B4DF` | 5.61 |

On `bg.panel` directly: dark success 7.80, warning 8.76, error 6.84, info
8.32; light success 6.13, warning 6.33, error 6.55, info 6.74.

### 3.5 CAN / CANopen message types (trace chips, filters)

Chip text = the type label, so the meaning never depends on colour alone.

| Type | Label | Dark text | Dark fill | ratio | Light text | Light fill | ratio |
|---|---|---|---|---|---|---|---|
| NMT | `NMT` | `#C3A6FF` | `#333143` | 6.15 | `#6A3DB8` | `#EDE8F6` | 5.88 |
| SYNC | `SYNC` | `#5CCFE0` | `#23383E` | 6.71 | `#0A6E7F` | `#E2EEF0` | 4.99 |
| EMCY | `EMCY` | `#FFA25C` | `#3D3129` | 6.33 | `#A24A00` | `#F4E9E0` | 5.00 |
| TIME | `TIME` | `#A9B2BD` | `#2F3338` | 5.93 | `#4A5461` | `#E9EAEC` | 6.39 |
| PDO (R/T) | `RPDO1`…`TPDO4` | `#6FD08A` | `#263830` | 6.54 | `#1C6E33` | `#E4EEE7` | 5.31 |
| SDO | `SDO↑` (client→server) `SDO↓` | `#79B8FF` | `#283443` | 6.08 | `#1458B8` | `#E3EBF6` | 5.61 |
| Heartbeat / node guard | `HB` | `#F49AD0` | `#3B2F3B` | 6.27 | `#A1286A` | `#F4E5ED` | 5.71 |
| LSS | `LSS` | `#E3D36B` | `#38382B` | 7.80 | `#6E6000` | `#EEECE0` | 5.31 |
| Error frame / bus error | `ERR` | `#FF7A7A` | `#3D2A2E` | 5.30 | `#B42323` | `#F6E5E5` | 5.38 |
| Other / raw | `RAW` | text.secondary | bg.button | 6.20 | text.secondary | bg.button | 6.42 |

Row tint (trace setting "Colour by type = Row tint", and always for ERR and
EMCY rows): row fill = type colour at 8 % (dark) / 6 % (light) over
`bg.panel`. Dark ERR row `#2A2125`, EMCY row `#2B2521`; light ERR row
`#FBF1F1`, EMCY row `#FAF4EE`. Text in a tinted row stays text.primary.

### 3.6 Direction Rx / Tx

| Dir | Glyph | Text | Colour |
|---|---|---|---|
| Rx | Phosphor `arrow-down-left` | `Rx` | text.secondary |
| Tx | Phosphor `arrow-up-right` | `Tx` | text.accent (dark `#79B8FF`, light `#1458B8`) |

### 3.7 Node states (Network pills, Devices list dot)

| State | Label | Glyph | Dark text / fill | Light text / fill |
|---|---|---|---|---|
| Boot-up | `Boot-up` | `circle-dashed` | `#5CCFE0` / `#23383E` | `#0A6E7F` / `#E2EEF0` |
| Pre-operational | `Pre-op` | `circle-half` | `#E5B04B` / `#393326` | `#8A5300` / `#F1EAE0` |
| Operational | `Operational` | `circle` (filled: `record`) | `#4CC38A` / `#203630` | `#16703F` / `#E3EEE8` |
| Stopped | `Stopped` | `stop-circle` | `#A9B2BD` / `#2F3338` | `#4A5461` / `#E9EAEC` |
| Lost (heartbeat timeout) | `Lost` | `x-circle` | `#FF7A7A` / `#3D2A2E` | `#B42323` / `#F6E5E5` |
| Unknown (never seen) | `Unknown` | `question` | text.secondary / transparent, 1 px border.strong | same |

Ratios as in 3.4/3.5 (boot = SYNC values, pre-op = warning, op = success,
stopped = TIME, lost = error). Each state has its own glyph, so the pill
works without colour.

### 3.8 Graph series (8)

Each series has a colour AND a line style, so a series is identifiable for
colour-blind users and in grey-scale prints. Order is the assignment order.
Contrast of the line on `bg.panel` (graphics need ≥ 3:1).

| # | Dark | ratio | Light | ratio | `egui_plot::LineStyle` | Marker in legend |
|---|---|---|---|---|---|---|
| 1 | `#5AA9F0` blue | 6.89 | `#1F6FC0` | 5.13 | Solid | circle |
| 2 | `#F0A030` orange | 8.04 | `#B86400` | 4.32 | Solid | square |
| 3 | `#3FC1A6` teal | 7.73 | `#0E8A70` | 4.30 | Solid | diamond |
| 4 | `#E86FB2` pink | 6.05 | `#B83C86` | 5.21 | Solid | up triangle |
| 5 | `#E8D44D` yellow | 11.48 | `#857400` | 4.67 | Dashed 6/4 | circle |
| 6 | `#A98BF5` violet | 6.33 | `#6B4FC8` | 5.87 | Dashed 6/4 | square |
| 7 | `#F07858` vermilion | 6.20 | `#C3401E` | 5.16 | Dotted 2/3 | diamond |
| 8 | `#AAB6C2` grey | 8.38 | `#5A6B7C` | 5.48 | Dotted 2/3 | up triangle |

Line width 1.5 pt (setting). Hue order alternates warm/cool so neighbours
differ in lightness as well as hue (blue/orange first, the safest CVD pair).
Plot background `bg.panel`; grid lines `border.subtle`; axis labels
`small`/text.secondary; cursor line `accent`.

### 3.9 egui `Visuals` mapping (summary)

```
dark_mode            = theme == Dark
panel_fill           = bg.panel        window_fill   = bg.popup
extreme_bg_color     = bg.input        faint_bg_color = bg.row.alt
code_bg_color        = bg.bar          hyperlink_color = text.accent
selection.bg_fill    = bg.selection    selection.stroke = 1 px accent
override_text_color  = None (set per widget state below)
window_stroke        = 1 px border.strong   window_corner_radius = 6
window_shadow        = Shadow { offset: [0,4], blur: 16, spread: 0, color: #000 @ 35 % dark / #0B1320 @ 18 % light }
popup_shadow         = same, blur 8
menu_corner_radius   = 4
widgets.noninteractive: bg_fill bg.panel, bg_stroke 1 px border.subtle, fg_stroke 1 px text.primary
widgets.inactive    : weak_bg_fill/bg_fill bg.button, bg_stroke 1 px border.strong, fg_stroke text.primary
widgets.hovered     : weak_bg_fill/bg_fill bg.button.hover, bg_stroke 1 px border.strong (light: #AEB7C2), fg_stroke text.primary, expansion 0
widgets.active      : weak_bg_fill/bg_fill bg.button.active, bg_stroke 1 px accent, fg_stroke text.primary, expansion 0
widgets.open        : like hovered
corner_radius (all widgets) = 4
text_cursor          = 2 px accent
striped tables       = true (uses faint_bg_color)
```
Set `expansion` to 0 for all states (no size jump on hover).

## 4. Spacing, sizes, radii, strokes

- Base unit 4. `item_spacing` = (8, 4). `button_padding` = (10, 4).
  `interact_size.y` = 26 (body controls). `indent` = 16 (tree).
- View content padding: 8 left/right, 6 top/bottom. Toolbar: height 34,
  padding 6/8, gap 6 between controls, 1 px `border.subtle` vertical
  separator (height 18) between groups, bottom border 1 px `border.subtle`.
- Row heights: **Compact 22** (default), **Comfortable 28**. Table header
  24 (compact) / 28. Tree indent 16 per level.
- Radii: widgets 4, pills/chips 9 (fully rounded at their height), dialogs
  6, tabs 4 top corners only, menus 4.
- Strokes: always 1 px; focus 1 px accent; plot lines 1.5.
- Scrollbars: `ScrollStyle::thin()`, bar width 8, handle `border.strong`,
  hover `text.disabled`, always floating.
- Minimum window size 1024 × 640.

## 5. Components and states

### 5.1 Buttons (height 26, padding 10/4, radius 4, label `body`, optional leading icon 16 pt with 6 gap)

| Kind | Normal | Hover | Pressed | Disabled | Focus |
|---|---|---|---|---|---|
| Primary (Connect, Apply, Save) | fill accent.fill, no stroke, text on_accent | accent.fill.hover | accent.fill.active | fill bg.button, text text.disabled, no stroke | + 1 px accent ring 1 px outside |
| Secondary (default) | fill bg.button, 1 px border.strong, text.primary | bg.button.hover | bg.button.active, 1 px accent | fill transparent, 1 px border.subtle, text.disabled | ring |
| Ghost / toolbar icon button (24×24, icon 16) | transparent, icon text.secondary | bg.button.hover, icon text.primary | bg.button.active | icon text.disabled | ring |
| Toggle icon button (Pause, Autoscroll) | as ghost; ON = fill accent.tint, icon accent | ON hover: accent.tint + 1 px accent | — | as ghost | ring |
| Danger (Write all, NMT all, LSS Activate bit rate) | fill danger.fill, text on_danger | dark `#B83232` (5.93) / light `#9E1E1E` (7.90) | dark `#A52B2B` / light `#881919` | as primary disabled | ring |
| Caution (actions that can take a device off the bus: LSS Set node-ID, LSS Set bit rate; also DCF Restore) | fill = warning tint (dark `#393326`, light `#F1EAE0`), 1 px warning border (dark `#745E33`, light `#CAB28C`), text + leading `warning` icon in `warning` colour (6.35 / 5.30) | dark `#433A28` (5.68) / light `#E9DECF` (4.77) | dark `#4D422A` (5.00) / light `#E6DBC9` (4.62) | as secondary disabled (no warning colour) | ring |

Both hover and pressed get darker in both themes, so white text keeps
≥ 4.5:1.

Disconnect is a secondary button with the `plugs` icon (not danger).
Every icon-only button has a tooltip with the action and its shortcut.

### 5.2 Inputs (TextEdit, ComboBox, DragValue)

- Height 26, fill bg.input, 1 px border.strong, radius 4, text `body` or
  `mono` for numeric/hex fields, hint text.secondary.
- Hover: border `text.disabled`. Focus: border accent (1 px), no glow.
- Error: border `error`, a 12 pt `warning-circle` icon in `error` at the
  right inside the field, and the message in `small`/error below or as a
  tooltip in tables.
- Disabled: fill bg.bar, border border.subtle, text text.disabled.
- Read-only value cells: no box, just the text.
- ComboBox: same box plus `caret-down` 12 pt in text.secondary at right.
- Filter box: leading `magnifying-glass` 14 pt, trailing `x` clear button
  when not empty.
- Checkbox 16×16, radius 3, unchecked fill bg.input + border.strong,
  checked fill accent.fill + white `check`. Radio same with circle.
- Segmented control (Hex | Dec | Text, Theme): one outlined group
  (1 px border.strong, radius 4, height 26), segments separated by 1 px
  border.strong; selected segment fill accent.tint, text accent; others
  text.secondary, hover bg.button.hover. Build with
  `selectable_value` in a horizontal frame.
- Slider: track 4 px bg.button, filled part accent, handle 12 px circle
  fill text.primary, 1 px border.strong.

### 5.3 Dock tabs (egui_dock `Style`)

- Tab bar: height 30, fill bg.bar, bottom line 1 px border.strong
  (`tab_bar.hline_color`). Tabs left-aligned, 2 px gap.
- Tab: padding 12 horizontal, title `body`, leading Phosphor icon 14 pt
  with 6 gap, close `x` 12 pt on the right (shown on the active tab and on
  hover; `show_close_buttons = true`).
- States (`tab.active`, `tab.inactive`, `tab.focused`, `tab.hovered`):

| State | Fill | Title | Outline |
|---|---|---|---|
| Inactive | transparent (bg.bar) | text.secondary | none |
| Hovered | bg.button.hover | text.primary | none |
| Active (leaf not focused) | bg.panel | text.primary | 1 px border.strong, top + sides (outline merges into the hline) |
| Focused (active tab of the focused leaf) | bg.panel | text.primary | 1 px border.strong, plus a 2 px accent line on the top edge (draw it in `TabViewer::on_tab_button` or as a custom painter; if that is not practical, use outline = accent) |
| Dragged tab ghost | bg.popup, shadow | text.primary | 1 px accent |

- Corner radius 4 top-left/top-right, 0 bottom.
- Tab context menu (right-click): Close, Close others, Split right, Split
  down, Move to new window (only if egui_dock windows are enabled; else
  hidden), Rename (trace/graph tabs only).
- Tab add button: hidden (views open from View menu).
- Overflow: scroll the tab bar (`tab_bar.fill_tab_bar = false`, egui_dock
  scroll arrows) .

### 5.4 Splitter (`separator`)

- Width 4 (the gap between leaves shows bg.frame), extra interact width 4.
- Idle: `color_idle` = border.strong drawn as a 1 px centred line.
- Hover: `color_hovered` = accent, cursor resize.
- Dragging: `color_dragged` = accent.
- Minimum leaf size 120 px.

### 5.5 Drop-zone overlay while dragging a tab

- `overlay.overlay_type = OverlayType::Widgets` (the five-target cross)
  plus highlight of the resulting area.
- Hovered target area fill: accent.tint (`overlay.selection_color`),
  outline 1 px accent, radius 4, inset 4 from the leaf edge.
- Cross buttons: 30 × 30, gap 4, radius 4, fill bg.popup, 1 px
  border.strong (`button_border_stroke`), inner icon = a small rectangle
  sketch showing the side (fill text.secondary, the target half in accent
  when hovered) (`button_color` text.secondary, hovered accent).
- Centre button = stack as tab. Tab-bar drop = insert at the caret
  position, shown by a 2 px accent vertical line between tabs.
- `overlay.feel.window_drop_coverage` etc. defaults; `fade_hold_time`
  default.
- Dragged tab follows the pointer as a ghost tab (bg.popup, 1 px accent,
  shadow) 70 % opacity.

### 5.6 Pills and chips

- Pill (node state, bus state): height 18, padding 8 horizontal, radius 9,
  fill = state tint, text `small` in the state colour, leading glyph 12 pt
  in the same colour, gap 4. No border (except Unknown).
- Chip (message type in trace): height 16, padding 6, radius 3, fill =
  type tint, text `mono.small` 11 in type colour, fixed width 52 so the
  column aligns.
- Count badge (filter count on a button): 16 high, radius 8, fill
  accent.fill, text on_accent `small`.

### 5.7 Tables

- Built with `egui_extras::TableBuilder` (striped, resizable columns,
  sticky header). Header fill bg.bar, text `small`/text.secondary, sort
  caret `caret-up`/`caret-down` 10 pt next to the active sort column.
- Column resize handle: 1 px border.subtle line in the header only.
- Grid: no vertical lines; horizontal lines off when striped.
- Hover row bg.row.hover; selected row bg.selection (+ 1 px accent
  outline only when the table has keyboard focus).
- Cell padding 8 horizontal.
- Truncate long text with `…` and show the full text as a tooltip.

### 5.8 Banners and inline messages

- Banner (top of a view, full width, height 28): tint fill of the semantic
  colour, 1 px semantic border bottom, icon 14 + text `body` in
  text.primary, optional action link button at the right, close `x`.
- Toast (bottom right of the window, above the status bar): bg.popup,
  1 px border.strong, radius 6, semantic icon 16 at the left, text
  `body`. Auto-hide after 4 s, except errors (they stay until closed).

### 5.9 Empty, disabled, error, not-connected states

| Situation | Treatment |
|---|---|
| View empty (no device, no frames) | Centred block in the view: icon 32 pt text.disabled, `heading` line in text.primary, one `body` line in text.secondary, one secondary button with the next action (e.g. "Add device", "Connect"). |
| Not connected | Connection bar bus-state pill = Unknown style with text "Not connected". Every bus action (Read, Write, NMT, Scan, Send, Start) is disabled; its tooltip says "Connect first". Object dictionary still shows the EDS objects; the Value column shows the EDS default value in text.secondary with the prefix `EDS ` (egui has no italic in a single Regular font). Trace view shows the empty state "No frames yet — connect to an adapter to start the trace" with a Connect button. |
| Listen-only | A pill `Listen only` in info tint next to the bus state; all transmit actions disabled with tooltip "Listen-only mode". |
| Disabled control | As in 5.1/5.2. Never hide a control that is only temporarily unavailable. |
| SDO error in a cell | Value cell shows the abort text in `error`, prefixed with `warning-circle`, e.g. `Abort 0x06010002 · read only`; full abort description in the tooltip. Row keeps its fill. |
| Timeout | Same as SDO error, text `Timeout (500 ms)`. |
| Bus off / error passive | Bus-state pill error / warning; a banner in the Trace view "Bus off — the adapter stopped sending. Reconnect" with a Reconnect link. |
| Lost node | Network row state pill Lost; "Last seen" in error colour. |
| Loading / scanning | Progress bar 4 px high under the toolbar (accent on bg.button) + text "Scanning 37 / 127" in the toolbar; a Stop button replaces Scan. |

## 6. App frame

Top to bottom (window 1280 × 800):

1. **Native window title bar** (OS). Title `CANoxide 0.5.0` (`CANoxide`
   and the version), the app icon.
2. **Menu bar** — height 28, fill bg.frame, bottom 1 px border.subtle.
   Items `File  View  Settings  Help`, `body`, padding 10, hover fill
   bg.button.hover radius 4. Right side: theme quick switch, a frameless
   glyph button (`desktop` = System, `moon` = Dark, `sun` = Light) that
   cycles System → Dark → Light → System; tooltip e.g. `Theme: System
   (click: Dark)`.
   - File: `Open EDS...` (Ctrl+O), `Save trace as...` (saves the first
     trace tab), `Import DBC...` (tooltip "Import the signals of a DBC file
     as user signals", 8.11), `Program download...` (tooltip "Download a
     program file to a node (CiA 302-3, 0x1F50)", 8.16), `Open data
     folder`; separator; `Exit`.
   - View: one check box per view, in this order: Devices, EDS library,
     Object dictionary, Network, Emergencies, Messages, Find signal, Graph,
     Dashboard, PDO configuration, Transmit list, LSS, DCF save / restore,
     EDS check, Drive (CiA 402); then `Trace 1` … `Trace 4`; separator;
     `New trace tab` (disabled at 4); separator; `Reset layout`; `Full
     screen` (F11).
   - Settings: opens the Settings dialog (Ctrl+,). It is a direct menu
     button, not a submenu.
   - Help: `Keyboard shortcuts` (writes the shortcut list into the status
     bar, section 10), `Copy display diagnostics` (tooltip "Window, screen
     and DPI values for a bug report, to the clipboard"), `About CANoxide`.
   - About CANoxide: modal 420 wide. The app icon 48 × 48, `CANoxide` in
     `title`, `Version x.y.z` mono, the line "A portable CANopen bench
     tool. Copyright (c) 2026 MechAkh. Licence: MIT.", the font and icon
     licences, the link `https://github.com/MechAkh/CANoxide`, button
     `Close`.
3. **Connection bar** — min height 30, fill bg.frame, bottom 1 px
   border.strong. While connected, the adapter, channel and bitrate
   controls are locked. Left to right:
   - `Adapter` label (small, secondary) + ComboBox 150 with the entries
     `Simulator`, `PEAK PCAN`, `CANfox`, `Kvaser`, `SLCAN (serial)`,
     `SLCAN (TCP)`, `Log file (replay)`.
   - The second label follows the adapter: `File` (log file), `Port`
     (SLCAN serial), `Address` (SLCAN TCP), else `Channel`. The control
     after it:

     | Adapter | Control |
     |---|---|
     | Simulator | Text `virtual bus`. |
     | PEAK PCAN | ComboBox 120: the available channels as `name (available)`, a separator, the label `Manual choice` and the PCAN-USB channels. With no list: `No available channel reported`. |
     | CANfox | ComboBox 150 (`CANfox net N`): the connected nets, then `Manual choice` with the default net. With no list: `No device list reported`. |
     | Kvaser | ComboBox 180: the channel list, then `Manual choice` `Channel 0` … `Channel 3`. With no list: `No channel list reported`. |
     | SLCAN (serial) | ComboBox 110 (`Choose a port` until one is chosen). With no port: `No COM port found`. |
     | SLCAN (TCP) | TextEdit 150, hint `host:port`, tooltip "The address of the SLCAN gateway, for example 192.168.1.10:2001"; warning text `not host:port` when the input does not parse. |
     | Log file (replay) | The file name (or `no file chosen` in secondary) and `Browse...` (`folder-open`, tooltip "A trace CSV written by CANoxide, a candump log, a PEAK .trc or a Vector .asc file"); file dialog `Log file to play back` (csv, log, txt, trc, asc). While the file loads: progress bar 110 `Loading n %`. |

   - PEAK PCAN, CANfox, Kvaser and SLCAN (serial) have a small refresh
     button `arrow-counter-clockwise` after the combo. It asks the driver
     for its list and sends nothing; tooltips: "Ask the driver which
     channels are available (sends nothing)" (PEAK), "... which CANfox
     nets are connected (opens no net, sends nothing)", "... which
     channels exist (opens no channel, sends nothing)" (Kvaser), "List the
     COM ports of this PC again (opens no port, sends nothing)" (SLCAN).
   - A channel note, if any, in warning colour.
   - `Bitrate` + ComboBox 110.
   - Checkbox `Listen only` (tooltip "Open the channel in listen-only
     mode; every send control is disabled").
   - Primary button `Connect` (accent fill, no icon), min width 110,
     tooltip "Opens the channel only; nothing is sent"; when connected it
     becomes a plain `Disconnect` (min width 110).
   - Send button `Scan nodes` (tooltip "SDO read of 0x1000 on nodes a..b,
     one at a time"); it starts the scan and opens Network. While the scan
     runs: a progress bar 90 `done / total` and `Cancel scan`.
   - Right-aligned group: bus-state pill (`Not connected` secondary on no
     fill; connected: `Error active`, or `Bus: <status>` in error colour
     for bus off / connection lost, warning for error passive / warning,
     else success); an info pill `Replay: nothing is sent` or `Listen
     only` when it applies; `Load` label + a 60 × 6 meter (fill bg.button,
     value text_accent; > 70 % warning, > 90 % error) + `12.4 %` mono;
     `fr/s` label + the frame rate mono.
4. **Replay transport bar** — only during a log replay, under the
   connection bar (8.12).
5. **Dock area** — `DockArea` filling the rest, background bg.frame (shows
   in the 4 px gaps), 4 px margin to the window edge left/right.
6. **Status bar** — height 24, fill bg.frame, top 1 px border.subtle,
   `small` text.
   - Left: connection summary: `Replay · <file> · nothing is sent`,
     `Connected · <adapter> · <channel> · <bitrate>` (with ` · listen only`
     when it applies; the channel is `virtual bus`, `net N`, `channel N`,
     the COM port, `host:port` or the PCAN channel name), or `Not
     connected. Nothing is sent until you press a button.`
   - Then a separator and the last action result with a semantic icon
     (`check-circle` text.primary / `warning-circle` error), e.g. `SDO
     read 6064:00 = 100 000 (2 ms)`; long texts are truncated.
   - Right: a frameless button `folder-open <folder name> (<mode>)` that
     opens the data folder (tooltip `Data folder (<mode>): <path>`);
     `Trace n % of <capacity>`; while the SDO queue is not empty, `SDO n
     iiii:ss done / total bytes` or `SDO queue q` and a small `Cancel SDO`
     button.
7. **Confirm dialog** (used by every "asks first" in this file): modal,
   max width 420, heading, wrapped text, the action button (danger or
   accent fill) first, then `Cancel`. Cancel writes `Cancelled; nothing
   sent` into the status bar.

## 7. Default dock layouts

Chosen on first start (no saved layout) by window inner width:
< 1600 → layout A; ≥ 1600 → layout B. The layout is saved (serde) in
`layout.json` in the data folder on exit and restored on start; a file
that cannot be read gives a note and the default layout. View ▸ Reset
layout rebuilds the default for the current width. `*` = active tab of
the leaf.

### Tabs

| Tab | Title | Icon | Open by default |
|---|---|---|---|
| Devices | `Devices` | `cpu` | yes |
| EDS library | `EDS library` | `books` | yes |
| Object dictionary | `Object dictionary` | `list-magnifying-glass` | yes |
| Network | `Network` | `graph` | yes |
| Trace | `Trace 1` … `Trace 4` | `list-dashes` | Trace 1 only |
| Messages | `Messages` | `table` | yes |
| Transmit list | `Transmit list` | `paper-plane-tilt` | yes |
| Graph | `Graph` | `chart-line` | yes |
| PDO configuration | `PDO configuration` | `sliders-horizontal` | yes |
| LSS | `LSS` | `identification-card` | no |
| DCF save / restore | `DCF save / restore` | `file-code` | no |
| Find signal | `Find signal` | `magnifying-glass` | no |
| Emergencies | `Emergencies` | `warning` | no |
| EDS check | `EDS check` | `check-square` | no |
| Drive (CiA 402) | `Drive (CiA 402)` | `engine` | no |
| Dashboard | `Dashboard` | `gauge` | no |

The tab title is `<icon>  <name>`.

### Layout A (1280 × 800)

```
Horizontal
├── 0.21  Leaf [Devices*, EDS library]
└── 0.79  Vertical
    ├── 0.56  Leaf [Object dictionary*, Network, PDO configuration]
    └── 0.44  Horizontal
        ├── 0.58  Leaf [Trace 1*, Messages, Transmit list]
        └── 0.42  Leaf [Graph*]
```
Approximate pixel sizes in the 1272 × 710 dock area: Devices 264 wide;
OD 1000 × 395; Trace 580 × 311; Graph 420 × 311.

### Layout B (1920 × 1080)

```
Horizontal
├── 0.15  Leaf [Devices*, EDS library]
└── 0.85  Horizontal
    ├── 0.60  Vertical                          (= 0.51 of the window)
    │   ├── 0.55  Leaf [Object dictionary*, PDO configuration]
    │   └── 0.45  Leaf [Trace 1*, Messages, Transmit list]
    └── 0.40  Vertical                          (= 0.34 of the window)
        ├── 0.40  Leaf [Network*]
        └── 0.60  Leaf [Graph*]
```

### Building it with egui_dock

`egui_dock` splits are binary. In egui_dock 0.16, the `fraction` of
`split_left/right/above/below(parent, fraction, tabs)` is the share of the
first child (left or top): with `split_left` that is the **new** node,
with `split_right` and `split_below` the **existing** node. Layout A:

```rust
let mut dock = DockState::new(vec![Tab::Od, Tab::Network, Tab::Pdo]);
let s = dock.main_surface_mut();
let [main, _devices] = s.split_left(NodeIndex::root(), 0.21, vec![Tab::Devices, Tab::Eds]);
let [_od, bottom]    = s.split_below(main, 0.56, vec![Tab::Trace(1), Tab::Messages, Tab::TxList]);
let [_trace, _graph] = s.split_right(bottom, 0.58, vec![Tab::Graph]);
```
Layout B:
```rust
let mut dock = DockState::new(vec![Tab::Od, Tab::Pdo]);
let s = dock.main_surface_mut();
let [mid, _dev]      = s.split_left(NodeIndex::root(), 0.15, vec![Tab::Devices, Tab::Eds]);
let [mid, right]     = s.split_right(mid, 0.60, vec![Tab::Network]);
let [_od, _trace]    = s.split_below(mid, 0.55, vec![Tab::Trace(1), Tab::Messages, Tab::TxList]);
let [_net, _graph]   = s.split_below(right, 0.40, vec![Tab::Graph]);
```

### Where a closed tab opens

The tabs that are closed by default are commissioning or analysis work,
not everyday monitoring. A View menu check box opens a tab as follows:

- A tab that is open already comes to the front.
- LSS, Emergencies and Drive (CiA 402): a new tab in the leaf that holds
  **Network**.
- DCF save / restore and EDS check: in the leaf that holds **Object
  dictionary**.
- Dashboard: in the leaf that holds **Graph**.
- Trace n, Messages and Find signal: in the leaf of the first open trace
  tab.
- Otherwise, or when that leaf no longer exists: a new tab in the largest
  leaf; with no leaf at all, a new root.

Once opened or moved, the saved layout keeps a tab where the user put it.

### Tab rules

- Every view is closable. Tabs cannot be torn out into windows.
- Trace tabs: up to 4 (`Trace 1` … `Trace 4`), each with its own filter,
  mode and pause state. The name is fixed. The 4th disables `New trace
  tab`.
- Tab content has a margin of 8 × 6; the dock adds no scroll bars, each
  view scrolls itself.
- A tab title carries ` •` after the name when the view needs attention:
  a paused trace tab, a PDO configuration with pending changes.

## 8. Views

Column widths are starting widths in px; `flex` = takes the rest
(`Column::remainder()`), min given in brackets. All tables are resizable
unless noted. Sections 8.1 to 8.9 keep their numbers (the code cites
them); the views added later follow from 8.10.

### 8.1 Devices (and EDS library)

- Toolbar: `+ Add` (secondary, icon `plus`), `Edit` (ghost `pencil-simple`),
  `Remove` (ghost `trash`); right: filter icon button.
- List rows 40 high (two lines): left a node-ID badge (28 × 20, radius 4,
  bg.button, `mono` `01`), then line 1 `body` name (`Motor drive`), line 2
  `small` secondary EDS file name (`drive_generic.eds`); right a state dot
  8 px with the node-state colour + glyph tooltip. Selected = bg.selection.
  The selected device is the "active device" of the Object dictionary and
  PDO views.
- Drop target: dropping `.eds/.dcf` on the window shows a full-window
  accent.tint overlay with "Drop to add EDS files".
- EDS library tab: table File (flex [140]), Vendor (100), Product (120),
  Rev (60), Used by (60, count). Toolbar: `Add…`, `Remove`, filter.
- Empty: icon `cpu`, "No devices yet", "Add a device with its node ID and
  EDS file.", button `Add device`.

### 8.2 Object dictionary

- Toolbar (left → right): strong `<device>, node n` (or `No active
  device, node n`); filter box 140 (hint `index or name`); `Values` with
  selectable `Hex | Decimal | String`; icon button `download-simple`
  `Read all 0xIIII` (reads the selected object; `Read all (select an
  object)` without a selection); icon buttons `caret-double-down` `Expand
  all` and `caret-double-up` `Collapse all`; warning text `listen only:
  SDO disabled` or `not connected` when no SDO can be sent.
- Collapsing `Manual SDO` (open by default): `Node n`, Index `0x` (50),
  Sub (36), Type combo (`raw bytes` and the data types) with a small `Type
  from EDS`, Value (140, hint `dec, 0x hex, text`), `Read`, `Write` (asks
  first: `Write object`); separator; `Transfer` combo 100 (`auto`,
  `segmented`, `block`), `Upload to file...`, `Download from file...`;
  a warning `index: hex 0..FFFF, sub: 0..255` for a bad address; the last
  result text.
- While an SDO file transfer runs: progress bar 200 `SDO node n
  0xIIII:SS: done / t bytes` and `Cancel transfer`.
- Table, inside a horizontal scroll area (when the columns need more than
  the view, the whole table scrolls sideways, so that the action column is
  never cut off; when the view gets narrower and the columns no longer
  fit, the widths start again from the start values). Start widths follow
  the view width (`rest` = view width minus the fixed columns, at least
  420):

  | Column | Start width | Content |
  |---|---|---|
  | Index | 76 [60] | object row: tree caret (`caret-right` / `caret-down`, frameless, named Expand / Collapse) + selectable `IIII`; sub row of a VAR: selectable `IIII`, else weak `IIII` |
  | Sub | 30 [26] | object row: the number of subs; sub row: `SS` |
  | Name | flex [80] | object row: strong name; sub row: name, tooltip the full name |
  | Type | 13 % of rest [56] | object row: object type; sub row: data type name |
  | Access | 48 [36] | the access label |
  | Value | 22 % of rest [70] | mono value in the chosen format |
  | Default | 13 % of rest [50] | the EDS default |
  | New value | 18 % of rest [64] | TextEdit (hint `dec / 0x hex`) for a writable entry |
  | (actions) | fixed, measured from its two buttons | object row: icon button `download-simple` `Read all`; sub row: `download-simple` `Read` and `upload-simple` `Write` |

- The action buttons are always shown; Read is enabled for a readable
  entry and Write for a writable one, both only while SDO can be sent.
- Value states: unread `—` in text.disabled (tooltip `not read yet`);
  reading `...` (weak); read OK mono text.primary with the value as
  tooltip; error text in error colour.
- Empty (no EDS): icon `list-magnifying-glass` 32, "No object dictionary.
  Select a device with an EDS file in the Devices view, or add an EDS file
  in the EDS library." and "The manual SDO box above works without an
  EDS."

### 8.3 Network

A vertical scroll of groups. At the top, a warning line when no command
can be sent: `Listen only: every send control is disabled.` or `Not
connected: send controls are disabled.`

- **NMT** (strong):
  - Row `Node n` (label 72) + buttons `Start`, `Stop`, `Enter
    pre-operational`, `Reset node`, `Reset communication` for the active
    node; separator; `Program download...` (tooltip "Open the program
    download wizard for this node (CiA 302-3); nothing is sent yet",
    8.16).
  - Row `All nodes` + the same five commands, sent to node 0. Each asks
    first (`NMT to all nodes`, "Send NMT X to all nodes (node 0)?");
    `Reset node` and `Stop` use the danger action button.
  - Collapsing `More` (closed): `Send SYNC`; on the simulator also
    `Simulator EMCY code 0x` [48] `reg 0x` [28] `Trigger EMCY`.
- **Parameters** (strong), objects 0x1010 / 0x1011:
  - `Node n` + sub combo 190. The entries are the subs that the node
    reported (else 1 to 3), shown as `<name> (sub s)` with the names `All
    parameters`, `Communication parameters`, `Application parameters`,
    `Manufacturer specific`.
  - Caution button `Store (0x1010)` (writes `save`); it asks first
    (`Store parameters`) when the confirm setting is on.
  - Danger button `Restore defaults (0x1011)` (writes `load`); it always
    asks first (`Restore default parameters`: the saved parameters are
    lost, the defaults are active after a reset node or a power cycle).
  - Send button `Read supported` (tooltip "Read 0x1010 and 0x1011 (sub 0
    and each sub) by SDO"); then the small lines `0x1010: ...` and
    `0x1011: ...` with `on command`, `autonomous` or `not supported` per
    sub, and a note line.
- **Network overview** (strong):
  - `Refresh node info` (tooltip "Read 0x1000, 0x1001, 0x1018 and 0x1008
    of every known node by SDO"); `Heartbeat of all nodes` DragValue 0 …
    65 000 ms (default 1000) + `Set heartbeat` (writes 0x1017 on every
    known node, asks first: `Heartbeat of all nodes`); `Cancel` while a
    task runs.
  - Table (horizontal scroll): Node (44), State (150, pill with the state
    glyph), Last seen (80, `x.x s ago`), Device type (110, `0x%08X`), Err
    reg (60, `0x%02X`, error colour when not zero), Identity (170, vendor
    / product / rev / serial), Name (120, tooltip with read errors), Last
    EMCY (flex, warning colour `0xCODE text`).
  - Empty: icon `graph` 28, "No node seen yet. Nodes appear when they
    send a heartbeat, an EMCY or an SDO answer." and "Scan nodes or
    Refresh node info read them by SDO."
- **Scan nodes** (strong):
  - "SDO read of 0x1000 on nodes a..b, one at a time, timeout" +
    DragValue 20 … 2000 ms; `Scan nodes` (or `Cancel` while it runs) and
    a progress bar `done / total`.
  - Result grid per answering node: `Node n`, `device type 0x... (profile
    p)` or `answered: <error>`, then `Device: <name>` for a known device
    or a small `Add as device` button (tooltip "Open Add device with this
    node"; opens the device dialog with the node, the name and the
    selected EDS).

### 8.4 Trace (up to 4 tabs)

- Toolbar (wraps when narrow): icon toggle `Pause` (`pause` / `play`,
  tooltip "Pause: stop adding rows to this trace tab"), icon toggle
  `arrow-line-down` `Auto-scroll`; separator; combos Display mode
  (`Scroll | Fixed`, 64; Fixed = one row per COB-ID updated in place),
  Time format (`Absolute | Delta`, 76), Data format (`Hex | Decimal |
  String`, 70); separator; icon button `trash` `Clear` (tooltip "Clear the
  buffer of every trace tab"), `floppy-disk` `Save trace...`, `plus` `New
  trace tab` (at most 4); separator; `funnel  Filters` with a count badge;
  right: small counters `n shown, m in buffer (max c), r received` and,
  while paused, the warning `paused, n frame(s) not recorded`.
- `Save trace...` (tooltip "Save the rows of this tab as CSV, PEAK .trc
  (version 1.1) or Vector .asc"): file dialog with the name `trace.csv`
  and the filters CSV, PEAK trace (version 1.1), Vector ASC; the
  extension picks the format.
- Filters popup (min width 300): `Node ID` [48, hint `all`], `COB-ID 0x`
  min .. max (70 each), `Text` [180], `Types (n/N)` with `All` / `None`
  and a 3-column grid of type check boxes, `Reset filters`.
- Columns, Scroll mode: Time (`s`, or Delta in `ms`), Dir, COB-ID, DLC,
  Data, Type, Node, Decoded. Fixed mode: COB-ID, Dir, Count, Cycle `ms`,
  DLC, Data, Type, Node, Decoded.
- Rows: neutral fill with zebra; chips carry the type colour; ERR and
  EMCY rows get the row tint (3.5). Selected row bg.selection. The
  Decoded cell has one tooltip with the full decode.
- Up to the buffer size: use `TableBuilder::body().rows()` (virtualised).
- Empty: icon `list-dashes` 32, `No frames yet.` when connected, else `No
  frames. Connect to see the bus traffic.`
- Auto-scroll: on = the view follows the newest row; a user scroll up
  (wheel, drag, Arrow/Page Up, Home with the pointer over the table)
  leaves the bottom, a scroll back to the end (or End) follows again. Off =
  the top visible row stays anchored by its sequence number, also while
  the ring buffer drops old rows.

### 8.5 Graph

- Toolbar: `plus` `Add signal...`; separator; `stop` `Stop polling` /
  `play` `Start polling`; toggle `pause` `Pause` ("Freeze the picture; the
  signals keep recording"); toggle `arrow-line-right` `Follow` ("Follow the
  newest points (off: drag and zoom; a click, drag or scroll in the plot
  switches it off)"); `magnifying-glass-plus` Zoom in and
  `magnifying-glass-minus` Zoom out ("also Ctrl + scroll in the plot;
  switches Follow off"); separator; `clock` + time window DragValue 1 …
  3600 s; separator; `trash` `Clear` ("Delete the recorded points");
  `floppy-disk` `Export CSV...` (`signals.csv`); `export` `Export
  range...`; pill `polling` while polling runs.
- Hint line: `SDO signals record only while polling runs.` and the send
  hint.
- Plot: `egui_plot::Plot`, x label `time (s)`, the egui legend.
- Legend grid under the plot: swatch (14), `Show`, `Signal` (`name
  [unit]`), `Source`, `Last`, `Min`, `Max`, `Points`, `Remove`.
- `Export range...` window: `From` / `To` (s), `Visible range`, the small
  note "CSV: time_s, then one column per shown signal; one row per time
  stamp, an empty cell where a signal has no sample.", `Save...`
  (`signals_range.csv`), `Cancel`.
- Add signal window (movable, over a scrim of black alpha 90; Escape
  closes it). Source: `PDO object | SDO poll | Raw frame bits | User
  signal`.
  - PDO object / SDO poll: Node 1 … 127; `Mapped object` combo 260
    (`TPDOn iiii:ss name`, TPDOs first, `(choose)`; a small note on
    RPDOs); Index `0x` [60]; Sub (hex); Period ms (SDO poll only).
  - Raw frame bits: COB-ID `0x` [80] + `Ext`, Start bit 0 … 63, Bits 1 …
    64, Type `Unsigned | Signed | Float`, check box `Big endian`.
  - User signal: a combo of the user signals, or the note "None yet:
    define one in the Messages view (select bits, Define signal)."
  - All sources: Name (hint `automatic`), Unit [60], Scale, Offset.
  - SDO note "An SDO signal sends a read request each period, only while
    polling runs."
  - `Add`, `Cancel`.
- The same window adds a widget to the Dashboard (8.18).
- Empty: icon `chart-line` 28, "No signals." and "Press Add signal to plot
  a PDO object, an SDO poll or raw frame bits."

### 8.6 PDO configuration

- Toolbar: `Node n`; send button `download-simple` `Read PDOs` (tooltip
  "Read 0x1400..0x1BFF over SDO"); `Save to library`, `Load from library`
  (tooltip "Use the saved configuration for decoding (sends nothing)");
  separator; `Disable all TPDO`, `Enable all TPDO`, `Disable all RPDO`,
  `Enable all RPDO`.
- While reading: progress bar 200 and `Cancel`; then the last status
  line, `Read <time>` (`, incomplete` when some reads failed) and the read
  errors in warning colour.
- Error banner after a failed step: "X was / were left disabled on the
  device after a failed step: <error>" with the action `Read PDOs`.
- Table: PDO (64), COB-ID (70, hex; an `x` prefix marks an extended ID),
  State (76, pill `enabled` / `disabled` / `left disabled`), Transmission
  (120), Inhibit (70, `x.x ms`), Event (70, `n ms`), Mapping (260, `n bit:
  ...`), last column `Edit` and `Disable` / `Enable`.
- Editor under the table (after `Edit`): heading `TPDOn of node n` + pill
  `pending` while changes wait; grid `COB-ID 0x` [90], `Enabled` check
  box `valid (bit 31 clear)`, `Transmission type` 0 … 255, `Inhibit time`
  (`x 100 us`), `Event timer` (ms); strong `Mapping (n bit of 64)` with one
  line per entry `nn  iiii:ss  nn bit` and `arrow-up` / `arrow-down` /
  `Remove`; an add field [120, hint `6000:1:8`] + `Add entry`; error text;
  `Apply`, `Revert`, `Close`.
- PDO check, under the editor, updated while you edit: `PDO check: no
  finding`, or strong `PDO check: n error(s), m warning(s)` and one line
  per finding in error or warning colour (`error: entry i: <message>` or
  `<severity>: <message>`). It checks: length over 64 bits; more than 64
  mapping entries (more than 8 with byte mapping); an entry of length 0;
  an object that is not in the EDS; a length that does not match the
  EDS; an object with PDOMapping=0; the access (a TPDO object must be
  readable, an RPDO object writable); a COB-ID in a range that CiA 301
  restricts; the default COB-ID of another node; a duplicate COB-ID;
  transmission type 241 … 251 (reserved); an RTR-only type on an RPDO.
- Apply asks first (`Apply PDO change`). With an error it always asks
  (`PDO mapping has errors`): the findings, "The node will likely refuse
  the change, or it breaks the network.", the number of SDO writes, and
  the danger button `Write anyway`.
- A failed apply shows the modal `PDO change not complete` with the step
  lines, "Read the PDOs again to see the state of the node." and `Close`.
- Empty: icon `arrows-left-right` 28, "No PDO configuration of node n."
  and "Press Read PDOs (connected), or Load from library."

### 8.7 Transmit list

- Toolbar: `+ New frame`, `Duplicate` (`copy`), `Remove` (`trash`),
  separator, `Start all periodic` (`play`), `Stop all` (`stop`).
- Columns: Name (flex [140], inline editable), COB-ID (72, mono), DLC
  (40), Data (196, mono editable, 2 hex chars per byte), Trigger (130,
  combo `Manual`, `Periodic`, `On receive`), Period / Condition (120:
  ms DragValue for Periodic; `on 0x181` mono for On receive; `—` for
  Manual), Count (64, mono sent count), Action (88: Manual → `Send`
  secondary button with `paper-plane-right`; Periodic/On receive →
  toggle `Start`/`Stop`; running = success pill `Running` inline left of
  the button).
- Empty: icon `paper-plane-tilt`, "No frames", "Create a frame to send it
  by hand, periodically, or on receive.", button `New frame`.

### 8.8 LSS (CiA 305 layer setting services)

Purpose: configure node-ID and bit rate of devices that support LSS. One
view, no tabs inside; content in a vertical scroll of **groups** (a group
= `heading` title, 1 px border.subtle under it, 12 gap, content rows of
height 34). When the leaf is ≥ 900 px wide, groups 1–2 sit in a left
column (440) and 3–5 in a right column (flex); below 900 one column.

- **Toolbar**: LSS state pill (left): `Waiting` (Stopped style, glyph
  `pause`), `Configuration (global)` (info tint, glyph `wrench`),
  `Configuration (selective)` (info tint, glyph `crosshair`); then the
  **global switch** segmented `Waiting | Configuration` (switch state
  global); separator; small secondary text `LSS master · timeout 1000 ms`.
  Right: ghost `Clear log` (`trash`).
- **Not connected / listen-only**: a banner at the top (warning tint):
  "LSS needs a connected adapter with transmit on." or "Listen-only: LSS
  services are off."; every control below is disabled (fields too), the
  last values stay visible.
- **Group 1 — Identity** (CiA 305 LSS address):
  - Four rows, label 110 (`body`) + mono TextEdit 120 with `0x` prefix
    (8 hex digits): `Vendor ID`, `Product code`, `Revision`, `Serial`.
    Invalid hex → input error state.
  - Buttons row: `Select` (primary, icon `crosshair`, tooltip "Switch
    state selective: only the device with this identity enters
    configuration"), `Inquire` (secondary, icon `magnifying-glass`,
    tooltip "Read identity and node-ID of the device in configuration
    state"; fills the four fields and the "Current node-ID" field).
  - Fields filled by Inquire or Fastscan flash bg.row.hover once (600 ms).
- **Group 2 — Fastscan**:
  - Row: `Fastscan` (secondary, icon `magnifying-glass-plus`), then a
    progress bar (flex, 6 high, accent on bg.button) with the text
    `Bit 74 / 128 · Serial` (mono.small) on its right, then `Cancel`
    (secondary, visible only while running; Fastscan button disabled
    while running).
  - Option checkboxes (small): `Vendor ID known` / `Product code known` /
    `Revision known` (each uses the value in group 1 to shorten the scan).
  - Result: "Found 1 device → identity filled in, device is in
    configuration state" (success icon) or "No unconfigured device
    answered" (warning icon).
- **Group 3 — Node-ID**:
  - `Current node-ID` read-only mono (from Inquire), `—` when unknown.
  - `New node-ID`: DragValue 1 … 127 (mono, width 72) + checkbox
    `Unconfigured (255)` which disables the DragValue and sends 255.
  - Button `Set node-ID` (**caution** style, icon `warning`).
  - Caution note under the row (`small`, warning colour, glyph
    `warning`): "The device answers on the new node-ID after a reset;
    it may leave this network configuration."
- **Group 4 — Bit rate**:
  - `New bit rate` combo (CiA 305 table: 1000, 800, 500, 250, 125, 50,
    20, 10 kbit/s; index shown in small secondary `index 3`) width 150.
  - Button `Set bit rate` (**caution**).
  - `Switch delay` DragValue ms 0 … 65 535 (mono, width 88, default from
    settings) and button `Activate bit rate` (**danger** style, icon
    `lightning`), tooltip "All devices in configuration state switch to
    the new bit rate after the delay; the adapter follows".
  - Caution note: "Devices on another bit rate leave this bus. Set the
    adapter bit rate to match afterwards." (the app offers to reconnect
    at the new rate after Activate).
- **Group 5 — Store**: `Store configuration` (secondary, icon
  `floppy-disk`), tooltip "Device stores node-ID and bit rate in
  non-volatile memory".
- **Log** (bottom, full width, min height 140, resizable through the dock
  only): table Time (96, mono), Service (170, e.g. `Switch state
  selective`, `Configure node-ID`, `Configure bit timing`, `Activate bit
  timing`, `Store configuration`, `Inquire identity`, `Fastscan`),
  Request (150, mono, e.g. `node-ID 5`, `index 3`, `delay 100 ms`),
  Result (100, pill: `OK` success, `Error 1` error, `Timeout` error,
  `No answer` warning, `Sent` info for unconfirmed services such as switch
  global and activate), Detail (flex [160], `table`, e.g. "Node-ID out of
  range", "Not supported"). Newest row at the bottom, auto-scroll.
- Confirmation (Safety setting on) for Set node-ID, Set bit rate and
  Activate: modal 420 wide, warning icon, text "Set node-ID 5 on the
  selected device? It may leave this network.", buttons `Cancel`
  (secondary, default focus) and the action (caution / danger style).
- Empty state (no adapter ever connected): not used — the view always
  shows the form; the not-connected banner replaces it.

### 8.9 DCF save / restore (node configuration)

Purpose: save a node's object values to a DCF, restore them, compare.

- **Toolbar**: active device combo (`Node 1 · Motor drive`, 200);
  segmented `Save | Restore | Compare` (mode); separator; file field
  (mono `small`, read-only, flex [200], shows the file name, full path in
  tooltip, hint `No file chosen`) + `Browse…` (secondary,
  `folder-open`); separator; the mode's action button:
  - Save: `Save to DCF` (primary, icon `download-simple`).
  - Restore: `Restore` (**caution** style, icon `upload-simple`; with
    Dry run on it becomes secondary `Dry run`, icon `eye`).
  - Compare: `Compare` (primary, icon `git-diff`).
- **Options row** (height 34, bg.bar, only in Restore mode; in Save mode
  only the index range): checkboxes `Dry run` (default off),
  `Continue on error` (default on), `Store parameters (0x1010)`
  (default **off**; when ticked shows a caution note "Writes 0x1010:01
  'save' after the restore; values survive a power cycle"); `Index range`
  two mono fields `0x1000` – `0x9FFF` (width 72 each).
- **Progress strip** (height 30, shown while running, under the
  options): progress bar (flex, 6 high), text `Reading 214 / 388 ·
  3 failed` (Save) / `Writing 120 / 388 · 1 failed` (Restore) /
  `Comparing 300 / 388` in mono.small, `Cancel` secondary. After the
  run the strip shows the summary with a semantic icon and the elapsed
  time, e.g. `✓ Restore finished in 12.4 s · 201 written, 160 unchanged,
  24 skipped, 3 failed` (warning icon when failed > 0).
- **Result filter** (row under the strip, height 30): chip-buttons, each
  `label count`, toggle style (selected = accent.tint + accent text):
  `All 388`, `Written 201`, `Unchanged 160`, `Skipped 24`, `Failed 3`.
  In Save mode: `All`, `Read`, `Failed`. In Compare mode: `All`,
  `Different`, `Same`, `Only in file`, `Only on device`. Default after a
  run with failures: `Failed`.
- **Result table (Save / Restore)**: Index (64, mono), Sub (40, mono),
  Name (flex [180]), Result (110, pill), Value in file (130, mono), Value
  on device (130, mono), Detail (220, `table`, abort text in error colour,
  reason in text.secondary).
  Result pills: `Written` success, `Read` success, `Unchanged` Stopped
  style (neutral), `Skipped` info (reason in Detail: "read only",
  "outside index range", "not in device", "dry run"), `Failed` error
  (Detail: `Abort 0x06090030 · value range exceeded`).
- **Compare table**: Index, Sub, Name, In file (130, mono), On device (130,
  mono), Status (110: `Different` warning pill, `Same` neutral, `Only in
  file` info, `Only on device` info). A Different row's On-device cell uses
  pending.fill + pending.border. Context menu on a row: `Restore this
  value`, `Copy`.
- **Empty states**: Save mode before a run: icon `file-code`, "Save the
  node's configuration", "Reads every readable object of node 1 into a DCF
  file.", button `Save to DCF`. Restore/Compare without a file: "Choose a
  DCF file", button `Browse…`.
- Not connected / listen-only: Save, Restore and Compare disabled with
  the tooltip; Browse and the file field stay enabled; banner as in LSS.
- Confirmation (Safety setting on) for Restore (not for Dry run): modal
  "Restore 388 objects from motor_drive_2026-11-19.dcf to node 1?" with
  the options repeated as text, `Cancel` and `Restore` (caution).

### 8.10 Messages

- Dock tab `Messages` (`table` icon), in the trace leaf by default; one
  row per COB-ID (11-bit and 29-bit apart, at most 2048 IDs).
- Toolbar: filter box 200 (hint `Filter: COB-ID, type, node`), `Sort by`
  combo (`COB-ID | Count | Age`); separator; selectable `Hex | Decimal`
  for the data bytes (saved in the settings); `Mute noisy bytes` (tooltip
  "No tint on a byte that changes in nearly every frame (for example a
  counter)"); `Reset all statistics`; `Import DBC...` (8.11); ID counter
  `n IDs`; a warning `n frames of further IDs not counted (limit 2048)`
  when the ID limit is reached.
- Columns: COB-ID (76, mono), Type (110, chip, 3.5), Node (40), Count
  (70), Cycle (72, mean period, ms), DLC (34), B0..B7 (32 each, mono, hex
  or decimal), Age (64, s since the last frame), Decoded (flex).
- Change tint: a changed byte cell gets `change_tint` (value up = blue,
  value down = red), mixed into the row background and fading to it over
  the fade time (Settings, default 2 s). Text keeps AA contrast at every
  step. With `Reduce colour`, a caret up / down mark is drawn instead.
  Repaint at 30 Hz only while a tint fades.
- Empty: "No frames yet. Connect to see one row per COB-ID."
- Bit view: a panel of the tab for the selected row, resizable. Position
  `Bottom` (default) or `Top`: a bottom / top panel, default height 460,
  min 120. `Left` or `Right`: a side panel, default width 560, min 260, max
  the tab width less 200; the table then scrolls sideways in the rest. The
  position is saved in the settings (`msg_detail_pos`). A click on the
  selected row again closes the bit view.
  - Header row: `x` `Close` (tooltip "Close the bit view (also: click the
    selected row again)"), four selectable icons `align-bottom`,
    `align-left`, `align-right`, `align-top` (16 pt; tooltip "Where the bit
    view is shown: bottom" etc.), strong `<id> <label> node n`, `n frames,
    m compared`, `Reset statistics` (row; keeps the last data); separator;
    the selection text, or "Drag over bits or click a byte or a bit
    (Shift: extend) to select" (tooltip on Shift + click); `LE | BE`; `Add
    to Graph` (adds a Raw-bits signal; status message, no tab switch),
    `Define signal...`, `Find similar bits` (one bit only).
  - Selection: a drag over the cells selects bits, also across byte rows.
    A click selects one bit. Shift + click on a bit extends the selection
    from its anchor (the first selected bit) to that bit; Shift + click on
    a `Byte n` button extends it to whole bytes from the anchor byte to
    that byte, in either direction. LE runs in bit order (start = the
    lowest bit), BE in grid reading order (start = the first cell, the
    MSB; text `Selected: bit s (MSB), l bit, big endian`). `Define
    signal...` opens filled in with the start bit, length and byte order.
  - The grid and the value list scroll (both ways) in the upper part of
    the panel; the history keeps at least 100 of the height (the upper part
    gets at least half).
  - Bit grid: header `Bit` 7..0 and `Hex` (or `Dec`); cells 24 × 20; rows
    start with a button `Byte n` (56) that selects the whole byte. Cell
    fill `bit_heat` by toggle rate, with `bit_flash` over it: a bit that
    changes is filled and then fades, and so is the Hex cell of its byte.
    Outlines: 1.5 px in the series colour of each PDO-mapped signal; an
    inner 1.5 px outline in the colour of each user signal (inside the PDO
    outline, so both show); 2 px text_accent for the selection. Tooltip:
    `Byte b bit i (bit k): v`, `Shift + click: extend the selection to
    this bit` (small), `Toggled t times in n compared frames (p %)`, and
    `Mapped: ...` / `Signal: ...` when they apply. A frame with DLC 0 shows
    `No data bytes`.
  - Value list right of the grid: `User signals` (small secondary); each
    line a 3 × 14 text_accent bar when the signal has bits in the selection
    (tooltip "This signal has bits in the selection"), a 10 pt colour
    square, `name = value unit`, an 84 × 16 sparkline, small buttons
    `Edit`, `Delete` and, for a 1-bit signal, `Similar bits`; then the
    mapped list, each line also with a sparkline, or `No known PDO mapping
    for this COB-ID.` Each user and mapped line ends with two icon buttons:
    `chart-line` (tooltip "Add this signal to the Graph"; the same source
    as the Add signal picker; a signal that the Graph already shows is not
    added again, status "Signal x is already in the Graph") and `gauge`
    (tooltip "Add this signal to the Dashboard (a new widget)"; adds the
    signal to the Graph when needed, opens the `Dashboard widget` window
    and brings the Dashboard tab to the front). A sparkline is a 1.5 pt
    line of the last `sparkline_s` seconds (Settings, default 10 s),
    auto-scaled, from a fixed ring of 128 points per signal.
  - History under the grid: `History` (strong), checkbox `Filter`, signal
    combo, operator combo (`= != > <`), value field, `Export CSV...`; a
    virtualised table (newest first): `Time (s)`, `Δt (ms)`, `Data`
    (mono), one column per signal (PDO-mapped first, then user signals).
    It fills the pane down to its bottom edge.
- `Define signal` / `Edit signal` window (movable, like Add signal): grid
  Name, COB-ID 0x + `Ext`, Start bit, Length, Byte order (`Little endian |
  Big endian`), Type (`Unsigned | Signed | Float32`), Factor, Offset, Unit,
  Colour (8 swatches, the chosen one outlined); a small note on the start
  bit; error text in `error`; `Save`, `Cancel`. Escape closes it.
- `Similar bits` window: progress and `Cancel` while it runs, then a table
  `COB-ID | Bit | Match | Inverted | Toggles` (best first); a click on a
  row selects that message and bit in the Messages view.

### 8.11 Import DBC dialog

- Opened from File ▸ `Import DBC...` or the Messages toolbar. File dialog
  `DBC file to import as user signals` (filters DBC, All files). A file
  that cannot be read gives the status `DBC not read: <error>` and no
  dialog.
- Window `Import DBC` (in the foreground, not collapsible, not
  resizable, centred):
  - Strong file name.
  - `n message(s) with m signal(s) to import (k message(s) in the file)`.
  - `Nothing skipped.`, or a warning `Skipped: n[, m multiplexed]` and a
    scroll list (max height 160) of small lines, one per skipped item.
  - Small note: "Add keeps the user signals you have and skips a signal
    whose name is already used on the same ID. Replace first removes every
    user signal of the IDs in this file."
  - Buttons `Add`, `Replace all signals of these IDs` (both need at least
    one signal to import), `Cancel`. Escape closes the window.
- After Add or Replace, the status bar shows `DBC file: <report>`.

### 8.12 Log replay

- Adapter combo entry `Log file (replay)`; the channel control becomes
  the file name with `Browse...` (section 6). Formats: CANoxide CSV,
  candump log, PEAK trace (`.trc`, versions 1.0 to 2.1) and Vector ASC
  (`.asc`); the format is found from the content.
- Nothing is sent during a replay: every send control is disabled, the
  connection bar shows the pill `Replay: nothing is sent`.
- Transport bar under the connection bar, only during a replay (min
  height 28): info pill `film-strip` `Replay`; icon button Play / Pause;
  `skip-forward` `Step` (one frame forward); `Speed` + combo 150
  (`0.25x`, `0.5x`, `1x`, `2x`, `4x`, `8x`, `16x`, `As fast as possible`;
  tooltip "Replay speed (log time per wall time)"); checkbox `Loop`
  (tooltip "Start again at the end of the log"); the clock `t.ttt s /
  T.TTT s` (mono) and small `frame n / N`; `Seeking n %` while it seeks;
  a scrubber slider over the rest of the bar (tooltip "Drag or click to
  move the replay; the views are rebuilt up to this time").
- A log with bad lines asks first: modal (max width 520) `Log file with
  bad lines`, "<name>: n line(s) could not be read; m frame(s) are good.",
  up to 50 error lines (mono, small, error colour) and `... and n more`;
  buttons `Load the good frames` (accent) and `Cancel`.

### 8.13 Find signal

- Dock tab `Find signal` (`magnifying-glass` icon), View menu, opened
  beside a trace tab.
- Settings row: `Sizes` checkboxes (1, 8, 16, 32 bit), `Byte aligned`,
  `Little endian | Big endian`, `Signed`; `COB-IDs` field (hint `all (or
  hex: 181, 281)`) and `Start` (later `Reset`).
- Step row: `Step` combo (= != > < between increased decreased changed
  unchanged), one or two DragValues for the value steps, `Apply step`,
  `Undo step` (8 levels).
- `n candidates` (strong) and `after s1, then s2` (secondary); a warning
  line when the start set was cut at 200 000. At 50 candidates or fewer, a
  table `COB-ID | Start bit | Bits | Value` with `Define signal...` per
  row (opens the Define signal window filled in).

### 8.14 Emergencies

- Dock tab `Emergencies` (`warning` icon), closed by default, opens in
  the Network leaf.
- Toolbar: `Node` combo (`All nodes`, `Node n`); `trash` `Clear` (tooltip
  "Clear the list (nothing is sent)"); `Export CSV...`
  (`emergencies.csv`); the count `n EMCY` (or `k of n EMCY` with a node
  filter) and `(n oldest dropped)` when the list is full.
- Table, newest first (max scroll height 360): Time (96, `x.xxx s`), Node
  (120, `n <name>`), Code (260: tone pill with the hex code, success for
  an error reset, else warning, and the code text in the same
  colour), Error register (200, the set bits in words, tooltip), Manufacturer
  (130).
- Empty: icon `warning` 28, "No emergency received yet." and "Each EMCY
  frame on the bus (COB-ID 0x081..0x0FF) is listed here, newest first."
- Separator, then strong `Error history (0x1003) of node n` (the filter
  node, else the active node):
  - `Read error history (0x1003)` (tooltip "Read 0x1003:00 and then each
    entry by SDO"); caution button `Clear error history` (tooltip "Write
    0 to 0x1003:00"; asks first: `Clear error history`, danger).
  - Result: `Node n: the error history is empty.` or `Node n: k entries
    (1 = newest)` and a table No. (44), Code (64), Text (flex), Info (64).
    A read error in error colour.
- On the simulator only: strong `Simulator`, `EMCY code 0x` [48] (default
  `8130`), `reg 0x` [28] (default `11`), `Raise EMCY` (tooltip "The
  simulated node sends this EMCY and adds it to its 0x1003"), `Reset EMCY`
  (sends code 0x0000).

### 8.15 EDS check

- Dock tab `EDS check` (`check-square` icon), closed by default, opens in
  the Object dictionary leaf.
- Header: strong `Check node n against <EDS file>`; secondary "Reads each
  entry by SDO and compares it with the EDS. Nothing is written."
- Options row: checkbox `Skip DOMAIN entries` (default on); `Index from
  0x` [44] `to 0x` [44] (defaults `0000`, `FFFF`); `Stop after`
  DragValue 0 … 100 (default 3) `time-outs in a row (0 = never)`.
- Send button `play` `Start`; next to it the send hint, or `No EDS:
  select a device with an EDS file`. On the simulator without a device it
  checks against the built-in simulator EDS.
- While it runs: progress bar 220 `<label>: done / total` and `Cancel`.
- Before the first run: `No check run yet. Press Start.`
- Result:
  - The summary line in error, warning or success colour; small `n
    DOMAIN entries not read` when they were skipped.
  - Checkbox `show ok entries`, `Export CSV...` (`nodeN_edscheck.csv`),
    `Export JUnit XML...` (`nodeN_edscheck.xml`).
  - Table: Index:sub (72), Name (220), Result (120, tone pill: pass
    success, warning warning, fail error, skipped neutral), Detail (flex,
    error colour for a fail). Result kinds: `ok`, `missing`,
    `write_only_ok`, `access`, `size`, `value`, `abort`, `timeout`,
    `not_checked`.
  - With the ok entries hidden and nothing else: `Every entry is ok.`

### 8.16 Program download (wizard)

A window, not a dock tab: File ▸ `Program download...` or the Network
view (NMT group, for the active node). Title `Program download`, not
collapsible, not resizable, centred, max width 520. Nothing is sent until
`Download program`.

- Grid (locked while the download runs): `Node` DragValue 1 … 127;
  `Program` 1 … 254 (tooltip "The sub-index of 0x1F50, 0x1F51, 0x1F56 and
  0x1F57"); `File` TextEdit 300 (hint `program file`) + `Browse...`; the
  size `n bytes` and the SHA-256 (mono, small), or the read error in error
  colour; `Transfer` combo (`auto`, `segmented`, `block`).
- Strong `Steps`, one checkbox per step, in this order (all on by default
  except Clear):
  1. `Read the program identification (0x1F56)`
  2. `Stop the program (0x1F51 = 0)`
  3. `Clear the program (0x1F51 = 3)` (default off)
  4. `Download the file (0x1F50)`
  5. `Wait for the flash (0x1F57)`
  6. `Check the flash error code`
  7. `Start the program (0x1F51 = 1)`
  8. `Wait for the boot-up message` (enabled only with step 7)
  9. `Read the new program identification`
- Caution note: "The node stops its program for the download. A wrong file
  can leave the device without a working program. Never download to a
  machine in operation."
- Buttons: danger `Download program` (tooltip "Run the enabled steps (asks
  first)"); it always asks first (`Download program`: the file, its size,
  the program and node, the list of enabled steps and the warning; action
  button `Download`). `Cancel` while it runs, `Close` when it does not.
- Report: progress bar `done / size bytes`; a grid of step name, tone
  pill (`pending`, `running` info, `ok` success, `skipped`, `failed`
  error, `cancelled` warning, `not run`) and a small detail; a final pill
  `Download done`, `Download cancelled` or `Download failed: <step>`; the
  line `Program identification 0x.. -> 0x.. (CRC-32 of the file 0x..)`.
  A failed step stops the sequence.

### 8.17 Drive (CiA 402)

- Dock tab `Drive (CiA 402)` (`engine` icon), closed by default, opens in
  the Network leaf. It works on the active node.
- Warning line when no command can be sent: `Listen only: every command is
  disabled.` or `Not connected: the commands are disabled.`
- Poll row: strong `Node n`; checkbox `Poll` (tooltip "Read 6041, 6061,
  6064, 606C, 6077 and 603F by SDO each period"); period DragValue 50 …
  10 000 ms (default 200, tooltip `Poll period`); pill `polling` while it
  runs. Polling stops when the tab is closed or nothing can be sent.
- State: strong `State: unknown (not polled)`, `State: <name>` or `State:
  no valid state (statusword 0x..)`. Under it a row of the 8 state pills:
  `Not ready to switch on`, `Switch on disabled`, `Ready to switch on`,
  `Switched on`, `Operation enabled`, `Quick stop active`, `Fault reaction
  active`, `Fault`. The current state is a tone pill (Fault and Fault
  reaction active error, Quick stop active warning, Operation enabled
  info, the others success); the others are pills in text.disabled.
- Values grid: `Statusword`, `Mode (6061)`, `Position (6064)`, `Velocity
  (606C)`, `Torque (6077)`, `Error code (603F)`; `n/a` for an object the
  drive does not have, `-` before the first read.
- Status bits: `check-square` (set) or `square` (clear) + `Voltage
  enabled`, `Warning`, `Remote`, `Target reached`, `Internal limit
  active`.
- Strong `Commands`:
  - `Quick stop` and `Disable voltage`: the largest buttons of the view,
    icon `hand-palm`, text size 16, white on danger.fill, min size 150 ×
    34. They are enabled whenever a command can be sent (connected, not
    listen-only), in every state and without the motion check box.
    Tooltip "Write the controlword 0x.. (always allowed while
    connected)".
  - Send buttons `Shutdown`, `Switch on`, `Disable operation`, `Fault
    reset`: enabled only when the current state allows the command;
    disabled tooltip "Not allowed in this state (poll first)".
  - `Mode of operation (6060)` combo 240 (default 3) with the modes 1
    (pp), 3 (pv), 4 (tq), 6 (hm), 7 (ip), 8 (csp), 9 (csv), 10 (cst), each
    shown as `<number> <name> (<short>)`, e.g. `3 Profile velocity (pv)`;
    + `Write mode`. In Operation enabled, `Write mode` is disabled until the
    motion check box is ticked (tooltip "Operation enabled: tick the check
    box of the motion group first").
- **Motion (danger)** group: a frame with a 1 px danger.fill stroke,
  radius 4, margin 8.
  - Header: `warning` + `Motion (danger)`, strong, error colour.
  - Small secondary text: "These commands can make the drive move."
  - Checkbox `I have checked that the drive can move safely`, enabled
    while a command can be sent. The tick is not saved: it is cleared on
    disconnect and when the node changes.
  - The group is locked until the box is ticked (and a command can be
    sent). Unlocked: danger button `Enable operation` (tooltip "Write the
    controlword 0x000F"), then a grid `Target velocity (60FF)` DragValue +
    danger `Write 0x60FF`, `Target position (607A)` + `Write 0x607A`,
    `Target torque (6071)` + `Write 0x6071`.
- A note line at the bottom (error colour for a failed command, else
  secondary).

### 8.18 Dashboard

- Dock tab `Dashboard` (`gauge` icon), closed by default, opens in the
  Graph leaf. The widgets show Graph signals; the layout is saved as
  `dashboard.json` in the data folder.
- Toolbar: `plus` `Add signal...` (tooltip "Add a widget for a signal (the
  picker of the Graph)"; opens the Add signal window of the Graph, 8.5);
  `squares-four` `Arrange` (tooltip "Place the widgets in rows, in their
  order"; disabled without widgets); separator; `stop` `Stop polling` +
  pill `polling`, or `play` `Start polling`.
- Canvas: the widgets are cards at their own place and size (`rect` x, y,
  w, h in points, in `dashboard.json`), in a scroll area that scrolls both
  ways. All four values snap to a 10-point grid; size 120 × 90 to 800 ×
  600; x and y not below 0. Cards may overlap: the list order is the draw
  order, and the card that was last moved, resized or added is at the end
  (on top).
  - Move: drag the title row (cursor grab / grabbing). Resize: drag the
    14 × 14 grip in the bottom-right corner (three short diagonal strokes
    in text.secondary, text.primary on hover; cursor resize NW-SE). While a
    card is dragged it follows the pointer snapped to the grid, and a
    faint line grid (border.subtle at 0.45) is drawn on the canvas. The
    file is written when the drag ends.
  - `Arrange`: left to right in rows that fit the visible width, gap 10,
    each size kept, top-aligned per row. A new widget goes to the first
    free place of the same row scan (10 from the other cards), else below
    all cards.
  - A `dashboard.json` without rectangles (0.5.0) loads with the sizes of
    `size` and the places of Arrange at a width of 1000.
- A card: fill bg.panel, 1 px border.subtle, radius 6, margin 8; its own
  top-down layout on its rectangle (clipped to it). Title row: strong
  title, truncated to one line, then icon buttons `pencil-simple` `Edit`
  and `trash` `Remove` at the right, always inside the card. `No value
  yet` before the first sample.
- Size presets (the `Size` combo): Small 170 × 130, Medium 240 × 180 (the
  default), Large 340 × 250.
- Widget types; the content scales with the inner rectangle of the card.
  The value text is mono, at the largest size from 12 to 96 pt that fits
  the width and the height of its area on one line; it never wraps.
  - Value: the value in the whole content area.
  - Bar: a track across the inner width (height 0.22 of the content, 8 to
    40) in bg.button with the fill, 1.5 px marks at the thresholds; the
    value below it.
  - Dial: a 270° arc as large as the area above the text allows, the
    needle, ticks at the thresholds; the value text below (0.3 of the
    content height, 16 to 110).
  - Lamp: a circle, centred (success when on, bg.button when off), and
    `ON (value)` / `OFF (value)` below it.
- Colour by level: normal text_accent (text in text.primary), warning
  warning, alarm error.
- Stale value (older than 5 poll periods or 2 s, whichever is longer):
  the colours are dimmed (gamma 0.35), the text is text.disabled, and a
  small warning `clock` with the age is shown.
- Empty: icon `gauge` 28, "No widgets." and "Press Add signal to show a
  value as a number, a bar, a dial or a lamp."
- Window `Dashboard widget` (Edit, and after Add signal): `Signal` (the
  source, text), `Title` [200], `Type` combo `Value | Bar | Dial | Lamp`,
  `Size` `Small | Medium | Large` (sets w and h; a card resized by the
  pointer shows `Custom (w × h)`; Save keeps the place), `Min`, `Max`,
  `Decimals` 0 … 6;
  `Warning at` and `Alarm at`, each a checkbox + value; for a lamp only,
  `Lamp on above` checkbox + value (tooltip "Off: the lamp is on when the
  value is not 0"); `Save`, `Cancel`.

## 9. Settings dialog

- Modal `egui::Window` 760 × 560 (min 640 × 480), centred, bg.popup,
  radius 6, 1 px border.strong, shadow; scrim behind (3.1).
- Header 48: title `Settings` (`title` style), search box (240, hint
  `Search settings`) right-aligned, close `x`.
- Left nav 180 wide, fill bg.bar, items 32 high, padding 12, icon 16 +
  `body`; selected = bg.selection + text.primary; hover bg.row.hover.
  Sections: Appearance (`palette`), Connection (`plugs`), SDO
  (`arrows-left-right`), LSS (`identification-card`), Scan (`broadcast`), Trace (`list-dashes`), Graph
  (`chart-line`), Safety (`shield-check`), Data folder (`folder`).
- Content: one long scroll list of all sections; clicking a nav item
  scrolls to it; the nav follows the scroll position. Section title
  `heading` + 1 px border.subtle under it, 16 gap before the next section.
- Each setting row: min height 40; left column 260: label `body`
  text.primary + one line of help in `small` text.secondary; right: the
  control, left-aligned at x = 280. A changed-from-default setting shows
  a ghost `arrow-counter-clockwise` (reset) icon after the control.
- Footer 52, top border 1 px border.subtle: left ghost `Restore all
  defaults`; right `Cancel` (secondary) and `Save` (primary).
  Appearance changes preview live; Cancel reverts them.
- Stored as JSON in the data folder (`settings.json`).

Settings and defaults (`+` = added by the designer, not in the brief):

| Section | Setting | Control | Default |
|---|---|---|---|
| Appearance | Theme | segmented System / Dark / Light | System |
| | UI scale | combo 80 / 90 / 100 / 110 / 125 / 150 / 175 / 200 % | 100 % |
| | Font size | DragValue offset −2 … +4 pt (shown as body size 12…18) | 14 pt (offset 0) |
| | Row density | segmented Compact / Comfortable | Compact |
| | + Zebra stripes in tables | checkbox | on |
| | + Show `0x` prefix on hex values | checkbox | on |
| | + Reduce colour (chips/pills outlined, no fill) | checkbox | off |
| Connection | Default adapter | combo (detected adapters + Simulator) | Simulator |
| | Default bitrate | combo 10 k … 1 M | 250 kbit/s |
| | Listen-only on start | checkbox | off |
| | + Connect automatically on start | checkbox | off |
| | + Restart CAN controller after bus-off | checkbox | on |
| SDO | Timeout | DragValue ms 50 … 10 000 | 500 ms |
| | Block size (block transfer) | DragValue 1 … 127 | 127 |
| | Transfer mode | combo Auto / Expedited+segmented / Block | Auto |
| | + Retries on timeout | DragValue 0 … 5 | 1 |
| | + Client COB-ID offset (default 0x600/0x580) | checkbox "Use default" + mono fields | Use default |
| LSS | LSS timeout (wait for a confirmed service) | DragValue ms 50 … 10 000 | 1 000 ms |
| | + Default switch delay (Activate bit rate) | DragValue ms 0 … 65 535 | 100 ms |
| | + Fastscan step timeout | DragValue ms 5 … 500 | 20 ms |
| Scan | Node ID range | two DragValues 1 … 127 | 1 – 127 |
| | Timeout per node | DragValue ms 10 … 2 000 | 100 ms |
| | + Scan after connect | checkbox | off |
| | + Heartbeat lost after | DragValue ms (0 = 3 × producer time) | 0 (auto) |
| Trace | Buffer size | combo 10 000 / 50 000 / 100 000 / 500 000 / 1 000 000 frames | 100 000 |
| | Timestamp format | combo Absolute / Relative / Delta | Relative |
| | + Timestamp resolution | combo ms / µs | µs |
| | Default data view | segmented Hex / Dec / ASCII | Hex |
| | Colour by type | combo Off / Chips / Chips + row tint | Chips |
| | + Autoscroll by default | checkbox | on |
| | + Log file format | combo CSV / candump log | CSV |
| Graph | Buffer size | combo points per series 10 000 / 100 000 / 1 000 000 | 100 000 |
| | Default time window | combo 5 s / 10 s / 30 s / 1 min / 5 min | 10 s |
| | + Line width | DragValue 1.0 … 3.0 | 1.5 |
| | + Show cursor readout | checkbox | on |
| Safety | Confirm before SDO write | checkbox | on |
| | Confirm before NMT to all nodes | checkbox | on |
| | + Confirm before PDO apply | checkbox | on |
| | + Confirm before "Write pending" with more than 1 value | checkbox | on |
| | Confirm before LSS node-ID or bit rate change (also Activate bit rate) | checkbox | on |
| | Confirm before DCF restore | checkbox | on |
| | + Block all transmit in listen-only (always on, shown read-only) | checkbox disabled, checked | on |
| Data folder | Path | read-only mono text field + `Change…` + `Open` | `<exe folder>\CANoxide_data` |
| | + DCF folder (Save / Restore default) | read-only mono field + `Change…` | `<data folder>\dcf` |
| | + DCF file name pattern for Save | TextEdit mono | `{name}_{node}_{date}.dcf` |
| | + Keep the last N trace logs | DragValue 0 … 1000 (0 = keep all) | 0 |

## 10. Keyboard shortcuts (shown in tooltips and Help ▸ Keyboard shortcuts)

Ctrl+, Settings · Ctrl+O Open EDS · Ctrl+Enter connect/disconnect · F11
full screen. Help ▸ Keyboard shortcuts writes this line into the status
bar. Escape closes the Add signal, Define signal and Import DBC windows.
There are no shortcuts for LSS, DCF restore, program download or drive
commands, on purpose.

## 11. Icon list (Phosphor Regular names)

| Icon | Where |
|---|---|
| `plugs` | Settings ▸ Connection |
| `sun`, `moon`, `desktop` | Theme quick switch (menu bar right) |
| `gear-six` | Settings menu item |
| `cpu` | Devices tab, Devices empty state |
| `books` | EDS library tab |
| `list-magnifying-glass` | Object dictionary tab, its empty state |
| `graph` | Network tab, Network empty state |
| `list-dashes` | Trace tab, Settings ▸ Trace, Trace empty |
| `table` | Messages tab |
| `chart-line` | Graph tab, Settings ▸ Graph, Graph empty, Messages value list "Add to the Graph" |
| `squares-four` | Dashboard `Arrange` |
| `align-bottom`, `align-left`, `align-right`, `align-top` | Messages bit view position |
| `sliders-horizontal` | PDO configuration tab |
| `paper-plane-tilt` | Transmit list tab, empty state |
| `paper-plane-right` | Send button |
| `warning` | Emergencies tab and empty state, caution buttons and notes, Drive "Motion (danger)" header |
| `check-square` | EDS check tab; Drive status bit set |
| `square` | Drive status bit clear |
| `engine` | Drive (CiA 402) tab |
| `hand-palm` | Drive `Quick stop`, `Disable voltage` |
| `gauge` | Dashboard tab, Dashboard empty state, Messages value list "Add to the Dashboard" |
| `film-strip` | Replay pill of the transport bar |
| `skip-forward` | Replay `Step` |
| `plus` | Add device, Add signal (Graph, Dashboard), New frame, New trace tab |
| `pencil-simple` | Edit device, Dashboard widget Edit |
| `trash` | Remove, Clear trace / graph / emergencies, Dashboard widget Remove |
| `copy` | Duplicate frame, copy cell |
| `magnifying-glass` | Find signal tab, filter boxes, settings search |
| `funnel` | Trace filter popup button |
| `x` | Close tab, clear filter, close dialog, Messages bit view Close |
| `caret-right`, `caret-down` | Tree expand / collapse in the Object dictionary |
| `caret-up`, `caret-down` (10 pt) | Sort indicators, combo arrows |
| `caret-double-down`, `caret-double-up` | Object dictionary Expand all / Collapse all |
| `download-simple` | Read, Read all (Object dictionary), Read PDOs |
| `upload-simple` | Write (Object dictionary) |
| `arrow-up`, `arrow-down` | Move a PDO mapping entry |
| `arrow-u-up-left` | Discard pending changes |
| `arrow-counter-clockwise` | Refresh the channel / net / port list, reset setting to default |
| `play`, `pause`, `stop` | Trace / graph pause; start / stop polling (Graph, Dashboard); EDS check Start; transmit start / stop; replay play / pause |
| `record` | Node state operational |
| `arrow-line-down` | Trace Auto-scroll toggle |
| `arrow-line-right` | Graph Follow toggle |
| `magnifying-glass-plus`, `magnifying-glass-minus` | Graph Zoom in / Zoom out; `magnifying-glass-plus` also LSS Fastscan |
| `clock` | Graph time window, Dashboard stale-value age |
| `floppy-disk` | Trace `Save trace...`, Graph `Export CSV...`, LSS Store configuration |
| `broadcast` | Settings ▸ Scan |
| `dots-three` | Overflow menus |
| `export` | Graph `Export range...` |
| `arrow-down-left`, `arrow-up-right` | Rx / Tx direction |
| `lock-simple` | Read-only / const access |
| `circle-dashed`, `circle-half`, `record`, `stop-circle`, `x-circle`, `question` | Node states boot-up, pre-op, operational, stopped, lost, unknown |
| `check-circle`, `warning`, `warning-circle`, `info` | Success, warning, error, info messages and pills |
| `palette` | Settings ▸ Appearance |
| `arrows-left-right` | Settings ▸ SDO, PDO configuration empty state |
| `shield-check` | Settings ▸ Safety |
| `folder`, `folder-open` | Settings ▸ Data folder; data folder button of the status bar, log file `Browse...`, DCF `Browse…` |
| `identification-card` | LSS tab, Settings ▸ LSS |
| `crosshair` | LSS Select (switch state selective), LSS state pill "Configuration (selective)" |
| `wrench` | LSS state pill "Configuration (global)" |
| `lightning` | Danger buttons (e.g. LSS Activate bit rate, Restore defaults, Download program, Enable operation) |
| `file-code` | DCF save / restore tab, its empty state |
| `eye` | DCF Dry run button |
| `git-diff` | DCF Compare |

If a name is missing in the egui-phosphor release, use the closest glyph
and note it in the code; do not substitute emoji.

## 12. Sample data used in the canvas (generic)

Nodes: 1 `Motor drive` (operational), 2 `I/O module` (pre-op), 3
`Encoder` (operational), 4 `Valve block` (stopped), 5 `Sensor hub`
(lost). Objects: 1000 Device type, 1001 Error register, 1017 Producer
heartbeat time, 1018 Identity object (subs 0–4), 2000 Motor speed, 6040
Controlword, 6041 Statusword, 6060 Modes of operation, 606C Velocity actual
value, 60FF Target velocity. Graph series: Motor speed (rpm), Velocity
actual value (rpm), Statusword bit 2 (raw bits). LSS identity
`0x000000AB / 0x00005020 / 0x00030004 / 0x00007710`, new node-ID 5, bit
rate 500 kbit/s. DCF file `motor_drive_1_2026-11-19.dcf`, 388 objects.
