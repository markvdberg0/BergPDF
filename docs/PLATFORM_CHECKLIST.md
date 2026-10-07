# Platform verification checklist

**Status of every item: NOT YET RUN.** The authoring environment was Linux. Windows 11 x86_64
and macOS Apple Silicon are first-class targets; until each box below is ticked by a person
on that OS (with notes), nothing in `FEATURE_MATRIX.md` may be marked “Verified”.

## Build (each OS)
- [ ] `rustup show` picks Rust 1.97.0; `cargo build --release -p bergpdf` succeeds
- [ ] `cargo xtask check` passes (Windows: install poppler or accept skipped oracle checks)
- [ ] `cargo xtask dist` produces the unsigned folder (macOS: `BergPDF.app`)
- [ ] CI workflow `ci.yml` runs green on `windows-latest`, `macos-14`, `ubuntu-latest`

## Launch and rendering
- [ ] Window opens on a machine with a real GPU (Windows: DX12 and Vulkan; macOS: Metal)
- [ ] Software fallback behaviour acceptable (Windows without GPU / VM)
- [ ] HiDPI / mixed-DPI monitors: page sharpness, UI scale, tile sizes
- [ ] Open the fixtures in `tests/fixtures` (+ `cargo xtask fixtures` output); compare with
      another viewer visually

## Native integration
- [ ] File open/save dialogs (Windows common dialog; macOS NSOpenPanel/NSSavePanel), UNC paths,
      network drives, paths with non-ASCII characters, read-only files, files open elsewhere
      (Windows sharing violations!) — **atomic rename over an open file behaves differently on
      Windows; verify the save path and the error message**
- [ ] Command-line open and drag-and-drop; macOS Finder “Open With” (not implemented yet),
      Windows “Open with” / default-app registration (not implemented yet)
- [ ] External links open the default handler (`rundll32`/`open`) after confirmation
- [ ] Recovery files: written under `%LOCALAPPDATA%`/`~/Library/Application Support`; restore flow
- [ ] Clipboard copy of selected text

## Input
- [ ] Shortcut defaults per platform (Ctrl vs Cmd), conflicts with OS shortcuts, remapping
- [ ] Trackpad pinch/scroll on macOS; precision touchpads on Windows
- [ ] IME composition in text fields (egui limitation — record findings)
- [ ] Pen/touch (density “Touch/Pen” setting)

## Signing, OCR, pictures
- [ ] Sign with a real-world .p12 (company certificate): check in Adobe Reader / Foxit / Preview what
      they say about the signature (expect "not trusted" unless the certificate is installed there)
- [ ] Sign on Windows with a certificate that has a long chain (signature size limit 16 KB)
- [ ] OCR on Windows and Apple Silicon: speed, memory (`rten` CPU kernels), models folder location
- [ ] OCR models: the dialog's download button works through the OS certificate store (Windows/macOS); a bundled `ocr-models/` next to the exe (Windows) / inside the .app (macOS) is found
- [ ] Windows: release exe opens **no console window**; the exe shows the BergPDF icon in Explorer, taskbar and shortcuts; `rc.exe` was found at build time (no `embedding the icon failed` warning)
- [ ] Windows: Preferences ▸ Graphics shows a real GPU (not "Cpu"/WARP/Basic Render); try DirectX 12 vs Vulkan and "Uncapped" and note which resizes smoothly
- [ ] Installed fonts appear in the font pickers on Windows and macOS (names, bold/italic), text in one embeds and extracts; Calibri/Arial/Segoe UI style fonts work, CFF-only fonts are simply absent
- [ ] Translate inline against a real provider: layout of a real multi-page document (tables, columns), cost for a long document
- [ ] Windows: resizing the window in Fit page / Fit width stays responsive (zoom changes during a resize are treated as settling, tiles re-render after ~150 ms)
- [ ] Windows: `cargo xtask dist` (static CRT) builds and starts on a machine without Visual C++ redistributable; SmartScreen behaviour noted
- [ ] Open JPEG photos straight from a phone (EXIF rotation) and PNG screenshots

## Accessibility
- [ ] Keyboard-only operation of every dialog; focus order
- [ ] Screen reader exposure (Narrator/NVDA, VoiceOver) — expected gaps with egui, record them
- [ ] High-contrast / reduced-motion behaviour

## Packaging (needs the owner’s accounts; not done)
- [ ] Windows: Authenticode signing of the NSIS installer, SmartScreen reputation (the Microsoft Store package is covered below)
- [ ] macOS: icon (`.icns`), code signing, hardened runtime, notarisation, universal vs arm64-only
- [ ] Uninstall removes nothing the user did not expect (recovery files, preferences)

## Security
- [ ] Windows: render worker in a job object with limits; macOS: sandbox profile (design only
      today — no worker process exists yet)

## Added 2026-10-05 (nothing below has been run on Windows or macOS)

* [ ] **TLS / certificates**: Copilot "Test connection" with a real key reaches api.openai.com and
      api.anthropic.com from a normal network and from behind a corporate proxy (OS verifier).
* [ ] **Key file**: Windows – the file inherits the user-profile ACL (check no other account can read
      `%APPDATA%\BergPDF\ai-key`); macOS – mode 0600 verified.
* [ ] **Right-click menu** on Windows (Shift+F10 / menu key) and macOS (two-finger click, Ctrl-click).
* [ ] **Panels**: collapse/expand rails and padding at 100 %, 150 % and 200 % display scaling; fit-page
      centring at open on a HiDPI display (the offset reported on the Intel NUC could not be reproduced
      here).
* [ ] **Snap** markers visible and aligned on a HiDPI display.
* [ ] **Save As Optimized / Convert to PDF/A**: native save picker, a 100 MB+ file, cancel behaviour.
* [ ] **Translate → Save as PDF/text** pickers; non-Latin target languages (the PDF replaces characters
      outside Latin/Greek/Cyrillic with "?" and says so).
- [ ] Windows installer: `packaging/windows/bergpdf.iss` builds with Inno Setup, installs per user, Start-menu entry and uninstall work, "Open with" lists BergPDF, the icon shows, SmartScreen behaviour noted (unsigned)
- [ ] cargo-packager: `cargo packager --release -f nsis` on Windows produces an installer that installs per user, starts BergPDF with the icon, lists it under "Open with" for PDF, and uninstalls cleanly; `-f dmg` on macOS (needs signing/notarisation before others can open it without warnings)
- [ ] Windows installer shows the licence (GPLv3) page, asks about the OCR models at the end, downloads them when answered "Yes" (files appear in the per-user data folder `…\BergPDF\ocr-models`), still finishes when offline or when answered "No"; silent install (`/S`) asks nothing
- [ ] Windows installer offers "just me" / "all users"; an all-users install puts OCR models in `Program Files\BergPDF\ocr-models`, finds them in BergPDF, and the uninstaller removes them
- [ ] With BergPDF open, double-clicking a PDF in Explorer opens it as a tab in the running window (no second window); check that the window comes to the front or at least flashes; `--new-instance` starts a separate one
- [ ] macOS: opening a PDF with BergPDF already running (Finder/Dock "Open With") — not handled yet, expected to need Apple-event support
- [ ] Language: Preferences ▸ Language switches the interface at once; with "System" a Dutch / German Windows or macOS shows Dutch / German; check dialogs and the installer-launched first start for text that is cut off or still English
- [ ] Updates: after the first release is *published*, File ▸ Check for Updates… on the installed older version says a newer one exists (and "latest version" on the newest); the one-time question appears once; behind a company proxy the check fails quietly

## Microsoft Store package (MSIX; see docs/MICROSOFT_STORE.md and D-034)

- [ ] `cargo xtask msix` on Windows 11 builds `dist/packages/*.msix` (the SDK tools are found; no warnings from `makepri`/`makeappx`)
- [ ] Installed (developer-mode registration or a test-signed copy): the Start menu shows BergPDF with the right logo and name; the taskbar and Alt+Tab icons have no coloured plate; the tile is sharp at 100 % and 200 % scaling
- [ ] Settings ▸ Apps ▸ Default apps lists BergPDF for `.pdf`; double-clicking a PDF opens it (the path arrives as the first argument), with BergPDF already running it opens as a tab (D-031 works inside the package: the loopback port and the lock file)
- [ ] The Store copy has **no** update check: no one-time question after the welcome screen, no *Check for Updates…* in the ribbon or the command palette, no *Updates* preference (also not when searching the preferences); a copy installed with the NSIS installer still has all of them
- [ ] Preferences, recovery files and downloaded OCR models survive an update of the package and are removed by uninstalling; nothing is written inside `C:\Program Files\WindowsApps`
- [ ] OCR model download, PDF Copilot "Test connection" and opening a link all work from the packaged app (network, certificates)
- [ ] Installed fonts appear in the font pickers (the DirectWrite/registry font folders are readable from a full-trust package); DirectX 12 starts (Preferences ▸ Graphics)
- [ ] Windows App Certification Kit passes on the package
- [ ] Partner Center accepts the package (identity, version, languages) and the pre-certification report is clean
