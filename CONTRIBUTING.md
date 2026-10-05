# Contributing

Thanks for helping. BergPDF is free software under the [GPL-3.0-or-later](LICENSE); by contributing you agree
that your contribution is licensed the same way.

* **Bugs and ideas:** open an issue. For a bug, include your system, the BergPDF version (*File ▸ About*) and, if
  you can share it, a small PDF that shows the problem. Never attach documents that are private.
* **Security problems:** please do not open a public issue; see [SECURITY.md](SECURITY.md).
* **Code:** build and run it with [DEVELOPING.md](DEVELOPING.md). Before opening a pull request run
  `cargo xtask check` (formatting, clippy with warnings denied, all tests) and add a test for what you change.
  The tests write real PDFs and read them back with an independent program where possible; please keep that habit.
* **Keep it honest:** do not describe something as working unless it was run. `docs/FEATURE_MATRIX.md` says what
  has been verified and how; update it with your change.
* **Dependencies:** a new dependency needs a reason, a licence compatible with the GPL (`cargo deny check`) and a
  line in `docs/DEPENDENCIES.md`.
