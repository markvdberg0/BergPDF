# BergPDF interface style

The visual style and layout of BergPDF from version 0.4 on: **direction C, "Ribbon, refined"**, plus the new
**start screen**. This file is the source of truth: the mock-ups illustrate it, the numbers here win. It is written so
it can be used for other apps as well (section 9 has the tokens as JSON and CSS) and implemented in BergPDF (section 8).

Mock-ups: the design canvas "BergPDF UI Directions" (artboards *C · Ribbon, refined* and *Start screen*). Where this
file differs from the mock-ups, that is on purpose; section 10 lists the differences.

---

## 1. Principles

1. **The document is the hero.** Chrome is quiet and neutral; colour is reserved for what is active or selected.
2. **Familiar, not old.** The ribbon stays (people know where things are), but as one calm row with labels.
3. **One accent.** Terracotta means "active, selected or the main action". Never decoration.
4. **Plain words.** Labels say what happens ("Make a scan searchable", not "OCR"). Sentence case everywhere.
5. **Honest about privacy.** Only state what is true: offline, no account, nothing sent without a button press.
6. **Readable for everyone.** Text meets 4.5:1, icons and control edges meet 3:1, in light and dark.

---

## 2. Tokens

All sizes are in px at 100 % UI scale. In egui 1 px = 1 point at zoom factor 1.0, so the numbers can be used
directly; the UI-scale preference multiplies everything.

### 2.1 Colour

The theme never changes page colours: a white page stays white in dark mode.

| Token | Light | Dark | Used for |
|---|---|---|---|
| `bg.chrome` | `#F6F5F2` | `#1E1E1C` | Title bar, ribbon tab row, document tab row, status bar, start screen background |
| `bg.surface` | `#FFFFFF` | `#262624` | Ribbon row, right panel, active tabs, cards, buttons, selected segment (light) |
| `bg.panel` | `#FAFAF8` | `#232321` | Left panel, drop zone |
| `bg.canvas` | `#E9E8E3` | `#141413` | Area around the pages, recent-file thumbnail area |
| `bg.field` | `#FFFFFF` | `#1C1C1B` | Text fields, dropdowns, page-number input |
| `bg.track` | `#EFEEEA` | `#1C1C1B` | Segmented-control track, keyboard-shortcut chips, privacy note |
| `bg.segment` | `#FFFFFF` | `#34332F` | Selected segment in a segmented control |
| `bg.hover` | `#EFEEEA` | `#302F2D` | Hover on ghost buttons, tabs, list rows |
| `bg.pressed` | `#E6E4DE` | `#383734` | Pressed state |
| `border.default` | `#E2E0DA` | `#383734` | Panel edges, tab outlines, cards, secondary buttons, dividers |
| `border.subtle` | `#EFEEEA` | `#2F2E2C` | Lines inside a panel (header and footer separators) |
| `border.strong` | `#949189` | `#7A7870` | Edges of text fields, dropdowns, checkboxes, switch track (off) |
| `border.dashed` | `#D3D1CA` | `#4A4945` | Drop zone outline |
| `text.primary` | `#1C1C1A` | `#F2F1EC` | Body text, labels, icons at rest |
| `text.secondary` | `#55544E` | `#C4C2B9` | Field labels, status bar, inactive tabs and segments |
| `text.muted` | `#66655F` | `#A3A197` | Meta text, placeholders, chevrons, page numbers |
| `text.disabled` | `#AAA8A0` | `#6E6C66` | Disabled controls (exempt from contrast rules) |
| `accent` | `#C2603E` | `#D97757` | Icons and fills: active-tool icon, frames, switch on, slider, focus ring, modified dot. **Not for text on light.** |
| `accent.text` | `#A54D2F` | `#E8957A` | Accent-coloured text: active ribbon tab, selected tool label, links, selected page number |
| `accent.soft` | `#F8E6DD` | `#4A3027` | Background of the active tool, selected thumbnail, icon tiles |
| `primary.bg` | `#B5532F` | `#D97757` | Primary button |
| `primary.hover` | `#A04A2A` | `#E3876A` | Primary button, hovered |
| `primary.text` | `#FFFFFF` | `#1A1A19` | Text and icons on the primary button |
| `success.bg` / `success.text` | `#EEF6EE` / `#1F5A2A` | `#1F3324` / `#9FD3A8` | "This can be done safely" notes |
| `warning.bg` / `warning.text` | `#F8EBC9` / `#7A5A12` | `#3A3020` / `#E3C27A` | "This cannot be done, here is why" notes |
| `danger.bg` / `danger.text` | `#F6DCDC` / `#A3302B` | `#3E2424` / `#E58A80` | Delete, errors, "not found" |
| `overlay.bg` / `overlay.text` | `#1C1C1A` / `#FFFFFF` | same | Chips and tooltips drawn over the page (the page is white in both themes) |
| `selection.text` | `rgba(90,140,235,.32)` | same | Selected text on the page |
| `window.close.hover` | `#C42B1C` (text `#FFFFFF`) | same | Close button hover (Windows convention) |

**Category tints** (icon tiles on the start screen; background / icon):

| Tint | Light | Dark |
|---|---|---|
| `tint.accent` | `#F8E6DD` / `#A54D2F` | `#3D2A22` / `#E8957A` |
| `tint.blue` | `#DCEBF8` / `#2D6A9F` | `#1F2E3D` / `#8DB9E8` |
| `tint.purple` | `#E6E1F5` / `#5B4A9E` | `#2C2740` / `#B7A8EA` |
| `tint.green` | `#DFF0E2` / `#2F6B3B` | `#1F3324` / `#9FD3A8` |
| `tint.amber` | `#F8EBC9` / `#7A5A12` | `#3A3020` / `#E3C27A` |
| `tint.neutral` | `#EFEEEA` / `#4A4842` | `#33322F` / `#C4C2B9` |
| `tint.red` | `#F6DCDC` / `#9E2F2F` | `#3E2424` / `#EE9A92` |

**Checked contrast** (WCAG 2.1, rounded):

| Pair | Ratio | | Pair | Ratio |
|---|---|---|---|---|
| text.primary on surface (light) | 17.1 | | text.primary on surface (dark) | 13.4 |
| text.secondary on chrome (light) | 7.0 | | text.secondary on surface (dark) | 8.5 |
| text.muted on chrome (light) | 5.4 | | text.muted on chrome (dark) | 6.4 |
| accent.text on surface (light) | 5.7 | | accent.text on surface (dark) | 6.5 |
| accent.text on accent.soft (light) | 4.7 | | accent.text on accent.soft (dark) | 5.2 |
| primary.text on primary.bg (light) | 5.0 | | primary.text on primary.bg (dark) | 5.6 |
| success.text on success.bg (light) | 7.5 | | success.text on success.bg (dark) | 7.9 |
| danger.text on surface (light) | 7.0 | | danger.text on surface (dark) | 6.0 |
| accent (icon) on surface (light) | 4.2 | | accent (icon) on surface (dark) | 4.9 |
| border.strong on surface (light) | 3.2 | | border.strong on surface (dark) | 3.4 |

Every tint pair above is at least 4.7:1.

### 2.2 Typography

Typeface: **Inter** (SIL Open Font License 1.1), static weights Regular 400, Medium 500, SemiBold 600 and Bold 700.
DejaVu Sans stays behind it as the fallback for symbols and scripts Inter lacks (⌘ ⇧ ⌥ ⌃, Greek, Cyrillic and so on).
Page content keeps its own fonts.

| Style | Size | Weight | Line height | Used for |
|---|---|---|---|---|
| `display` | 24 | 700 | 1.25, tracking −0.01 em | Drop-zone heading on the start screen |
| `section` | 15 | 700 | 1.3 | Section headings on the start screen ("Quick tasks", "Recent") |
| `panel-title` | 14 | 600 | 1.3 | Panel headings ("Text"), document name |
| `body` | 13 | 400 | 1.45 | Default: ribbon labels, tabs, fields, menus |
| `body-strong` | 13 | 600 | 1.45 | Active ribbon tab, active tool, selected segment, card titles (700 on the start screen) |
| `label` | 12 | 500 | 1.35 | Field labels (`text.secondary`) |
| `small` | 12 | 400 | 1.35 | Status bar, notes, meta lines |
| `caption` | 11.5 | 400/600 | 1.35 | Thumbnail page numbers, inline-edit chip |
| `micro` | 11 | 500 | 1.2 | Keyboard chips, badges |

Numbers that change (zoom, page number, measurements, sizes) use tabular figures.

### 2.3 Spacing

Base unit 2 px; use only these steps: **2, 4, 6, 8, 10, 12, 14, 16, 18, 22, 24, 26, 28, 32, 36**.
Usual gaps: icon to label 7 (ribbon) or 8 (tabs, fields); buttons in a row 2 (ribbon) or 8 (footers); fields in a form
16 vertical, 10 horizontal; label to field 6.

### 2.4 Radius

| Token | px | Used for |
|---|---|---|
| `radius.xs` | 3 | Page thumbnails, inline-edit frame |
| `radius.sm` | 5 | Status-bar controls, small chip buttons |
| `radius.segment` | 6 | Segment inside a segmented control, tooltips |
| `radius.md` | 7 | Fields, dropdowns, icon buttons, inline-edit chip |
| `radius.lg` | 8 | Ribbon buttons, buttons, tabs (top corners only), segmented track, notes |
| `radius.xl` | 10 | Menus, selected thumbnail, start-screen navigation items, icon tiles |
| `radius.2xl` | 12 | Dialogs and windows, large primary button |
| `radius.3xl` | 14 | Task cards, recent-file thumbnail area |
| `radius.4xl` | 18 | Drop zone |

### 2.5 Lines and elevation

* Hairline borders are 1 px. The drop zone's dashed border is 1.5 px. Frames and focus rings are 1.5–2 px.
* Shadows (x y blur spread colour):

| Token | Light | Dark |
|---|---|---|
| `shadow.page` | `0 1 3 0 rgba(0,0,0,.10)` + `0 10 30 0 rgba(0,0,0,.08)` | `0 1 3 0 rgba(0,0,0,.40)` + `0 20 50 0 rgba(0,0,0,.50)` |
| `shadow.thumb` | `0 1 3 0 rgba(0,0,0,.14)` | `0 1 3 0 rgba(0,0,0,.50)` |
| `shadow.raised` | `0 1 2 0 rgba(0,0,0,.08)` | `0 1 2 0 rgba(0,0,0,.40)` |
| `shadow.popover` | `0 8 24 0 rgba(0,0,0,.12)` | `0 10 30 0 rgba(0,0,0,.45)` |

`shadow.raised` is for the selected segment and the active start-screen navigation item; `shadow.popover` for
menus, tooltips, dialogs and the inline-edit chip.

### 2.6 Icons

* BergPDF's own icons (`apps/desktop/src/icons.rs`): 24 × 24 grid, outline only, stroke 1.6 units, round caps and
  joins, no fills except small solid dots.
* Sizes: 18 in the ribbon and panels, 17 in the title bar, 15 in tabs, the status bar and chips, 14 for chevrons and
  close crosses, 30 in the drop zone (stroke 1.5).
* Colour follows the text: `text.primary` at rest, `accent` when active (the label next to it is `accent.text`),
  `text.muted` for chevrons and close crosses.
* An icon-only button always has a tooltip and an accessible name.

### 2.7 Motion

Short and functional: hover and press colour 80 ms, panels opening or collapsing 150 ms ease-out, nothing else moves.
Follow the system's reduced-motion setting where it is known.

---

## 3. Window layout

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ Title bar 40   logo BergPDF  [open][save][undo][redo]   [ Search… Ctrl K ]  – □ ✕ │  bg.chrome
│ Ribbon tabs 34 File Home [Edit] Comment Forms Sign … View      Single-line ribbon ◉ │  bg.chrome
├───────────────────────────────────────────────────────────────────────────────┤
│ Ribbon 52      [Edit text] Add text  Add image │ Font ▾  11 ▾  B I ■ │ …   More ▾  │  bg.surface
├───────────────────────────────────────────────────────────────────────────────┤
│ Doc tabs 36    [report.pdf ● ×] helvetica_lines.pdf  +                          │  bg.chrome
├────────────┬───────────────────────────────────────────────────┬──────────────┤
│ Left 220   │ Canvas                                            │ Right 300    │
│ bg.panel   │ bg.canvas                                         │ bg.surface   │
│ Pages|Book-│            ┌──────────────┐                       │ Properties|… │
│ marks|Find │            │     page     │                       │              │
│ thumbnails │            └──────────────┘                       │ form         │
├────────────┴───────────────────────────────────────────────────┴──────────────┤
│ Status 30   hint …                     Page [1] of 3 · 210 × 297 mm · ▯▤▯▯ · ━●━ 100% │  bg.chrome
└───────────────────────────────────────────────────────────────────────────────┘
```

* Rows from top: title bar, ribbon tabs, ribbon, document tabs, body, status bar. The ribbon tabs belong to the
  ribbon: the active tab joins the ribbon row below it.
* Side panels can be collapsed (as today). Their widths can be dragged: left 180–360, right 260–420.
* At window widths below 1100 the right panel collapses first, then the left panel; the ribbon moves items into
  **More** from the right.

### Density

The existing density preference (`Density`) changes these sizes; everything else stays the same.

| | Compact | Comfortable (default) | Touch |
|---|---|---|---|
| Title bar | 36 | 40 | 48 |
| Ribbon tab row / tab | 30 / 28 | 34 / 32 | 42 / 40 |
| Ribbon row / button | 40 / 28 | 52 / 34 | 64 / 44 |
| Ribbon labels | icons only, tooltips | icon + label | icon + label |
| Document tab row / tab | 32 / 26 | 36 / 30 | 44 / 38 |
| Field and button height | 28 | 32–34 | 44 |
| Status bar | 26 | 30 | 36 |
| Icon size (ribbon) | 16 | 18 | 22 |

---

## 4. Components

### 4.1 Title bar

* Height 40, `bg.chrome`, no bottom line (the ribbon tab row continues the same background).
* Left: logo 22 (outline `text.primary`, zigzag `accent`), 6 gap, "BergPDF" `body-strong`, 10 gap, then the quick-access
  buttons Open, Save, Undo, Redo: icon buttons 30 × 30, `radius.md`, icon 17. Unavailable ones use `text.disabled`.
* Centre (centred on the window, not on the free space): the **command search**, 400 × 28, `bg.surface`,
  `border.default`, `radius.lg`, padding 0 8 0 10. Search icon 15, placeholder in `text.muted`:
  *Search commands, e.g. "rotate" or "OCR"*, keyboard chip "Ctrl K" (`micro`, `bg.track`, `border.default`, `radius.sm`,
  padding 1 6). On macOS the chip reads "⌘K". Clicking it opens the existing command palette.
* Right: window controls 46 × 40, glyphs 10 × 10 with 1 px lines. Hover `bg.hover`; Close hovers `window.close.hover`.
  On macOS the system buttons sit on the left and the logo moves right of them.

### 4.2 Ribbon tabs

* Row height 34, padding 0 10, tabs aligned to the bottom, gap 2.
* Tab: height 32, padding 0 14, top corners `radius.lg`, `body`.
  * Rest: transparent, `text.primary`. Hover: `bg.hover`.
  * Active: `bg.surface`, 1 px `border.default` on top, left and right, no bottom border, moved down 1 px so it
    covers the ribbon row's top line; text `accent.text`, weight 600.
* Order and rules as today: File, Home, Edit, Comment, Forms (only when the document has form fields), Sign, Redact,
  Copilot, Measure (Professional workspace, or while a measure tool is active), Organize, View.
* Far right: the switch **Single-line ribbon** (`small`, `text.muted`, switch 28 × 16, see 4.11). Off shows the classic
  ribbon (large icon above label, group captions), which stays available for people who prefer it.

### 4.3 Ribbon row (single-line)

* Height 52, `bg.surface`, 1 px `border.default` above and below, padding 0 10, content vertically centred.
* **Ribbon button**: height 34, padding 0 11, gap 7, `radius.lg`, icon 18 + label (`body`).
  * Rest: transparent, `text.primary`. Hover: `bg.hover`. Pressed: `bg.pressed`.
  * Active (current tool or a toggle that is on): `accent.soft`, icon `accent`, label `accent.text` weight 600.
  * Disabled: `text.disabled`, no hover.
* **Group separator**: 1 × 26, `border.default`, margin 0 8. Groups are what the classic ribbon calls a group; there are
  no captions in single-line mode.
* **Inline controls** (when the active tab or tool has options, e.g. Edit text): dropdown 30 high, `bg.field`,
  1 px `border.strong`, `radius.md`, padding 0 8 0 10, chevron 14 `text.muted`. A font dropdown shows the name in that
  font (170 wide); size dropdown 70 wide. Bold and Italic are icon buttons 32 × 32. The colour button is a 16 × 16 swatch
  (`radius` 4) with a chevron.
* **More ▾** is always last, right-aligned, `text.muted`. When the row is too narrow, buttons move into it from the
  right, a whole group at a time. The ribbon never scrolls sideways.

### 4.4 Document tabs

* Row height 36, `bg.chrome`, padding 0 10, tabs aligned to the bottom, gap 4. The body's top border (1 px
  `border.default`) closes the row.
* Tab: height 30, padding 0 8 0 12, gap 8, top corners `radius.lg`, `body`.
  * Active: `bg.surface`, 1 px `border.default` on top, left and right, weight 500; file icon 15 in `accent`.
  * Inactive: transparent, `text.secondary`, file icon in `text.secondary`. Hover `bg.hover`.
  * Unsaved changes: a 6 × 6 dot in `accent` after the name. Close cross 13, `text.muted`, shown on the active tab and
    on hover. Middle-click closes.
* "+" (icon button 28 × 28) opens the start screen in a new tab.

### 4.5 Segmented control

Used for the side-panel switchers.

* Track: `bg.track`, `radius.lg`, padding 3. Segments share the width equally, height 26, `radius.segment`.
* Selected: `bg.segment`, `shadow.raised`, weight 600, `text.primary`. Others: transparent, `text.secondary`,
  hover `bg.hover`.
* With four segments the label size drops to 12.

### 4.6 Left panel

* 220 wide, `bg.panel`, 1 px `border.default` on the right. Switcher (padding 10): **Pages | Bookmarks | Search**.
* Thumbnails: 110 wide at the page's aspect ratio, `radius.xs`, `shadow.thumb`, 12 apart, centred.
  Under each: page number `caption`, `text.muted`.
* Current page: wrapper with padding 8 22, `radius.xl`, `accent.soft`; its page number `accent.text`, weight 600.
  Hover on others: wrapper `bg.hover`.

### 4.7 Canvas and pages

* `bg.canvas`. 28 above the first page, 16 between pages in continuous view, pages centred.
* Pages use `shadow.page` and square corners. Selected text: `selection.text`.

### 4.8 Inline text editing

* The paragraph being edited gets a frame 1.5 px `accent`, `radius.xs`, 6 8 outside the text, filled with `accent` at
  4 % opacity. The caret is 1.5 px wide in the text colour.
* A chip floats 8 above the frame's top-left corner: height 26, `overlay.bg`, `overlay.text`, `caption`,
  `radius.md`, padding 0 4 0 10, `shadow.popover`. It shows the font and size ("Liberation Serif · 10.5 pt") and a
  **Done** button (height 20, `radius.sm`, `primary.bg`, `primary.text`, 11 / 600).
* Status bar hint while editing: *Type to change the text. Esc stops editing.*

### 4.9 Right panel and forms

* 300 wide, `bg.surface`, 1 px `border.default` on the left. Switcher (padding 10, `border.subtle` below):
  **Properties | Comments | Copilot**, plus **Measure** before Copilot when the Measure tab is available.
* Content padding 16, sections 16 apart.
* Header: icon 18 in `accent`, title `panel-title`, context in `small` `text.muted` ("· paragraph 2").
* **Note**: padding 10 12, `radius.lg`, gap 10, icon 16, `small` with line height 1.45. Success, warning and danger
  variants use their `*.bg` / `*.text` tokens. Example: *This font can be edited safely. Changes stay in the original
  font.*
* **Field**: label `label` above, 6 gap; control height 32, `bg.field`, 1 px `border.strong`, `radius.md`, padding 0 10.
  Two fields on one row: 10 apart, equal width. Toggle buttons (Bold, Italic) 36 × 32 with `border.default`.
* **Footer**: `border.subtle` above, padding 14 16, gap 8: primary button (fills the width) and a secondary button.

### 4.10 Buttons

| Kind | Fill | Border | Text | Height | Padding | Radius |
|---|---|---|---|---|---|---|
| Primary | `primary.bg` (hover `primary.hover`) | none | `primary.text`, 600 | 34 | 0 14 | `radius.lg` |
| Primary, large (start screen) | as primary | none | `primary.text`, 14 / 700 | 42 | 0 18 | `radius.2xl` |
| Secondary | `bg.surface` (hover `bg.hover`) | 1 px `border.default` | `text.primary` | 34 | 0 14 | `radius.lg` |
| Ghost | transparent (hover `bg.hover`) | none | `text.primary` | 32–34 | 0 10 | `radius.md` |
| Icon | transparent (hover `bg.hover`) | none | icon `text.primary` | 30 or 32 square | none | `radius.md` |
| Danger | as secondary | 1 px `border.default` | `danger.text` | 34 | 0 14 | `radius.lg` |

One primary button per view. Pressed: `bg.pressed` (secondary, ghost, icon) or 8 % darker (primary).

### 4.11 Switch, slider, checkbox

* Switch: track 28 × 16, `radius` 8; knob 12 white, 2 from the edge. On: track `accent`. Off: track `border.strong`.
* Slider: track 4 high in `border.default`, filled part and handle (14 round, white with 1 px `accent`) in `accent`.
* Checkbox: 16 × 16, `radius` 4, 1 px `border.strong`; checked: `accent` fill with a white check.

### 4.12 Status bar

* Height 30, `bg.chrome`, 1 px `border.default` above, padding 0 12, gap 14, `small` `text.secondary`.
* Left (fills the space): the contextual hint, as today.
* Right: **Page [n] of N** (input 28 × 20, `bg.field`, `border.default`, `radius.sm`, centred), page size
  ("210 × 297 mm"), page layout (three icon buttons 24 × 22, icons 14: single page, continuous, two pages; current one
  `accent.soft` + `accent`), zoom slider 110 wide plus the value (36 wide, tabular, `text.primary`).

### 4.13 Menus, tooltips, dialogs, focus

* Menus and the command palette: `bg.surface`, 1 px `border.default`, `radius.xl`, `shadow.popover`, padding 8; rows 30
  high, `radius.md`, hover `bg.hover`; shortcuts right-aligned in `text.muted`.
* Tooltips: `overlay.bg`, `overlay.text`, `small`, `radius.segment`, padding 4 8, delay 0.35 s (as today).
* Dialogs: `bg.surface`, `radius.2xl`, `shadow.popover`, padding 16, title `panel-title`, buttons right-aligned
  (primary last).
* Keyboard focus: 2 px ring in `accent`, 2 outside the control, following its radius plus 2. Only shown for
  keyboard focus, not on mouse click.

---

## 5. Start screen

Shown when no document is open, and in a new tab opened with "+". It replaces the current welcome screen.

### 5.1 Layout

* Title bar as in 4.1, with only the Open button in quick access. No ribbon, no document tab row, no status bar.
* Background `bg.chrome`. Left navigation 228 wide; main area padding 14 36 24 24, sections 26 apart.

### 5.2 Navigation

* Items: **Home**, **Recent**, **Pinned**, **Signatures and stamps**; at the bottom **Preferences**.
* Item: height 38, padding 0 12, gap 10, `radius.xl`, icon 18, `body` weight 500, `text.primary`.
  Active: `bg.surface`, `shadow.raised`, weight 700, icon in `accent`. Hover: `bg.hover`.
* Below Preferences, 8 apart: a privacy note, padding 10 12, `radius.xl`, `bg.track`, `small` `text.secondary`, shield
  icon 16 in `success.text`: *Offline. No account. Your files stay on this computer.*

### 5.3 Drop zone

* Padding 28 32, `bg.panel`, 1.5 px dashed `border.dashed`, `radius.4xl`, items in one row with gap 28.
* Icon tile 64 × 64, `radius` 16, `accent.soft`, upload icon 30 in `accent.text`.
* Heading `display`: *Drop a PDF or picture here*. Below, 6 apart, 14 px `text.secondary`:
  *PDF, PNG and JPEG. Pictures open as a one-page PDF.*
* Right: large primary button **Open a file** with folder icon 17, then a keyboard chip "Ctrl O".
* While a file is dragged over the window: border 2 px solid `accent`, fill `accent.soft`, heading *Drop to open*.

### 5.4 Quick tasks

* Heading `section`: **Quick tasks**. Grid of 4 columns, gap 10 (3 columns below 1100 wide).
* Card: `bg.surface`, 1 px `border.default`, `radius.3xl`, padding 12 14, gap 12; hover `bg.hover`.
  Icon tile 38 × 38, `radius.xl`, icon 19, tint as listed. Title `body` 700; description `small` `text.muted`, one line
  with ellipsis.
* Each task first asks for a file (or uses the selected recent file), opens it and then starts the action.

| Title | Description | Tint | Action after opening |
|---|---|---|---|
| Merge PDFs | Combine files into one | accent | Organize › Merge, asking for the other files |
| Make a scan searchable | Text recognition, on this computer | blue | OCR the document |
| Sign a document | Handwritten or with a certificate | purple | Sign tab |
| Fill in a form | Type into the form fields | green | Forms tab, Fill in tool |
| Shrink a file | Save as optimized | amber | Save As Optimized dialog |
| Convert to PDF/A | For long-term archiving | neutral | PDF/A dialog |
| Add a password | Protect who can open it | neutral | Protection dialog |
| Redact | Remove content for real | red | Redact tab |

### 5.5 Recent files

* Heading row: **Recent** (`section`) and, right-aligned, the link **Clear list** (`small` 600, `accent.text`).
* Grid of 6 columns, gap 14 (4 below 1100 wide). Up to 12 files (the current limit); Home shows the first 6.
* Card: thumbnail area 150 high, `bg.canvas`, `radius.3xl`; the first page sits at the bottom centre with square top
  corners `radius.xs` and `shadow.thumb`, 96 × 128 for portrait and 150 × 112 for landscape. Below, 10 apart: name
  (`body` 700, one line, ellipsis) and meta (`small` `text.muted`): parent folder and when it was last opened,
  e.g. *Documents · 10 minutes ago*.
* Pinned files show a pin badge in the thumbnail's top-right corner (24 × 24, `radius.lg`, `bg.surface`, icon 14 in
  `accent.text`) and are listed first.
* Hover: thumbnail area `bg.pressed`. Right-click: Open, Pin / Unpin, Show in folder, Remove from list.
* A file that no longer exists: thumbnail at 40 % opacity, meta *Not found* in `danger.text`; clicking offers to remove it.

### 5.6 Data the start screen needs

New compared to today (`recent_files` holds paths only):

* **Last opened time** per recent file: store entries as `{ path, opened_at }`, reading old string lists as entries
  without a time (serde default), so existing preferences keep working.
* **Pins**: `pinned_files: Vec<String>` in the preferences.
* **Thumbnails**: rendered from page 1 when a document is closed or saved, 300 px wide, stored in the user data folder
  (`thumbnails/<hash of the path>.png`). They never leave the computer; removing a file from the list or clearing the
  list deletes its thumbnail. Password-protected documents get no thumbnail (a lock icon instead).

---

## 6. Wording

* Sentence case for every label, title and button ("Single-line ribbon", "Save as optimized").
* Verbs for actions, nouns for places. Say what will happen; avoid jargon or explain it ("Text recognition").
* Hints in the status bar are one or two short sentences ending with a full stop.
* New strings go into `apps/desktop/src/i18n` for Dutch and German at the same time as English.

---

## 7. Accessibility checklist

* Text 4.5:1 or more, icons and the edges of fields and checkboxes 3:1 or more, in both themes (section 2.1).
* Hit targets at least 30 × 30 in Comfortable and 44 × 44 in Touch.
* Everything reachable with the keyboard: Ctrl K for commands, Tab through the ribbon tabs, ribbon, panels and status
  bar; arrow keys within the ribbon tabs and segmented controls; visible focus ring (4.13).
* Never colour alone: active tools also get weight 600 and a fill; "not found" has text, not just red.
* Icon-only buttons have a tooltip and an AccessKit name.

---

## 8. Implementing it in BergPDF (egui 0.36)

### 8.1 Palette (`apps/desktop/src/theme.rs`)

Keep the existing field names so the rest of the code keeps compiling, and add the new ones:

| Existing field | Becomes |
|---|---|
| `chrome` | `bg.chrome` |
| `panel` | `bg.panel` |
| `canvas` | `bg.canvas` |
| `card` | `bg.surface` |
| `text` | `text.primary` |
| `text_dim` | `text.secondary` |
| `accent`, `accent_soft` | `accent`, `accent.soft` |
| `border` | `border.default` |
| `hover` | `bg.hover` |
| `danger` | `danger.text` |
| `page_shadow` | the larger layer of `shadow.page` |

```rust
pub struct Palette {
    pub chrome: Color32, pub panel: Color32, pub canvas: Color32, pub card: Color32,
    pub field: Color32, pub track: Color32, pub segment: Color32,
    pub hover: Color32, pub pressed: Color32,
    pub border: Color32, pub border_subtle: Color32, pub border_strong: Color32, pub border_dashed: Color32,
    pub text: Color32, pub text_dim: Color32, pub text_muted: Color32, pub text_disabled: Color32,
    pub accent: Color32, pub accent_text: Color32, pub accent_soft: Color32,
    pub primary: Color32, pub primary_hover: Color32, pub on_primary: Color32,
    pub success_bg: Color32, pub success: Color32,
    pub warning_bg: Color32, pub warning: Color32,
    pub danger_bg: Color32, pub danger: Color32,
    pub page_shadow: Color32,
    pub dark: bool,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        chrome: rgb(0xF6F5F2), panel: rgb(0xFAFAF8), canvas: rgb(0xE9E8E3), card: rgb(0xFFFFFF),
        field: rgb(0xFFFFFF), track: rgb(0xEFEEEA), segment: rgb(0xFFFFFF),
        hover: rgb(0xEFEEEA), pressed: rgb(0xE6E4DE),
        border: rgb(0xE2E0DA), border_subtle: rgb(0xEFEEEA), border_strong: rgb(0x949189), border_dashed: rgb(0xD3D1CA),
        text: rgb(0x1C1C1A), text_dim: rgb(0x55544E), text_muted: rgb(0x66655F), text_disabled: rgb(0xAAA8A0),
        accent: rgb(0xC2603E), accent_text: rgb(0xA54D2F), accent_soft: rgb(0xF8E6DD),
        primary: rgb(0xB5532F), primary_hover: rgb(0xA04A2A), on_primary: rgb(0xFFFFFF),
        success_bg: rgb(0xEEF6EE), success: rgb(0x1F5A2A),
        warning_bg: rgb(0xF8EBC9), warning: rgb(0x7A5A12),
        danger_bg: rgb(0xF6DCDC), danger: rgb(0xA3302B),
        page_shadow: Color32::from_black_alpha(20),
        dark: false,
    };
    pub const DARK: Palette = Palette {
        chrome: rgb(0x1E1E1C), panel: rgb(0x232321), canvas: rgb(0x141413), card: rgb(0x262624),
        field: rgb(0x1C1C1B), track: rgb(0x1C1C1B), segment: rgb(0x34332F),
        hover: rgb(0x302F2D), pressed: rgb(0x383734),
        border: rgb(0x383734), border_subtle: rgb(0x2F2E2C), border_strong: rgb(0x7A7870), border_dashed: rgb(0x4A4945),
        text: rgb(0xF2F1EC), text_dim: rgb(0xC4C2B9), text_muted: rgb(0xA3A197), text_disabled: rgb(0x6E6C66),
        accent: rgb(0xD97757), accent_text: rgb(0xE8957A), accent_soft: rgb(0x4A3027),
        primary: rgb(0xD97757), primary_hover: rgb(0xE3876A), on_primary: rgb(0x1A1A19),
        success_bg: rgb(0x1F3324), success: rgb(0x9FD3A8),
        warning_bg: rgb(0x3A3020), warning: rgb(0xE3C27A),
        danger_bg: rgb(0x3E2424), danger: rgb(0xE58A80),
        page_shadow: Color32::from_black_alpha(128),
        dark: true,
    };
}
```

Add a unit test that computes the contrast of the pairs in section 2.1 and fails below 4.5 (text) or 3.0 (icons,
field edges), so later colour tweaks cannot quietly break readability.

### 8.2 Visuals (`theme::apply`)

* `panel_fill` = `card`, `window_fill` = `card`, `extreme_bg_color` = `field`, `faint_bg_color` = `track`.
* `selection.bg_fill` = `accent_soft`, `selection.stroke` = 1 px `accent`; `hyperlink_color` = `accent_text`.
* `window_corner_radius` 12, `menu_corner_radius` 10, widget corner radius 7 (the ribbon and tabs paint their own 8).
* `window_shadow` and `popup_shadow`: `Shadow { offset: [0, 8], blur: 24, spread: 0, color: black at 12 % (light) /
  45 % (dark) }`.
* Buttons: `inactive.weak_bg_fill` = `card`, `inactive.bg_stroke` = 1 px `border`; `hovered` fills `hover`;
  `active` fills `pressed`. Text fields, checkboxes and sliders are drawn through `ui_kit` helpers with a 1 px
  `border_strong` edge, because egui uses one `bg_stroke` for buttons and inputs alike.
* Spacing (Comfortable): `button_padding` (11, 8), `interact_size.y` 32, `item_spacing` (6, 5), `menu_margin` 8,
  `window_margin` 16, `slider_width` 110. Extend `Metrics` with the heights from the density table in section 3
  (`title_h`, `ribbon_tabs_h`, `ribbon_h`, `ribbon_button_h`, `doc_tabs_h`, `status_h`).

### 8.3 Fonts

* Bundle the four static Inter weights (OFL 1.1, add them to `docs/THIRD_PARTY_LICENSES.md` and `docs/DEPENDENCIES.md`)
  next to the existing fonts, and register them in `theme::install_fonts`: Regular as the first `Proportional` font,
  and Medium, SemiBold and Bold as `FontFamily::Name("inter-500" | "inter-600" | "inter-700")`, each with DejaVu Sans
  as fallback. egui picks weights by family, so add a helper `theme::font(weight, size) -> FontId`.
* Text styles: `Body` 13, `Button` 13, `Small` 11.5, `Heading` 15 (Bold family), `Monospace` 12.
* Tracking (the −0.01 em on `display`) via `TextFormat::extra_letter_spacing`.
* egui cannot switch on OpenType features, so tabular figures need a copy of Inter Regular with `tnum` made the
  default (for example with `pyftfeatfreeze`), registered as `inter-tnum` and used for numbers.

### 8.4 Title bar

Merging the command search into the title bar means drawing our own title bar (`ViewportBuilder::with_decorations(false)`,
drag with `ViewportCommand::StartDrag`, resize with `ViewportCommand::BeginResize` along the edges). Two known costs:
Windows 11 Snap Layouts on the maximise button need native hit-testing that winit does not offer, and Linux window
managers vary. Recommended order: first keep the native title bar and use this 40-high row as the app's top row (it
looks nearly the same); switch to a custom title bar per platform later, if it proves solid.

### 8.5 Where the work is

| Step | Files | What |
|---|---|---|
| 1. Tokens and fonts | `theme.rs`, `main.rs`, fonts in `crates/pdf-engine/assets/fonts` or `apps/desktop/assets` | Palette, Visuals, Inter, contrast test |
| 2. Ribbon | `chrome.rs` (`ribbon`, `cmd_button`, `group`), `prefs.rs` | Tab style, single-line row, More overflow, `ribbon_style: SingleLine \| Classic` preference |
| 3. Tabs, panels, status bar | `chrome.rs` (`tab_bar`, `status_bar`), `sidebar.rs`, `ui_kit.rs` | Document tabs, segmented control, thumbnails, status bar controls |
| 4. Inline edit | `inline_edit.rs`, `contentedit.rs` | Frame and chip |
| 5. Start screen | `chrome.rs` (`welcome`), `prefs.rs`, a thumbnail cache | Section 5, including the data in 5.6 |
| 6. Title bar | `main.rs`, `chrome.rs` | Section 8.4 |
| 7. Screens and docs | `debug_shots.rs`, `tools/website-shots`, `docs/screenshots`, `README.md` | New screenshots in both themes and all three languages |

---

## 9. Tokens for other apps

### 9.1 JSON

```json
{
  "font": { "family": "Inter", "fallback": "DejaVu Sans" },
  "color": {
    "light": {
      "bg.chrome": "#F6F5F2", "bg.surface": "#FFFFFF", "bg.panel": "#FAFAF8", "bg.canvas": "#E9E8E3",
      "bg.field": "#FFFFFF", "bg.track": "#EFEEEA", "bg.segment": "#FFFFFF", "bg.hover": "#EFEEEA", "bg.pressed": "#E6E4DE",
      "border.default": "#E2E0DA", "border.subtle": "#EFEEEA", "border.strong": "#949189", "border.dashed": "#D3D1CA",
      "text.primary": "#1C1C1A", "text.secondary": "#55544E", "text.muted": "#66655F", "text.disabled": "#AAA8A0",
      "accent": "#C2603E", "accent.text": "#A54D2F", "accent.soft": "#F8E6DD",
      "primary.bg": "#B5532F", "primary.hover": "#A04A2A", "primary.text": "#FFFFFF",
      "success.bg": "#EEF6EE", "success.text": "#1F5A2A", "warning.bg": "#F8EBC9", "warning.text": "#7A5A12",
      "danger.bg": "#F6DCDC", "danger.text": "#A3302B", "overlay.bg": "#1C1C1A", "overlay.text": "#FFFFFF"
    },
    "dark": {
      "bg.chrome": "#1E1E1C", "bg.surface": "#262624", "bg.panel": "#232321", "bg.canvas": "#141413",
      "bg.field": "#1C1C1B", "bg.track": "#1C1C1B", "bg.segment": "#34332F", "bg.hover": "#302F2D", "bg.pressed": "#383734",
      "border.default": "#383734", "border.subtle": "#2F2E2C", "border.strong": "#7A7870", "border.dashed": "#4A4945",
      "text.primary": "#F2F1EC", "text.secondary": "#C4C2B9", "text.muted": "#A3A197", "text.disabled": "#6E6C66",
      "accent": "#D97757", "accent.text": "#E8957A", "accent.soft": "#4A3027",
      "primary.bg": "#D97757", "primary.hover": "#E3876A", "primary.text": "#1A1A19",
      "success.bg": "#1F3324", "success.text": "#9FD3A8", "warning.bg": "#3A3020", "warning.text": "#E3C27A",
      "danger.bg": "#3E2424", "danger.text": "#E58A80", "overlay.bg": "#1C1C1A", "overlay.text": "#FFFFFF"
    }
  },
  "radius": { "xs": 3, "sm": 5, "segment": 6, "md": 7, "lg": 8, "xl": 10, "2xl": 12, "3xl": 14, "4xl": 18 },
  "space": [2, 4, 6, 8, 10, 12, 14, 16, 18, 22, 24, 26, 28, 32, 36],
  "type": {
    "display": [24, 700, 1.25], "section": [15, 700, 1.3], "panel-title": [14, 600, 1.3],
    "body": [13, 400, 1.45], "body-strong": [13, 600, 1.45], "label": [12, 500, 1.35],
    "small": [12, 400, 1.35], "caption": [11.5, 400, 1.35], "micro": [11, 500, 1.2]
  }
}
```

### 9.2 CSS custom properties

```css
:root {
  --font-sans: "Inter", "DejaVu Sans", system-ui, sans-serif;
  --bg-chrome: #F6F5F2; --bg-surface: #FFFFFF; --bg-panel: #FAFAF8; --bg-canvas: #E9E8E3;
  --bg-field: #FFFFFF; --bg-track: #EFEEEA; --bg-segment: #FFFFFF; --bg-hover: #EFEEEA; --bg-pressed: #E6E4DE;
  --border-default: #E2E0DA; --border-subtle: #EFEEEA; --border-strong: #949189; --border-dashed: #D3D1CA;
  --text-primary: #1C1C1A; --text-secondary: #55544E; --text-muted: #66655F; --text-disabled: #AAA8A0;
  --accent: #C2603E; --accent-text: #A54D2F; --accent-soft: #F8E6DD;
  --primary-bg: #B5532F; --primary-hover: #A04A2A; --primary-text: #FFFFFF;
  --success-bg: #EEF6EE; --success-text: #1F5A2A; --warning-bg: #F8EBC9; --warning-text: #7A5A12;
  --danger-bg: #F6DCDC; --danger-text: #A3302B; --overlay-bg: #1C1C1A; --overlay-text: #FFFFFF;
  --shadow-page: 0 1px 3px rgba(0,0,0,.10), 0 10px 30px rgba(0,0,0,.08);
  --shadow-raised: 0 1px 2px rgba(0,0,0,.08);
  --shadow-popover: 0 8px 24px rgba(0,0,0,.12);
  --radius-xs: 3px; --radius-sm: 5px; --radius-segment: 6px; --radius-md: 7px; --radius-lg: 8px;
  --radius-xl: 10px; --radius-2xl: 12px; --radius-3xl: 14px; --radius-4xl: 18px;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg-chrome: #1E1E1C; --bg-surface: #262624; --bg-panel: #232321; --bg-canvas: #141413;
    --bg-field: #1C1C1B; --bg-track: #1C1C1B; --bg-segment: #34332F; --bg-hover: #302F2D; --bg-pressed: #383734;
    --border-default: #383734; --border-subtle: #2F2E2C; --border-strong: #7A7870; --border-dashed: #4A4945;
    --text-primary: #F2F1EC; --text-secondary: #C4C2B9; --text-muted: #A3A197; --text-disabled: #6E6C66;
    --accent: #D97757; --accent-text: #E8957A; --accent-soft: #4A3027;
    --primary-bg: #D97757; --primary-hover: #E3876A; --primary-text: #1A1A19;
    --success-bg: #1F3324; --success-text: #9FD3A8; --warning-bg: #3A3020; --warning-text: #E3C27A;
    --danger-bg: #3E2424; --danger-text: #E58A80;
    --shadow-page: 0 1px 3px rgba(0,0,0,.40), 0 20px 50px rgba(0,0,0,.50);
    --shadow-raised: 0 1px 2px rgba(0,0,0,.40);
    --shadow-popover: 0 10px 30px rgba(0,0,0,.45);
  }
}
```

For another app, change the four accent tokens (`accent`, `accent.text`, `accent.soft`, `primary.*`) and the logo,
and re-check the contrast table; the neutrals, type, spacing and radii carry over unchanged.

---

## 10. Differences from the mock-ups, and open choices

Differences, on purpose:

* **Field edges** use `border.strong` (the mock-up drew them in the lighter `border.default`), so fields meet 3:1.
* **Start screen** uses direction C's colours and Inter instead of the warmer background and Manrope in its mock-up,
  so the app has one style.
* **Left panel switcher** keeps the existing label **Bookmarks** (the mock-up said "Outline"), so the existing
  translations stay.
* **Right panel** keeps the Measure panel as a fourth segment when measuring is available.
* **Done** on the inline-edit chip uses `primary.bg` for contrast.
* **Dark theme**: the mock-up of direction C is light only; the dark values here are new and checked for contrast.

Differences in the implementation (0.4, 2026-11-03):

* **Bold 700** is drawn with Inter SemiBold: only Regular, Medium and SemiBold are bundled so far.
* **Tabular figures** (§8.3) are not done: making the `tnum` copy of Inter needs `pyftfeatfreeze`, which was not
  available. Numbers use Inter's default figures.
* **Title bar**: the native title bar stays (§8.4, recommended first step); the 40-high row under it carries the logo,
  quick access and the centred command search.
* **Recent files**: the last-opened times are kept as `recent_opened` (path → time) next to the existing
  `recent_files` list instead of `{ path, opened_at }` entries; old preference files read unchanged.
* **Segmented controls** keep equal segments when the labels fit; when a translation is too long they become as wide
  as their labels and the text steps down to 10.5 px rather than being cut off. The side-panel hide button stays next
  to the switcher.
* **Start screen in a tab**: "+" shows the start screen as a *Start* tab after the open documents; the ribbon and
  status bar are hidden while it is shown.
* **Single-line ribbon notes**: the explanations the classic ribbon prints next to some groups (Forms, Sign, Redact,
  Copilot, Edit) become an info button with a tooltip.

Still to decide:

* Custom title bar, and on which platforms (section 8.4).
* Whether the classic ribbon stays as an option long term, or only for one release.
* Inter, or keeping egui's current default font with the same sizes (saves about 1 MB of font files, looks less
  refined).
