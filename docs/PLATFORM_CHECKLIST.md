# Platform verification checklist

**Status of every item: NOT YET RUN.** The authoring environment was Linux. Windows 11 x86_64
and macOS Apple Silicon are first-class targets; until each box below is ticked by a person
on that OS (with notes), nothing in `FEATURE_MATRIX.md` may be marked “Verified”.

## Build (each OS)
- [ ] `rustup show` picks Rust 1.97.0; `cargo build --release -p ferrum-pdf` succeeds
- [ ] `cargo xtask check` passes (Windows: install poppler or accept skipped oracle checks)
- [ ] `cargo xtask dist` produces the unsigned folder (macOS: `Ferrum PDF.app`)
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

## Accessibility
- [ ] Keyboard-only operation of every dialog; focus order
- [ ] Screen reader exposure (Narrator/NVDA, VoiceOver) — expected gaps with egui, record them
- [ ] High-contrast / reduced-motion behaviour

## Packaging (needs the owner’s accounts; not done)
- [ ] Windows: installer (MSI/MSIX), Authenticode signing, SmartScreen reputation
- [ ] macOS: icon (`.icns`), code signing, hardened runtime, notarisation, universal vs arm64-only
- [ ] Uninstall removes nothing the user did not expect (recovery files, preferences)

## Security
- [ ] Windows: render worker in a job object with limits; macOS: sandbox profile (design only
      today — no worker process exists yet)
