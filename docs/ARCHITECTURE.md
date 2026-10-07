# Architecture

```
apps/desktop   egui/eframe + wgpu shell (the only crate that knows about the UI toolkit)
  └─ editor-core  document session, history, commands/shortcuts, view model, tiles, jobs, prefs
       └─ pdf-engine  PDF reading/editing/rendering; no UI, no threads of its own
platform       file dialogs, directories, recovery files, link opening, install channel (rfd, std only)
printing       printers and print jobs: GDI on Windows (the only `unsafe` in the workspace), CUPS `lp` elsewhere
test-support   fixtures (pdf-writer), oracle helpers (poppler), bitmap diff
xtask          quality gate, licence inventory, bench, unsigned dist
```

## pdf-engine

| Module | Responsibility |
|---|---|
| `doc` | `PdfDocument` (lopdf `Document` + original bytes), **transactions** (`Tx`) that journal before/after images of every touched object, rollback on error, `ObjectDelta` for undo/redo, page-tree walk, `PageId` = page object id |
| `serialize` | object writer, **incremental** xref table/stream writer, base-file detection |
| `save` | atomic write (temp → fsync → re-open/validate → rename), external-modification stamp |
| `render` | hayro `Session`, tile rendering, glyph geometry/text extraction |
| `geom` | rotation/crop/flip-safe `pdf_to_view`/`view_to_pdf`, quads |
| `text` | glyph quads, lines, ligature-aware search, selection quads |
| `content`, `textfont`, `pagecontent` | content-stream scanner (byte spans, inline images), text/graphics state simulation, font encode/decode with glyph availability, in-place text/image edits with copy-on-write of shared streams |
| `fontembed`, `imageembed` | embed subset DejaVu as Type0 + ToUnicode; PNG/JPEG → Image XObject |
| `annot` | annotation model ↔ dictionary + appearance streams (regenerable), measurement label placement |
| `pageops` | rotate/delete/reorder/duplicate/import pages, crop box, dead-destination cleanup, `FormPolicy` |
| `forms` | field tree, fill (generated appearances), flatten, duplicate/import with Independent/Linked/Flatten |
| `measure` | units, scales, geometry, `/Measure` persistence, scale registry, report/CSV |
| `nav`, `caps` | links/outlines; capability detection (encryption, signatures, XFA, JS) and edit blockers |
| `protect` | password-protected files: unlock into a plain working copy, rights from the permission flags, `seal` (encrypt again on save), new AES-256 protection (D-035) |
| `redact` | `/Redact` marks and applying them: marked pages replaced by a picture with the marks burned in, invisible text for the rest, clean-up, leak report (D-036) |

Key invariants

* The UI never mutates an object graph directly; it calls an engine function inside
  `session.execute("label", |tx| …)`. The label becomes the undo entry. On `Err` the transaction
  rolls back and nothing is recorded.
* Rendering and text work from an immutable byte **snapshot per revision** (hayro parses these
  bytes itself); results carry the revision and stale ones are discarded.
* Original file bytes are never rewritten: a save appends changed/new objects plus an xref
  section. (`rewrite_bytes` exists for flows that need a full rewrite.)

## editor-core

`DocumentSession` owns the `PdfDocument`, undo/redo stacks (with merge for coalesced edits such
as typing), selection (annotations / text / page content / pages), search state, view state, and
dirty tracking against the saved state. `jobs` is a bounded priority queue with cancellation and
panic containment over worker threads. `command` is the registry of every command with titles,
descriptions, search keywords and per-platform default shortcuts; the palette, ribbon, menus and
shortcut remapping are all driven from it. `prefs` is TOML on disk and the searchable settings
index.

## apps/desktop

`app` (frame loop, shortcut dispatch, autosave, worker results), `state` (all UI state types),
`chrome` (quick access bar, ribbon, tabs, status bar, welcome), `canvas` (view context, tiles,
scrollbars), `interaction` (selection, drawing, text markup, links), `contentedit`, `forms_ui`,
`measure_ui`, `sidebar`, `dialogs`, `exec` (command execution), `theme`, `icons` (original
vector icons), `pagefilter` (dark page filter, render-time only).

UI principles that are implemented, not just intended: progressive disclosure (Essential vs
Professional workspace; Forms tab appears only for documents with forms; Measure tab hidden in
Essential until used); three clearly separated tool families — annotation (Comment tab), page
content (Edit tab, with a standing reminder that it changes real content), form fill and
measurement — and one searchable palette for everything.

## Threading

UI thread: egui frame loop, all document mutation, small synchronous engine calls (including
loading a page’s content objects for editing — a known stall risk on pathological pages).
Workers: tile rendering, text extraction, search, thumbnails; they hold only snapshots.
