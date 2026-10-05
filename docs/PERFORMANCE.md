# Performance

All numbers below were measured by `cargo run --release -p pdf-engine --example bench`
(also `cargo xtask bench`) on one Linux x86_64 sandbox VM with 4 cores, **software-only**
environment, Rust 1.97 release profile (thin LTO). They are engine-level wall-clock timings
(single thread unless stated), *not* GUI frame times, and **not** Windows/macOS numbers. Treat
them as a baseline to compare against, not as a promise.

Raw output of the run recorded in `EVIDENCE.md`.

| Scenario | Operation | Time |
|---|---|---|
| Real 3-page Chromium PDF (41 KB, embedded fonts) | open + parse | 0.8 ms |
| | hayro load + render page 1 at 1.0× | 10.2 ms |
| | **total to first rendered page** | **11.3 ms** |
| Synthetic 1000-page text PDF (2.9 MB) | open + parse | 18.1 ms |
| | page list | 3.1 ms |
| | **total to first rendered page** | **49.5 ms** |
| | page-tree walk (1000 ids) | 1.2 ms |
| | one thumbnail (0.2×) | 6.6 ms |
| | 50 thumbnails in one session | 137 ms |
| | extract text of all 1000 pages (basis of search) | 820 ms |
| | add one annotation (transaction) | 0.2 ms |
| | incremental snapshot after the edit | 1.1 ms (800 bytes appended to a 3.0 MB original) |
| | delete 500 pages in one transaction | 2.3 ms |
| Synthetic A0 page (2384×3370 pt), 60,000 vector shapes (6.5 MB) | open + parse | 11.1 ms |
| | render the whole page at 1.0× (8 M pixels) | 740 ms |
| | peak RSS after the run | 152 MB |

Reading these numbers honestly:

* The A0 figure is a *whole page in one go*. The viewer renders 512-px tiles on demand, shows
  placeholders, and cancels stale tiles, so the user waits for visible tiles, not 8 M pixels — but
  the per-tile cost of a dense page has not been measured separately, and GPU upload/compose time
  is not included.
* The 1000-page text search timing is full extraction on one thread. The app runs it on the
  worker pool and streams results; it has not been timed end to end in the GUI.
* The synthetic text pages are simple. Real scanned or image-heavy documents will be slower and
  larger in memory; no such corpus was measured.
* **Not measured:** GUI frame time/CPU while scrolling, memory over a long session, first paint
  time on a cold start, behaviour with 100+ MB scans, any Windows/macOS number.

## Design measures that matter for performance

* Tiled rendering with a byte-bounded LRU texture cache and per-revision invalidation.
* Priority queue: tiles in view (100) > text extraction for the cursor (90) > off-screen tiles
  (40) > search (20) > thumbnails (10); jobs for old revisions are dropped; a thumbnail request can no longer hang in “in flight” (a bug found and fixed
  during GUI testing).
* Editing never re-serialises the file: saves append only changed objects.
* A repaint loop caused by sending a window title every frame (250 % CPU) was found by watching
  `top` during GUI testing and fixed; titles are now sent only on change.
