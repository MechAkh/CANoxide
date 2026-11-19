# CANoxide v0.2 — UI design specification

For: the coder who builds the v0.2 UI in Rust with `egui` 0.31, `egui_dock`
and `egui_plot`. This file is complete on its own; the design canvas only
illustrates it. All sizes are egui logical points at UI scale 100 %
(1 pt = 1 px at 100 % Windows scaling). Hex colours are sRGB, opaque unless
an alpha is given.

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

1. **Native window title bar** (OS). Title `CANoxide 0.2`.
2. **Menu bar** — height 28, fill bg.frame, bottom 1 px border.subtle.
   Items `File  View  Settings  Help`, `body`, padding 10, hover fill
   bg.button.hover radius 4. Right side: theme quick switch (icon button
   `sun`/`moon`/`desktop`, cycles System→Dark→Light).
   - File: Open EDS…, Recent EDS ▸, Save trace as…, Export object dictionary (CSV)…, Open data folder, Exit.
   - View: list of panels with check marks (Devices, EDS library, Object dictionary, Network, Trace 1…4, Graph, PDO configuration, Transmit list, LSS, DCF save / restore); separator; New trace tab (disabled at 4); separator; Reset layout; Full screen F11.
   - Settings: opens the Settings dialog (Ctrl+,) — it is a direct menu item, not a submenu.
   - Help: User guide, Keyboard shortcuts, Open log folder, About CANoxide (modal 420 wide: app name `CANoxide` in `title`, version `0.2.x` mono, licence, font and icon licences, project link).
3. **Connection bar** — height 38, fill bg.frame, bottom 1 px border.strong.
   Left to right, gap 8, separators 1 px border.subtle × 20 between groups:
   - `Adapter` label (small, secondary) + ComboBox 150 (`Simulator`, `PCAN-USB 1`, …).
   - `Channel` + ComboBox 120 (`virtual bus`).
   - `Bitrate` + ComboBox 110 (`250 kbit/s`).
   - Checkbox `Listen only`.
   - Primary button `Connect` (icon `plugs-connected`), width 110; when
     connected becomes secondary `Disconnect` (icon `plugs`).
   - Separator. Right-aligned group: bus-state pill (`Error active` =
     success, `Error passive` = warning, `Bus off` = error, `Not connected`
     = unknown); `Load` label + a 60 × 6 meter (fill bg.button, value
     accent; > 70 % warning, > 90 % error) + `12.4 %` mono; `fr/s` label +
     `1 284` mono, width 56 right-aligned.
4. **Dock area** — `DockArea` filling the rest, background bg.frame (shows
   in the 4 px gaps), 4 px margin to the window edge left/right.
5. **Status bar** — height 24, fill bg.frame, top 1 px border.subtle,
   `small` text. Left: connection summary (`Connected · Simulator ·
   virtual bus · 250 kbit/s` or `Not connected. Nothing is sent until you
   press a button.`). Middle: last action result with semantic icon
   (`check-circle` success / `warning-circle` error), e.g. `SDO read
   6064:00 = 100 000 (2 ms)`. Right: `Trace 41 % of 100 000` and the data
   folder name (click opens the folder; full path in tooltip).

## 7. Default dock layouts

Chosen on first start (no saved layout) by window inner width:
< 1600 → layout A; ≥ 1600 → layout B. The layout is saved (serde) in the
data folder on exit and restored on start. View ▸ Reset layout rebuilds
the default for the current width. `*` = active tab of the leaf.

### Layout A (1280 × 800)

```
Horizontal
├── 0.21  Leaf [Devices*, EDS library]
└── 0.79  Vertical
    ├── 0.56  Leaf [Object dictionary*, Network, PDO configuration]
    └── 0.44  Horizontal
        ├── 0.58  Leaf [Trace 1*, Transmit list]
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
    │   └── 0.45  Leaf [Trace 1*, Transmit list]
    └── 0.40  Vertical                          (= 0.34 of the window)
        ├── 0.40  Leaf [Network*]
        └── 0.60  Leaf [Graph*]
```

### Building it with egui_dock

`egui_dock` splits are binary. In `split_left/right/above/below(parent,
fraction, tabs)` the `fraction` is the share the **existing (old)** node
keeps; the new tabs get `1 − fraction`. Check this against the egui_dock
version in Cargo.lock before relying on it. Sketch for layout A:

```rust
let mut dock = DockState::new(vec![Tab::ObjectDictionary, Tab::Network, Tab::PdoConfig]);
let s = dock.main_surface_mut();
let [main, _devices] = s.split_left(NodeIndex::root(), 0.79, vec![Tab::Devices, Tab::EdsLibrary]);
let [_od, bottom]    = s.split_below(main, 0.56, vec![Tab::Trace(1), Tab::TransmitList]);
let [_trace, _graph] = s.split_right(bottom, 0.58, vec![Tab::Graph]);
```
Layout B:
```rust
let mut dock = DockState::new(vec![Tab::ObjectDictionary, Tab::PdoConfig]);
let s = dock.main_surface_mut();
let [mid, _dev]      = s.split_left(NodeIndex::root(), 0.85, vec![Tab::Devices, Tab::EdsLibrary]);
let [mid, right]     = s.split_right(mid, 0.60, vec![Tab::Network]);
let [_od, _trace]    = s.split_below(mid, 0.55, vec![Tab::Trace(1), Tab::TransmitList]);
let [_net, _graph]   = s.split_below(right, 0.40, vec![Tab::Graph]);
```

### LSS and DCF save / restore in the default layouts

Both are **closed by default** in layouts A and B (commissioning work, not
everyday monitoring; the LSS actions can take a device off the bus).
View ▸ LSS opens it as a new tab in the leaf that holds **Network**
(layout A: beside Object dictionary / Network / PDO configuration; layout
B: beside Network). View ▸ DCF save / restore opens it as a new tab in the
leaf that holds **Object dictionary**. If that leaf no longer exists, the
largest leaf is used. Once opened or moved, the saved layout keeps them
where the user put them.

### Tab rules

- Every view is closable; View menu reopens it into the leaf where it was
  last, else as a new tab in the largest leaf.
- Trace tabs: up to 4 (`Trace 1`…`Trace 4`), each with its own filter,
  mode and pause state; renameable. The 4th disables "New trace tab".
- Tab icons: Devices `cpu`, EDS library `books`, Object dictionary
  `list-magnifying-glass` (fallback `table`), Network `graph` (Phosphor
  "graph" = node diagram), Trace `list-dashes`, Graph `chart-line`, PDO
  configuration `sliders-horizontal`, Transmit list `paper-plane-tilt`,
  LSS `identification-card`, DCF save / restore `file-code`.
- Tab titles may carry a status dot: Trace shows a 6 px accent dot while
  paused; PDO configuration shows a 6 px warning dot while changes are
  pending.

## 8. Views

Column widths are starting widths in px; `flex` = takes the rest
(`Column::remainder()`), min given in brackets. All tables are resizable
unless noted.

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

- Toolbar (left → right): active device combo (`Node 1 · Motor drive`,
  200); filter box (220, hint `Filter index or name`); segmented
  `Hex | Dec | Text` (value format); separator; `Read all` (secondary,
  icon `download-simple`, reads visible readable objects), `Write pending`
  (primary, icon `upload-simple`, enabled when new values exist, shows
  count badge); separator; ghost `Expand all` (`arrows-out-line-vertical`)
  and `Collapse all` (`arrows-in-line-vertical`); right: overflow
  `dots-three` menu (Export CSV, Show only changed, Show EDS defaults).
- Second row, collapsible `Manual SDO` (header `caret-right` + `small`
  label): Node (DragValue 48), Index `0x` (mono 64), Sub (mono 40), Type
  combo (110), Value (mono 160), `Read`, `Write` buttons, and the last
  result text.
- Table columns: Index (64, mono; tree caret in the column for objects
  with subs), Sub (40, mono), Name (flex [180]), Type (96, `small` mono,
  e.g. `UNSIGNED32`, shortened `U32` when the column < 80), Access (52,
  `ro`/`rw`/`wo`/`const` small; `ro`/`const` with `lock-simple` 10 pt),
  Value (120, mono, right-aligned for decimal, left for hex), New value
  (120, inline TextEdit mono; non-empty → pending.fill + pending.border),
  Actions (56: ghost icon buttons `arrow-clockwise` Read, `upload-simple`
  Write; Write disabled for ro/const and when no new value).
- Object rows (index level) use `table` text in text.primary; sub rows
  are indented 16 in the Name column and their Index cell is empty.
  Record/array header rows show the name and `(record)`/`(array)` in
  text.secondary in the Type column and no value.
- Value states: unread `—` text.disabled; reading `…` with spinner 10 pt;
  read OK text.primary (fresh read flashes bg.row.hover for 600 ms — hover
  colour only, no animation curve needed); error → 5.9.
- Write confirmation (setting on): a small popup anchored to the button
  "Write 0x01F4 to 1017:00 on node 1?" with `Write` (primary) and
  `Cancel`.

### 8.3 Network

- Toolbar: `Scan` (secondary, icon `broadcast`), range text `1–127`
  (small secondary), separator; NMT group label `NMT` then buttons
  `Start` (`play`), `Stop` (`stop`), `Pre-op` (`pause`), `Reset node`
  (`arrow-counter-clockwise`), `Reset comm` (`arrows-clockwise`) — they act
  on the selected nodes; separator; `All nodes…` danger button (opens a
  confirm with the command list). Right: `5 nodes · 4 operational` small.
- While scanning: 4 px progress bar under the toolbar, `Scanning 37 / 127`
  text, Scan becomes `Stop scan`.
- Columns: checkbox (24), ID (44, mono, `0x01` + tooltip decimal; setting
  shows `1`), Name (flex [140]), State (120, pill), Device type (96,
  mono `0x00020192`), Err reg (64, mono; non-zero in warning with
  tooltip of the set bits), Identity (150, mono `00AB:1234 r1.2 #A1F3`
  vendor:product rev serial), Last EMCY (180, `small`: code mono + text,
  e.g. `FF10 generic` in EMCY colour, `—` when none), Last seen (84, mono
  right-aligned `0.4 s`; > heartbeat timeout → error).
- Empty: icon `graph`, "No nodes seen", "Run a scan or wait for
  heartbeats.", button `Scan 1–127`.

### 8.4 Trace (up to 4 tabs)

- Toolbar order: toggle `Pause` (`pause`/`play`, shortcut Space), `Clear`
  (`trash`), separator, Mode segmented `Scroll | Fixed` (Fixed = one row
  per COB-ID updated in place), Time combo (`Absolute`, `Relative`,
  `Delta`; 96), Data segmented `Hex | Dec | ASCII`, separator, filter box
  (200, hint `Filter: id 0x181, type sdo, node 2`), funnel button
  `Filters` with count badge (opens a popup: per-type checkboxes with the
  chips, Rx/Tx, node range, ID range), separator, toggle `Autoscroll`
  (`arrow-line-down`); right side: `Log` toggle (`record` icon, red
  `error` dot while logging, tooltip = file path), frame counter
  `12 483 frames` small mono.
- Columns: Time (104, mono, `12.304512` s; Delta shows `+0.000278`),
  Dir (44, glyph + `Rx`/`Tx`), COB-ID (64, mono `0x181`), Type (60, chip),
  Node (40, mono decimal), DLC (36, mono), Data (196, mono, bytes separated
  by single spaces, 8 bytes = 23 chars), Decoded (flex [200], `table`,
  e.g. `Upload resp 6064:00 = 100 000`).
- Rows: neutral fill with zebra; chips carry the type colour; ERR and
  EMCY rows get the row tint (3.5). Selected row bg.selection. Hover
  tooltip on Decoded shows the full decode.
- Up to 100 000 rows: use `TableBuilder::body().rows()` (virtualised).
- Empty: icon `list-dashes`, "No frames yet", "Connect to an adapter to
  start the trace.", button `Connect`. When paused with new frames
  arriving: banner info "Paused — 1 204 new frames. Resume" .
- Autoscroll (v0.3): on = the view follows the newest row; a user scroll up
  (wheel, drag, Arrow/Page Up, Home with the pointer over the table)
  leaves the bottom, a scroll back to the end (or End) follows again. Off =
  the top visible row stays anchored by its sequence number, also while
  the ring buffer drops old rows.

### 8.4a Messages (v0.3)

- Dock tab `Messages` (`table` icon), opened beside a trace tab; one row
  per COB-ID (11-bit and 29-bit apart, at most 2048 IDs).
- Toolbar: filter box (hint `Filter: COB-ID, type, node`), `Sort by`
  combo (`COB-ID | Count | Age`), checkbox `Mute noisy bytes` (a byte that
  changes on more than 90 % of at least 10 frames gets no tint), `Reset
  all statistics`, ID counter `3 IDs` small.
- Columns: COB-ID (mono), Type (chip, 3.5), Node, Count, Cycle (mean
  period, ms), DLC, B0..B7 (mono hex), Age (s since the last frame).
- Change tint: a changed byte cell gets `change_tint` (value up = blue,
  value down = red), mixed into the row background and fading to it over
  the fade time (Settings, default 2 s). Text keeps AA contrast at every
  step. With `Reduce colour`, a caret up / down mark is drawn instead.
  Repaint at 30 Hz only while a tint fades.
- Detail pane under the table for the selected row: header `181 TPDO1
  node 1`, frame counts, `Reset statistics` (row; keeps the last data),
  selection text `Selected: bit s, l bit`, `Add to Graph` (adds a Raw-bits
  signal, Intel, unsigned; status message, no tab switch).
- Bit grid: rows `Byte 0..7` (button selects the whole byte), columns bit
  7..0 and Hex; cell fill `bit_heat` by toggle rate; outline in the series
  colour of each mapped PDO signal, accent outline for the selection;
  click selects one bit, Shift-click extends; tooltip = bit number, value,
  toggle count.
- Mapped list right of the grid: one line per mapped object with swatch,
  index:sub, name and the decoded value.
- Empty: "No frames yet. Connect to see one row per COB-ID."

### 8.5 Graph

- Toolbar: toggle `Pause`, `Add signal` (secondary, `plus`), window combo
  (`5 s`, `10 s`, `30 s`, `1 min`, `5 min`, `All`; 80), toggle `Auto Y`
  (`arrows-vertical`), `Fit` (`corners-out`), `Clear` (`trash`); right:
  `Export CSV` ghost.
- Plot: `egui_plot::Plot` with x = seconds (relative to now, right edge 0
  in rolling mode), grid on, legend off (custom legend below), cursor
  readout on hover (vertical accent line + values in a tooltip).
- Legend table under the plot (or right of it when the leaf is wider
  than 900 px): rows 22 high: visibility checkbox (20), swatch (24: a 16 px
  line sample in the series colour and style), Name (flex [120]), Last
  (80, mono right), Min (72, mono.small right), Max (72, mono.small
  right), Unit (40, small), remove `x` ghost (24). Clicking a row toggles
  bold line (2.5 pt) for that series.
- Add-signal dialog (window 480 × 420 over a scrim, movable by its title
  bar, kept inside the app window; Escape or `x` closes it): segmented
  `PDO signal | SDO poll | Raw bits`;
  - PDO signal: device combo, list of mapped objects with search
    (Name, Index:Sub, Type; TPDOs first, then RPDOs, each entry prefixed
    `TPDOn` / `RPDOn`), scale (DragValue, default 1), offset, unit. The
    name is filled from the entry until the user edits it.
  - SDO poll: node, index/sub, type, poll period (ms, default 100), scale,
    unit; note in small secondary "Each poll adds 2 frames to the bus".
  - Raw bits: COB-ID (mono), start bit (0–63), length (1–64), byte order
    (`Intel | Motorola`), signed checkbox, scale, offset, unit.
  - Footer: series colour preview (next free of the 8), `Cancel`,
    `Add` (primary).
- Empty: icon `chart-line`, "No signals", "Add a PDO signal, an SDO poll
  or raw bits.", button `Add signal`.

### 8.6 PDO configuration

- Toolbar: active device combo, `Read from device` (secondary,
  `download-simple`), `Apply` (primary, `upload-simple`, count badge of
  pending rows), `Discard` (ghost `arrow-u-up-left`), separator,
  segmented `RPDO | TPDO | All`.
- One row per PDO (row height 28 compact / 32 comfortable because of the
  mapping chips): Enable checkbox (28), PDO (60, `TPDO1`), COB-ID (84,
  mono TextEdit `0x181`), Trans. type (120, combo: `0 acyclic sync`,
  `1–240 cyclic sync`, `254 event (mfr)`, `255 event (profile)`), Inhibit
  (80, mono, unit `×100 µs` in header tooltip), Event timer (84, mono ms),
  Mapping (flex [240]: chips `6041:00 Statusword 16` in `mono.small` on
  bg.button radius 3, gap 4; trailing `pencil-simple` opens the mapping
  editor; bit total `32 / 64 bit` right-aligned small secondary, error
  colour when > 64).
- Pending change: the changed cell gets pending.fill + 1 px
  pending.border; the row's PDO cell shows a 6 px warning dot. After
  Apply: success flash (bg.row.hover 600 ms) or error cell (5.9).
- Disabled PDO rows: all cells in text.secondary except Enable.

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

Ctrl+, Settings · Ctrl+O Open EDS · (no shortcuts for LSS or DCF restore actions, on purpose) · Ctrl+F focus the filter of the focused
view · Space pause/resume trace or graph of the focused view · Ctrl+L
clear the focused trace · Ctrl+Enter connect/disconnect · F5 read selected
object · Ctrl+S write pending · Ctrl+W close tab · Ctrl+Tab next tab in
leaf · F11 full screen.

## 11. Icon list (Phosphor Regular names)

| Icon | Where |
|---|---|
| `plugs-connected` | Connect button |
| `plugs` | Disconnect button, Settings ▸ Connection |
| `sun`, `moon`, `desktop` | Theme quick switch (menu bar right) |
| `gear-six` | Settings menu item |
| `cpu` | Devices tab, Devices empty state |
| `books` | EDS library tab |
| `list-magnifying-glass` (fallback `table`) | Object dictionary tab |
| `graph` | Network tab, Network empty state |
| `list-dashes` | Trace tab, Settings ▸ Trace, Trace empty |
| `chart-line` | Graph tab, Settings ▸ Graph, Graph empty |
| `sliders-horizontal` | PDO configuration tab |
| `paper-plane-tilt` | Transmit list tab, empty state |
| `paper-plane-right` | Send button |
| `plus` | Add device, Add signal, New frame |
| `pencil-simple` | Edit device, edit mapping |
| `trash` | Remove, Clear trace/graph |
| `copy` | Duplicate frame, copy cell |
| `magnifying-glass` | Filter boxes, settings search |
| `funnel` | Trace filter popup button |
| `x` | Close tab, clear filter, close dialog, remove legend row |
| `caret-right`, `caret-down` | Tree expand, Manual SDO section |
| `caret-up`, `caret-down` (10 pt) | Sort indicators, combo arrows |
| `download-simple` | Read all, Read from device |
| `upload-simple` | Write, Write pending, Apply |
| `arrow-clockwise` | Read one object |
| `arrow-u-up-left` | Discard pending changes |
| `arrow-counter-clockwise` | Reset node (NMT), reset setting to default |
| `arrows-clockwise` | Reset communication (NMT) |
| `play`, `pause`, `stop` | NMT start/pre-op/stop; trace/graph pause; transmit start/stop |
| `record` | Log to file toggle |
| `arrow-line-down` | Autoscroll toggle |
| `broadcast` | Scan, Settings ▸ Scan |
| `arrows-out-line-vertical`, `arrows-in-line-vertical` | Expand all / Collapse all |
| `dots-three` | Overflow menus |
| `arrows-vertical` | Auto Y |
| `corners-out` | Fit plot |
| `export` | Export CSV |
| `arrow-down-left`, `arrow-up-right` | Rx / Tx direction |
| `lock-simple` | Read-only / const access |
| `circle-dashed`, `circle-half`, `record`, `stop-circle`, `x-circle`, `question` | Node states boot-up, pre-op, operational, stopped, lost, unknown |
| `check-circle`, `warning`, `warning-circle`, `info` | Success, warning, error, info messages |
| `palette` | Settings ▸ Appearance |
| `arrows-left-right` | Settings ▸ SDO |
| `shield-check` | Settings ▸ Safety |
| `folder`, `folder-open` | Settings ▸ Data folder, Open folder, DCF Browse… |
| `identification-card` | LSS tab, Settings ▸ LSS |
| `crosshair` | LSS Select (switch state selective), LSS state pill "Configuration (selective)" |
| `wrench` | LSS state pill "Configuration (global)" |
| `magnifying-glass-plus` | LSS Fastscan |
| `lightning` | LSS Activate bit rate |
| `floppy-disk` | LSS Store configuration |
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
