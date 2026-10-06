# Editor performance measurements

Run the same Rust workloads on the native host and in Chrome/WASM:

```sh
cargo run -p openwebide-editor-bench --features candidates --release
CHROMEDRIVER=<matching-driver> cargo test -p openwebide-editor-bench --features candidates --target wasm32-unknown-unknown --release -- --nocapture
```

The browser command needs the matching `wasm-bindgen-test-runner` on PATH and Chrome configuration described in [editor controls](editor.md). Copy the runner configuration to `tools/editor-bench/webdriver.json`; each package’s test runner reads its own working directory. CI runs both workloads. Timings are observations, not pass/fail thresholds; correctness checks and native/WASM builds are required. Setup and final text verification are outside timed edits.

Each edit workload performs 50 insert/delete or document edit/Undo pairs (100 edits) at the middle of a Unicode/CRLF buffer. Document history stores replacement deltas. Candidate rope runs compare isolated edits and edits that also materialize the complete display string after every operation. Queries repeat 100 times. `document_100_textarea_queries` exercises actual CRLF-aware browser offsets; raw UTF-16 candidate queries measure their indexing primitives and are not interchangeable with textarea offsets.

Rope candidates are optional dependencies of this measurement tool and are absent from application runtime dependencies. [Crop](https://docs.rs/crop/0.4.3/crop/struct.Rope.html) exposes byte edits and an optional UTF-16 metric. [Ropey](https://docs.rs/ropey/1.6.1/ropey/struct.Rope.html) uses character edits with byte/character conversion. Ropey enables SIMD while disabling extra Unicode/bare-CR line separators to match the editor’s LF/CRLF line semantics.

## Recorded comparison

One local observation on 2026-10-06, Rust 1.98.1, macOS arm64, Chrome 154.0.8037.98. All numbers are total milliseconds for 100 operations. Repeat on deployment hardware before choosing thresholds.

| Workload | Bytes | Native ms | Browser WASM ms |
| --- | ---: | ---: | ---: |
| `document_100_edits` | 2,097,144 | 4.888 | 3.145 |
| `crop_100_edits` | 2,097,144 | 0.004 | 0.015 |
| `ropey_100_edits` | 2,097,144 | 0.009 | 0.060 |
| `crop_100_edits_with_view` | 2,097,144 | 6.833 | 5.565 |
| `ropey_100_edits_with_view` | 2,097,144 | 8.512 | 6.860 |
| `document_100_textarea_queries` | 2,097,144 | 77.741 | 120.825 |
| `document_100_edits` | 16,777,194 | 51.277 | 24.380 |
| `crop_100_edits` | 16,777,194 | 0.005 | 0.055 |
| `ropey_100_edits` | 16,777,194 | 0.013 | 0.060 |
| `crop_100_edits_with_view` | 16,777,194 | 110.173 | 45.565 |
| `ropey_100_edits_with_view` | 16,777,194 | 131.001 | 55.185 |
| `document_100_textarea_queries` | 16,777,194 | 579.388 | 964.545 |

Full records: [native CSV](editor-performance/native-storage.csv), [browser CSV](editor-performance/browser-storage.csv).

Both ropes make isolated edits and indexed queries much cheaper. In this workload, recreating the full display string makes candidate edits slower than the current document. Retain current production storage while implementing worker/viewport rendering; choose storage against the resulting access pattern rather than adding a mirrored rope alongside a full string. Whole-source reference conversions remain in the comparison; production document queries use the incremental index described below.

## Incremental coordinates and shared projections

The document now maintains logical-line and raw/native UTF-16 prefixes across edit
transactions, grouped Undo/Redo and IME previews. Updates re-scan the changed rows
and shift suffix coordinates; queries binary-search a row and scan only within it.
Line/comment/reindent/selection commands reuse those rows. Folded projections share
immutable text, normalized textarea text and visible-row coordinates until source
or folds change. Read-only view preparation does not change document identity.

One observation on the same native host and Chrome version on 2026-10-06:

| Workload | Bytes | Native ms | Browser WASM ms |
| --- | ---: | ---: | ---: |
| `document_100_edits` | 2,097,144 | 5.850 | 5.270 |
| `document_100_textarea_queries` | 2,097,144 | 98.997 | 124.210 |
| `document_indexed_100_textarea_queries` | 2,097,144 | 0.004 | 0.010 |
| `document_cold_projection` | 2,097,144 | 3.209 | 3.035 |
| `document_1000_warm_projections` | 2,097,144 | 0.007 | 0.015 |
| `document_100_edits` | 16,777,194 | 69.710 | 42.660 |
| `document_100_textarea_queries` | 16,777,194 | 767.601 | 1025.380 |
| `document_indexed_100_textarea_queries` | 16,777,194 | 0.013 | 0.025 |
| `document_cold_projection` | 16,777,194 | 29.591 | 24.435 |
| `document_1000_warm_projections` | 16,777,194 | 0.008 | 0.015 |

Full records: [native CSV](editor-performance/native-index.csv),
[browser CSV](editor-performance/browser-index.csv). The indexed query workload
performs 100 paired byte-to-native/native-to-byte queries; the legacy reference
performs 100 byte-to-native queries. Cold projection includes its first allocation;
warm access clones shared handles 1,000 times. Setup is outside the timed section,
and browser clock resolution limits precision for the shortest workloads.

This removes whole-prefix coordinate scans on ordinary short-line files and repeated
projection allocations. It does not bound long-line scans, avoid full String copies
on edits, or eliminate suffix-coordinate shifts. Native textarea input still owns
full projected text. These microbenchmarks do not establish input/scroll latency,
total memory use or full-editor large-file limits.

## Long wrapped lines

The existing both-mode Unicode/CRLF regression keeps two cursors deep inside the same large wrapped line. It checks exact Down/Up restoration, unchanged source/history and fewer than 2,000 DOM range measurements. Paint now splits oversized tokens into Unicode-safe text runs, retaining complete grapheme clusters and escaped source. In the local debug browser fixture, splitting runs reduced elapsed fixture time from 9.05 seconds to 1.27 seconds; a Down press used 225 ranges in approximately 130 ms. This is a regression observation, not a claim of final large-file responsiveness.

Syntax preparation now runs in a Rust/WASM worker, with bounded shared cache retention, validated coordinates and coalesced requests. The built-worker Chrome check exercises all providers, incremental Unicode/CRLF, a UI event during preparation and oversized-source fallback. These correctness checks do not measure total editor memory or latency.

Remaining work: bounded cold and fine wrapped viewport paint, incremental input/source access, responsiveness and memory validation at the admission boundaries, and real-device verification. The production baselines below establish observations rather than completing that validation. Storage microbenchmarks do not prove those items complete.

## Production view and process memory

`python3 tools/measure-editor-view.py` runs the built app against disposable
Spin/SQLite accounts and fresh Chrome process trees. It hydrates the same saved
editor source in local and remote modes, inserts text through Chrome's native input
path, scrolls to 70% of the document, and waits for current source/layout paint.
Wrapped readiness additionally requires an exact height table. Local cases do not
grant an OS directory handle; these measure editor behavior rather than filesystem
permissions. Each source contains Unicode, and multi-line sources retain CRLF.

The harness records load-to-paint, input-to-paint, scroll-to-paint, frame intervals,
main-thread long tasks, main WASM committed memory before/after input, DOM size, and
Chrome process-tree RSS at 200 ms intervals. Linux additionally reports apportioned
PSS when every owned process's `smaps_rollup` is readable. Summed RSS counts shared
pages repeatedly; it is not unique resident memory. Main WASM allocation excludes
worker instances; process-tree measurements include their hosting renderer. Samples
can miss brief peaks. These are complete application workloads, including recovery,
background preparation and deferred saves, rather than isolated core operations.
Input timing includes the native event through paint observation; cold timing also
includes navigation and recovery transport. Results are single observations, not
percentiles or pass/fail latency budgets.

On 2026-10-06, macOS arm64 and Chrome 148, the built geometry checkpoint `645c91f`
(with concurrent branding/welcome working changes) produced these observations.
The raw records identify the compiled JS module hash, browser version and checkout.

| Workload | Mode | Wrap | Cold ms | Input ms | Scroll ms | Main WASM after input MiB | Sampled peak summed RSS GiB |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 58 KB / 1,001 rows | Local | Off | 550 | 37 | 31 | 22.8 | 1.02 |
| 58 KB / 1,001 rows | Remote | Off | 525 | 36 | 6 | 22.9 | 1.02 |
| 2 MiB / 36,158 rows | Local | Off | 1,035 | 254 | 124 | 89.0 | 1.34 |
| 2 MiB / 36,158 rows | Remote | Off | 1,027 | 265 | 124 | 91.0 | 1.38 |
| 100,000 rows | Local | Off | 896 | 202 | 134 | 46.1 | 1.38 |
| 100,000 rows | Remote | Off | 1,177 | 200 | 131 | 46.1 | 1.40 |
| Near 8 MiB byte limit | Local | Off | 1,541 | 358 | 148 | 239.9 | 1.66 |
| Near 8 MiB byte limit | Remote | Off | 1,631 | 331 | 145 | 234.2 | 1.68 |
| 58 KB / 1,001 rows | Local | On | 694 | 167 | 26 | 25.9 | 1.10 |
| 58 KB / 1,001 rows | Remote | On | 721 | 174 | 24 | 26.1 | 1.10 |
| Near 1 MiB single-line limit | Local | On | 2,122 | 447 | 17 | 48.4 | 3.76 |
| Near 1 MiB single-line limit | Remote | On | 2,159 | 402 | 11 | 49.1 | 4.39 |

Full records: [unwrapped JSONL](editor-performance/production-view-unwrapped.jsonl),
[wrapped JSONL](editor-performance/production-view-wrapped.jsonl). To repeat:

```bash
NO_COLOR=true spin build
CHROMEDRIVER=<driver> CHROME=<chrome> python3 tools/measure-editor-view.py \
  --cases small medium line-limit byte-limit
CHROMEDRIVER=<driver> CHROME=<chrome> python3 tools/measure-editor-view.py \
  --cases small long-line --wrap
```

An earlier separate 2 MiB wrapped local attempt timed out waiting for WebDriver
to observe cold paint. It did not establish a completed latency or memory result,
and remote completion at that size was not checked. The near-1 MiB wrapped line
also produced frame stalls above 1.5 seconds and several GiB of sampled summed
RSS despite a bounded final DOM. A bounded warm row count alone therefore does
not prove bounded shaping, cold layout, native input or overall memory.

These results contradict treating the current admission limits as validated
responsiveness limits. Finish bounded cold measurement/paint, finer rendering
within very long wrapped rows, and incremental input/source access; then repeat
these cases with memory sampling, including Linux PSS, and verify the previously
unresponsive wrapped workloads before closing the roadmap item.


## Cold measurement batches

The batched-layout working tree on 2026-10-06 (checkout `f628df9`, concurrent
branding changes, built JS `openwebide-frontend-1866e76a7cfeee5f.js`) completed
both previously unverified 2 MiB wrapped cases. The same Chrome 148/macOS harness
used bounded temporary row DOM, task yields and a frame turn every eight batches.
These are single observations, including concurrent host workloads; summed RSS
retains the shared-page caveat above.

| Workload | Mode | Cold ms | Input ms | Scroll ms | Main WASM after input MiB | Sampled peak summed RSS GiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 58 KB / 1,001 rows | Local | 711 | 91 | 17 | 24.4 | 1.08 |
| 58 KB / 1,001 rows | Remote | 762 | 107 | 32 | 24.6 | 1.09 |
| 2 MiB / 36,158 rows | Local | 2,366 | 2,579 | 89 | 105.4 | 2.02 |
| 2 MiB / 36,158 rows | Remote | 2,616 | 2,628 | 89 | 104.6 | 2.03 |

Full records: [batched wrapped JSONL](editor-performance/production-view-cold-batches.jsonl).
The larger cases still produce frame stalls around 533 ms and rebuild the complete
height table after input. Progressing preparation accepts ordinary native typing
and retains queued arrows, but ordered edits/IME/clipboard after a queued arrow
still need work. Incremental height reuse, finer shaping within long logical rows,
native input/source access and Linux PSS/device validation remain open. Completing
these cases does not validate the admission boundaries as responsiveness limits.


## Incremental styled-row reuse

The follow-on working tree at checkout `166ba6b`, built JS
`openwebide-frontend-9099131cd6992d5e.js`, reuses exact unchanged styled rows after
edits. The same Chrome 148/macOS workload on 2026-10-06 produced:

| Workload | Mode | Cold ms | Input ms | Scroll ms | Main WASM after input MiB | Sampled peak summed RSS GiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 2 MiB / 36,158 rows | Local | 2,375 | 315 | 89 | 102.6 | 1.81 |
| 2 MiB / 36,158 rows | Remote | 2,673 | 303 | 87 | 103.4 | 1.82 |

Full records: [incremental wrapped JSONL](editor-performance/production-view-row-reuse.jsonl).
Localized and disjoint edits reuse unchanged paint only when text, token styles,
line endings, font/width and ownership match. Identical rows with conflicting
previous heights are remeasured. Browser contracts count actual measurement DOM
for single-row changes, insertion/deletion, undo and two distant edits, and verify
full remeasurement after font invalidation in both modes.

Input-to-paint improves from roughly 2.6 seconds to 303–315 ms. Cold preparation
still takes 2.4–2.7 seconds, with frame stalls around 550 ms; native input retains
full projected text. The long-line/byte/row admission boundaries, Linux PSS and
real-device workflows still need the remaining validation described above. These
single observations establish improvement, not final responsiveness limits.
