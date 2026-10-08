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

## Source-change comparison

The shared minimal-edit comparison now compares 64-byte chunks before adjusting
only differing edges to UTF-8 character boundaries. The same helper serves native
input, transactions, fold updates, parser edits, lexical reuse and worker deltas.
The benchmark repeats 100 middle/late insert comparisons outside allocation and
checks the exact resulting span. Measurements on this machine (2026-10-07):

| Source | Position | Native before / after (ms) | Chrome WASM before / after (ms) |
| --- | --- | --- | --- |
| 2,097,144 bytes | Middle | 163.221 / 7.613 | 189.630 / 57.620 |
| 2,097,144 bytes | End | 156.038 / 5.948 | 186.885 / 57.505 |
| 16,777,194 bytes | Middle | 1184.709 / 53.540 | 1545.795 / 447.260 |
| 16,777,194 bytes | End | 1198.077 / 59.841 | 1497.915 / 429.395 |

These are observations from consecutive runs, not latency thresholds or proof of
viewport memory bounds. Comparison remains linear in unchanged text. Full records:
[native before](editor-performance/native-source-change-before.csv),
[native after](editor-performance/native-source-change-after.csv),
[browser before](editor-performance/browser-source-change-before.csv),
[browser after](editor-performance/browser-source-change-after.csv).

## Complete native-input comparison

Complete textarea replacements and duplicate composition commits compare borrowed
byte chunks between CR/LF normalization boundaries. Forward and reverse chunks
retain complete UTF-8 characters and keep CRLF pairs together. The shared input
facade returns original source byte offsets without constructing normalized copies.
The benchmark checks an exact middle insertion and repeats comparison 100 times;
document construction and candidate allocation stay outside the timed operation.

Consecutive runs on this machine (2026-10-07):

| Source bytes | Ending | Native before / after (ms) | Chrome WASM before / after (ms) |
| --- | --- | --- | --- |
| 2,047,212 | LF | 278.608 / 73.755 | 493.465 / 116.835 |
| 2,097,144 | CRLF | 276.221 / 106.637 | 496.785 / 143.945 |
| 16,377,737 | LF | 2235.934 / 592.272 | 3975.460 / 983.465 |
| 16,777,194 | CRLF | 2299.756 / 925.233 | 4016.480 / 1150.195 |

These observations do not establish event-to-paint latency or memory bounds.
Comparison remains linear; native text generation and source/selection publication
are separate costs. The workload now emits 57 records (75 with storage candidates).
Full records: [native before](editor-performance/native-input-before.csv),
[native after](editor-performance/native-input-after.csv),
[browser before](editor-performance/browser-input-before.csv),
[browser after](editor-performance/browser-input-after.csv).

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

## Shared document coordinate tables

An unfolded projection now shares the document index's coordinate table rather
than assembling another table of row handles. Edits update the unique table in
place; retained views and composition/document snapshots detach on mutation.
Folded and bounded projections keep their own visible coordinates without spare
growth capacity. Core contracts verify allocation sharing, immutable retained
views, native offsets, undo/redo and folded reconstruction for Unicode, LF, CRLF
and lone CR. Browser contracts exercise the same editor facade in both modes.

One observation on 2026-10-07 on the same macOS arm64 host and Chrome version;
other verification processes were running, so these are workload observations,
not an isolated before/after performance comparison:

| Workload | Bytes | Native ms | Browser WASM ms |
| --- | ---: | ---: | ---: |
| `document_100_edits` | 2,097,144 | 4.091 | 4.220 |
| `document_cold_projection` | 2,097,144 | 2.942 | 3.080 |
| `document_1000_warm_projections` | 2,097,144 | 0.009 | 0.015 |
| `document_100_edits` | 16,777,194 | 31.322 | 33.890 |
| `document_cold_projection` | 16,777,194 | 24.568 | 25.865 |
| `document_1000_warm_projections` | 16,777,194 | 0.009 | 0.015 |

Full records: [native CSV](editor-performance/native-shared-coordinates.csv),
[browser CSV](editor-performance/browser-shared-coordinates.csv). These records
precede the subsequent visible-row cache: unfolded row tables now prepare lazily,
share the document index and update affected rows plus shifted suffixes after
edits. Retained tables detach, growth reserves 256 rows of headroom, and major
deletions release excess capacity. Cold/folded row assembly, native CRLF
normalization and retained-table detachment still visit large containers; these
measurements do not establish memory bounds, input latency or completion of the
editor performance roadmap.

## Sparse glyph coordinates and cached eligibility

Long logical rows share immutable grapheme/UTF-16 checkpoints across the document
and folded views, approximately every 512 source bytes. Paint and cursor probes
scan from a known cluster boundary instead of retaining a coordinate per glyph.
Unchanged rows retain the same allocation; edited rows rebuild it. The initial
scan also caches whether horizontal fragments are safe for the source. DOM visual
boundaries and styled row shaping are still measured again; this does not finish
long-line rendering or total-memory work.

The shared native/browser workload compares complete glyph arrays with sparse
coordinates for Unicode, combining marks and tabs. It checks 1,000 distributed
lookups and byte-to-glyph inverses, including EOF, and requires estimated sparse
metadata retention below one eighth of the complete array allocation. One macOS
arm64 / Chrome 148 observation on 2026-10-06:

| Workload | Source bytes | Native ms | Browser WASM ms |
| --- | ---: | ---: | ---: |
| Complete glyph construction | 1,048,572 | 9.701 | 14.315 |
| Sparse glyph construction and eligibility | 1,048,572 | 17.889 | 20.770 |
| Sparse glyph queries ×1,000 | 1,048,572 | 2.277 | 2.380 |
| Source eligibility scans ×100 | 1,048,572 | 981.864 | 1047.545 |
| Cached eligibility queries ×100 | 1,048,572 | <0.001 | 0.005 |

Construction is slower than the complete coordinate array because it also checks
horizontal eligibility. Sparse queries trade direct array access for bounded
Unicode scanning; repeated eligibility queries avoid reclassifying the source.
An indivisible grapheme can exceed checkpoint spacing, so the admitted source-line
limit still bounds that exceptional scan. Timings are observations, not latency
gates. Raw records: [native CSV](editor-performance/native-glyph.csv) and
[browser CSV](editor-performance/browser-glyph.csv).

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
worker instances; process-tree measurements include their hosting renderer.

Samples
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
| Near 1 MiB single-line limit | Local | On | 2,122 | 447 | — | 48.4 | 3.76 |
| Near 1 MiB single-line limit | Remote | On | 2,159 | 402 | — | 49.1 | 4.39 |

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

## Sparse long-line coordinates

The follow-on working tree at checkout `9392227` on 2026-10-06 adds immutable
checkpoints approximately every 512 bytes inside long logical rows. Document
native-UTF-16/byte and character-column queries binary-search a checkpoint, then
scan only its tail. Fold projections share the same row indexes, and transactions
rebuild the edit region while retaining indexes outside it. Short rows allocate no
checkpoint array. The same macOS arm64 host and Chrome 148 ran these release
workloads; setup and correctness assertions are outside timed queries.

| Workload | Bytes | Native ms | Browser WASM ms |
| --- | ---: | ---: | ---: |
| Long-line document construction | 65,522 | 0.124 | 0.120 |
| Full-prefix reference, 100 native queries | 65,522 | 3.699 | 4.910 |
| Indexed document, 100 paired byte/native queries | 65,522 | 0.071 | 0.170 |
| Cold folded projection | 65,522 | 0.054 | 0.055 |
| Indexed projection, 100 paired byte/native queries | 65,522 | 0.072 | 0.160 |
| Indexed character columns, 100 queries | 65,522 | 0.003 | 0.010 |
| Long-line document construction | 1,048,574 | 1.895 | 1.835 |
| Full-prefix reference, 100 native queries | 1,048,574 | 56.506 | 76.030 |
| Indexed document, 100 paired byte/native queries | 1,048,574 | 0.005 | 0.015 |
| Cold folded projection | 1,048,574 | 0.739 | 0.840 |
| Indexed projection, 100 paired byte/native queries | 1,048,574 | 0.004 | 0.010 |
| Indexed character columns, 100 queries | 1,048,574 | 0.001 | 0.010 |

Full records: [native CSV](editor-performance/native-long-coordinates.csv),
[browser CSV](editor-performance/browser-long-coordinates.csv). Queries repeat
at one fixed position three quarters into Unicode/tab/standalone-CR text ending
in CRLF; checkpoint alignment differs between sizes. Tiny timings approach browser
clock resolution and are observations rather than thresholds. Reference queries
are one direction while indexed queries include both directions. These measurements
do not establish native input/shaping, fine wrapped paint, memory or admission-boundary
responsiveness. Grapheme indexing for browser geometry still scans a complete line.


## Destination-verified long-line scrolling

The measurement harness now scrolls horizontally for an unwrapped single logical
row, and vertically for wrapped or multiline source. Single-row readiness requires
fragment offsets covering the destination and current document paint ownership.
Earlier single-row scroll readings lacked that destination check; their wrapped
scroll values above are unverified, and an unwrapped single-row vertical scroll
would be a no-op. Historical raw records are retained.

On 2026-10-06, macOS arm64 / Chrome 148, checkpoint `26bbd18` with the concurrent
branding/welcome edits compiled into module `f5175cf8ffc15d3` produced these single
observations for 1,048,572 source bytes:

| Mode | Wrap | Cold ms | Native input ms | Scroll ms | Main WASM after input MiB | Sampled peak summed RSS GiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Local | Off | 1,602 | 768 | 38 | 46.6 | 1.61 |
| Remote | Off | 1,609 | 434 | 43 | 50.8 | 1.58 |
| Local | On | 3,849 | 1,460 | 359 | 47.7 | 3.57 |
| Remote | On | 4,433 | 1,467 | 367 | 49.6 | 4.15 |

The horizontal runs moved 4,406,358 pixels; wrapped runs moved 340,803 pixels.
Raw records: [horizontal JSONL](editor-performance/production-horizontal-anchors.jsonl)
and [wrapped JSONL](editor-performance/production-wrapped-destination.jsonl).
Native input, cold shaping and wrapped scrolling still have substantial stalls;
these observations do not validate the admission limits or complete the editor
performance goal. Linux PSS and actual local folder permissions remain unmeasured
here. Repeat with `--cases long-line` and additionally `--wrap` for the wrapped case.

### Wrapped source anchors

A subsequent [wrapped-anchor observation](editor-performance/production-wrapped-anchors.jsonl)
uses the same destination-verified 1 MiB source and both modes, with the new
wrapped source-slicing implementation in the worktree. It ran before the explicit
bidi eligibility guard was added; this workload contains source-monotonic text.
The built app includes the concurrent branding/welcome changes described above.

| Mode | Cold ready (ms) | Input to paint (ms) | Destination scroll to paint (ms) | Peak summed Chrome RSS (GiB) |
| --- | ---: | ---: | ---: | ---: |
| Local | 4302.3 | 1651.9 | 51.5 | 4.03 |
| Remote | 4682.7 | 1711.8 | 60.8 | 4.51 |

Scroll readiness improved in these individual observations, but initial/input
work and process memory remain costly. These are single observations, not
percentiles or admission validation; Linux PSS and full boundary/device gates
remain outstanding. The final guard retains full-paragraph probes for bidi text.

### Cold height anchors and syntax reuse

The [cold-anchor observation](editor-performance/production-cold-anchors.jsonl)
uses the same destination-verified wrapped 1 MiB source in both modes. The built
worktree app includes cold height-anchor publication, provenance-checked transfer
across equivalent styled syntax results, stable file tabs and the concurrent
branding/welcome changes. Its module identifier is recorded in the raw results;
`checkoutHead` identifies the parent commit rather than the uncommitted build.
No other owned browser test ran concurrently.

| Mode | Cold ready (ms) | Input to paint (ms) | Destination scroll to paint (ms) | Peak summed Chrome RSS (GiB) |
| --- | ---: | ---: | ---: | ---: |
| Local | 2701.5 | 1075.5 | 54.7 | 4.31 |
| Remote | 2768.4 | 488.7 | 54.9 | 4.54 |

An [earlier observation before syntax transfer](editor-performance/production-cold-anchors-before-transfer.jsonl)
recorded 382 ms destination scroll in local mode and 58.7 ms remotely. Equivalent
syntax results could discard anchors while reusing heights, leaving no cold probe
that would repopulate them. The final policy preserves only identical styled rows
with valid previous height provenance; browser contracts verify transfer and stale
publication rejection, including font loading with unchanged computed metrics.

These individual observations support further investigation, not percentiles or
admission validation. Complete cold row shaping, long-line input and multi-GiB
summed Chrome RSS remain costly. Summed RSS can double-count shared pages; Linux
PSS and full boundary/device gates remain outstanding.

### Source slices before HTML generation

The shared facade now selects bounded source from retained styled anchors before
HTML generation and parsing. Browser contracts in both modes audit the largest
source row passed to HTML escaping on unseen horizontal/wrapped intervals and
verify it stays within 64 KiB, preserving Unicode/native hits and Find. An injected
partial measurement failure restores complete source, drops the rejected anchors
and obtains fresh bounded paint on the next probe. Equivalent styled syntax can
retain unwrapped geometry within the same font/layout epoch without a height
table; font loading still invalidates it.

The final [horizontal observation](editor-performance/production-source-slices-horizontal.jsonl)
and [wrapped observation](editor-performance/production-source-slices-wrapped.jsonl)
use the same destination-verified 1 MiB workload, module `d4e04563118f0285`.
The build contains the current worktree and concurrent branding/welcome changes;
`checkoutHead` records its parent commit. Browser tests were terminal before these
sequential runs started.

| Layout | Mode | Cold ready (ms) | Input to paint (ms) | Destination scroll to paint (ms) | Peak summed Chrome RSS (GiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| Horizontal | Local | 1676.7 | 455.4 | 22.8 | 1.55 |
| Horizontal | Remote | 1282.1 | 429.3 | 367.7 | 1.57 |
| Wrapped | Local | 2631.2 | 850.0 | 455.8 | 4.59 |
| Wrapped | Remote | 3164.6 | 497.5 | 458.4 | 4.63 |

Earlier samples of the same source-slicing build before the unwrapped reuse fix
are retained for comparison: [horizontal](editor-performance/production-source-slices-horizontal-before-reuse.jsonl)
recorded 715.5/371.6 ms destination scroll; [wrapped](editor-performance/production-source-slices-before-unwrapped-reuse.jsonl)
recorded 35.7/39.1 ms. The initial height-provenance requirement excluded unwrapped
rows from equivalent syntax reuse; the shared contract now covers that correction.
These single observations remain inconsistent immediately after readiness and do
not prove stable startup scroll latency. Trace late syntax/font/layout changes and
uncached probes before claiming that gate complete. Full initial shaping, input
costs, process memory, Linux PSS, percentiles and admission/device gates remain
outstanding; summed RSS can double-count shared pages.

### Font provenance across syntax and layout reconciliation

The [before trace](editor-performance/production-layout-provenance-before.jsonl)
records full 748,980-native-unit paint probes after worker replies, alongside
bounded paint. Complete-row bounding-box reads took roughly 330–425 ms. No font
loading events occurred. These traces instrument browser primitives and perturb
timing; they establish probe type and ordering, not uninstrumented latency.

The shared facade regression reproduced a specific ordering bug: equivalent syntax
paint carried anchors into a new scope, then height reconciliation rejected them
against the older height-cache scope. Independent font provenance now retains
exact styled rows through ordinary layout reconciliation. Actual font changes,
source/read/account changes and stale callback publication remain protected.
The same contract failed before this fix and passes in both modes afterward.

The [after trace](editor-performance/production-layout-provenance-after.jsonl)
retains only bounded paint after the initial height probes, including after
worker replies. Wrapped cold shaping still runs twice in these observations;
new input still needs full height measurement. No font events or trace truncation
occurred. `--trace` caps each diagnostic array at 256 records and marks truncation;
normal benchmark runs do not replace browser measurement or worker primitives.
No source text or worker payload is stored in diagnostic records.

Uninstrumented [horizontal](editor-performance/production-font-provenance-horizontal.jsonl)
and [wrapped](editor-performance/production-font-provenance-wrapped.jsonl)
observations use the same destination-verified 1 MiB workload in both modes.
The built worktree includes concurrent branding/welcome changes; the raw module
identifier describes the build, while `checkoutHead` identifies its parent commit.
All owned browser tests were terminal before these sequential runs started.

| Layout | Mode | Cold ready (ms) | Input to paint (ms) | Destination scroll to paint (ms) | Peak summed Chrome RSS (GiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| Horizontal | Local | 1655.1 | 466.8 | 23.5 | 1.55 |
| Horizontal | Remote | 1660.4 | 452.2 | 23.3 | 1.56 |
| Wrapped | Local | 2745.0 | 515.8 | 36.5 | 4.63 |
| Wrapped | Remote | 2714.0 | 442.4 | 36.3 | 4.66 |

This removes the reproduced cache-ordering failure and extra full paint probes
in these runs. It does not prove percentiles or complete admission gates. Duplicate
initial height shaping, cold native input layout, full String/input costs,
bidirectional windows, Linux PSS, boundary and device verification remain open.
Summed Chrome RSS can double-count shared pages.

### Settled font readiness and duplicate cold measurement

The [original-scope before trace](editor-performance/production-font-ready-before.jsonl)
records two startup height probes in each mode with identical styles and source,
read and account revisions. The second probe advances font provenance from zero
to one despite no font loading events. The viewport observer unconditionally
invalidated font geometry when the already-resolved `document.fonts.ready`
promise completed. The both-mode browser regression reproduces this only after
waiting for animation frames, and fails before the fix.

The observer now invalidates on loading completion or failure, including loads
in progress when it is installed, rather than settled readiness. The
[after trace](editor-performance/production-font-ready-after.jsonl) records one
startup height probe per mode, retains font generation zero and has no font
events or trace truncation. Measurement probes record their original immutable
scope so diagnostic timing cannot mislabel jobs with newer state.

`--trace` also records `preparationBatches`: Rust HTML rendering (source bytes),
DOM installation (HTML bytes), row layout/width reads, and source-geometry
sampling/publication (row counts). Each phase includes elapsed milliseconds;
the layout and geometry totals sum per-row work. The optional adapter callback
is installed only by the measurement harness; ordinary preparation does not
read a timing clock or invoke diagnostics. Records retain the probe's immutable
account/read/source/view/font/layout scope and stop at 256 batches, marking
truncation. Querying nodes, plan bookkeeping and yielding are outside these
phase totals, so they are not an end-to-end preparation duration.

Traces record
style and revision attributes, never source text. These instrumented observations
perturb timings and establish probe counts rather than latency thresholds.
Both builds include concurrent branding/welcome changes; module identifiers
identify the builds and `checkoutHead` their parent commit. Initial full-row
shaping, native textarea/input and full String costs, bidirectional windows,
Linux PSS, percentiles and boundary/device gates remain outstanding.

### Repeated Linux boundary baseline

`tools/measure-editor-view-linux.sh` runs the same built Rust/WASM editor harness
in a disposable Linux container, with matching distribution Chromium/ChromeDriver,
DejaVu, Noto CJK and color emoji fonts. It reuses the existing built WASM files;
there is no second Cargo target directory. Every repetition creates fresh browser,
account and runtime state. The launcher requires numeric peak and final Chrome
PSS, runs by immutable image ID, and records that ID, the browser version, CPU
count and cgroup memory/CPU limits alongside the parent checkout and app module.

```bash
NO_COLOR=true spin build
tools/measure-editor-view-linux.sh \
  --cases medium line-limit byte-limit long-line --wrap --repeat 3
tools/measure-editor-view-linux.sh \
  --cases medium line-limit byte-limit long-line --repeat 3
```

The container has a four-CPU quota, a 10 GiB memory limit and 1 GiB shared memory.
These are observations on a Linux arm64 Docker Desktop VM, not deployment-host
percentiles or phone/device checks. The same source/destination readiness checks
cover local and remote editor behavior; local folder permissions are not exercised.
PSS apportions shared Chrome pages; it excludes the Spin runtime and ChromeDriver.
The 200 ms sampler can miss brief peaks. Three repetitions establish observed
variation, not reliable tail percentiles or pass/fail responsiveness budgets.

On 2026-10-06, Chromium 154.0.8037.92 on Linux arm64 completed all 48
Unicode-capable boundary runs. Each table entry summarizes three fresh runs.
Cold and scroll values are medians; input shows median and observed range; PSS
is the largest sampled peak among those runs. Both image exports share the same
runtime configuration and package layers; build attestation IDs differ.
The built worktree includes concurrent branding/welcome changes; module
identifiers identify the actual build and the checkout its parent commit.

| Layout | Workload | Mode | Cold median ms | Input median (min–max) ms | Scroll median ms | Largest peak Chrome PSS GiB |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Wrapped | 2 MiB | Local | 3943 | 500 (484–529) | 178 | 1.34 |
| Wrapped | 2 MiB | Remote | 4188 | 461 (457–498) | 177 | 1.34 |
| Wrapped | 100,000 rows | Local | 3702 | 397 (349–412) | 270 | 1.43 |
| Wrapped | 100,000 rows | Remote | 4045 | 335 (332–346) | 253 | 1.37 |
| Wrapped | Near 8 MiB | Local | 4217 | 644 (610–661) | 320 | 2.06 |
| Wrapped | Near 8 MiB | Remote | 5575 | 579 (577–622) | 322 | 2.25 |
| Wrapped | Near 1 MiB line | Local | 1763 | 556 (528–556) | 48 | 1.00 |
| Wrapped | Near 1 MiB line | Remote | 1766 | 496 (485–568) | 42 | 1.02 |
| Unwrapped | 2 MiB | Local | 1736 | 440 (346–451) | 193 | 0.95 |
| Unwrapped | 2 MiB | Remote | 1880 | 397 (381–456) | 354 | 0.97 |
| Unwrapped | 100,000 rows | Local | 1518 | 397 (388–399) | 275 | 1.07 |
| Unwrapped | 100,000 rows | Remote | 1539 | 282 (266–287) | 270 | 1.09 |
| Unwrapped | Near 8 MiB | Local | 2170 | 473 (468–664) | 413 | 1.28 |
| Unwrapped | Near 8 MiB | Remote | 2348 | 472 (454–474) | 439 | 1.29 |
| Unwrapped | Near 1 MiB line | Local | 1623 | 530 (518–545) | 26 | 1.03 |
| Unwrapped | Near 1 MiB line | Remote | 1654 | 538 (509–553) | 34 | 1.04 |

These baseline input timings start at `input`. The current harness starts at
`beforeinput` to include edit preparation; records label this as
`inputTimingStart`. Timings from the two clocks are not directly comparable.

Raw records: [wrapped](editor-performance/production-linux-wrapped-boundaries.jsonl),
[unwrapped](editor-performance/production-linux-unwrapped-boundaries.jsonl).
Earlier exploratory runs without a CJK font are excluded from this baseline;
font availability materially changed long-line shaping and memory. These font
fixture changes are not production editor performance improvements.

The completed runs provide Linux PSS evidence and repeated byte/row/long-line
boundary observations in both modes. They still show costly input and cold paint;
current admission limits remain unvalidated as responsiveness limits. Initial
shaping, bounded native input/source access, bidirectional visual-run windows,
more repetitions for tail latency and real-device/input-method/permission checks
remain required before closing the full editor roadmap item.

On 2026-10-06, the browser-owned insertion commit path passed twelve production
Linux runs (2 MiB, near 8 MiB and near 1 MiB line; both modes and layouts). These
are one-run correctness and PSS observations, not tail estimates. Input timing
starts at `beforeinput`, including preparation, so it cannot be compared directly
with the earlier `input` baseline. Large-file latency remains substantial.

| Layout | Case | Mode | Beforeinput to paint ms | Peak Chrome PSS GiB |
| --- | --- | --- | ---: | ---: |
| wrapped | medium | local | 522 | 1.32 |
| wrapped | medium | remote | 454 | 1.32 |
| wrapped | byte-limit | local | 644 | 2.06 |
| wrapped | byte-limit | remote | 620 | 2.17 |
| wrapped | long-line | local | 1024 | 1.00 |
| wrapped | long-line | remote | 973 | 1.03 |
| unwrapped | medium | local | 437 | 0.92 |
| unwrapped | medium | remote | 467 | 0.99 |
| unwrapped | byte-limit | local | 480 | 1.27 |
| unwrapped | byte-limit | remote | 496 | 1.29 |
| unwrapped | long-line | local | 823 | 0.98 |
| unwrapped | long-line | remote | 823 | 1.03 |

Raw records: [wrapped native commit](editor-performance/production-linux-native-commit-wrapped.jsonl),
[unwrapped native commit](editor-performance/production-linux-native-commit-unwrapped.jsonl).

The following trusted single-cursor value-retention checkpoint also passed all
twelve workloads in both modes/layouts on 2026-10-06. Browser contracts verify zero
full DOM-value reads in the successful trusted input handler, while synthetic
inputs and multiple cursors still reconcile. These one-run `beforeinput` timings
range from 388 to 1034 ms; they do not establish a latency improvement or
validate responsiveness limits. Preparation, source publication, initial shaping
and full native textarea layout remain expensive.

Raw records: [wrapped native retention](editor-performance/production-linux-native-retain-wrapped.jsonl),
[unwrapped native retention](editor-performance/production-linux-native-retain-unwrapped.jsonl).

## Existing-buffer transactions

On 2026-10-06, the shared engine validated borrowed proposed pieces before mutation
and applied edits/grouped history to the existing String. Allocation contracts
verify retained capacity for repeated typing/undo/redo and at most 64 KiB growth
headroom above a transaction's peak size. Merged row contexts preserve unchanged
interior indexes; randomized Unicode/CRLF transactions match complete replacement
and history, and rejected selections leave the document unchanged. Fold APIs share
precise boundary rebasing and retain unrelated collapsed ranges through refresh.

All 63 shared storage observations passed on native macOS arm64 (Rust 1.98.1) and
Chrome 148.0.7778.97/browser WASM. These are isolated one-run observations, not
percentiles or evidence of an overall speedup. The 16 MiB fixture uses the
unrestricted document API, outside the full editor's 8 MiB admission boundary.

| 100 middle edit/Undo operations | Native ms | Browser WASM ms |
| --- | ---: | ---: |
| 65,520 bytes | 0.885 | 0.740 |
| 2,097,144 bytes | 7.803 | 3.900 |
| 16,777,194 bytes | 26.279 | 31.715 |

Raw storage records: [native](editor-performance/native-buffer-transactions.csv),
[browser WASM](editor-performance/browser-buffer-transactions.csv).

The final capped-growth production build also passed twelve Linux Chromium 154
runs across 2 MiB, near 8 MiB and near 1 MiB line workloads, both modes and layouts.
Each has numeric apportioned Chrome PSS and the same compiled application module.
The build includes concurrent branding/welcome work; raw parent-checkout and module
identities describe the actual worktree build. Uncapped-reserve exploratory runs
are excluded.

One-run `beforeinput`-to-paint timings range from 412 to 1018 ms. Removing the
replacement candidate does not establish responsiveness: full proposed-text
admission scans, String suffix shifts, full buffer/projection/IME publication,
initial shaping and native textarea layout remain.

Raw production records: [wrapped](editor-performance/production-linux-buffer-transactions-wrapped.jsonl),
[unwrapped](editor-performance/production-linux-buffer-transactions-unwrapped.jsonl).

## Cold layout candidate check

The first cold logical-row probe still shapes a complete paragraph. Before
replacing it, `tools/measure-editor-layout.py` compares Parley 0.11.1 in native
Rust and browser WASM with DOM and canvas geometry. Parley provides shaping,
line breaking and bidirectional layout; see its
[layout description](https://github.com/linebender/parley/blob/main/doc/concept.md).

The isolated probe uses the same supplied Monaspace Neon v1.400 variable TTF in
each engine, 13 px text, 19.5 px line height, wrapping at 300 px, and texture
healing/ligature settings enabled and disabled. Parley has only that registered
font, with system-font discovery disabled to match the WASM environment. The
browser supplies its normal fallback fonts. Tab-size policy is supplied to the
DOM; Parley and canvas receive the original tab characters without an additional
tab-stop implementation. These are integration gaps to resolve, rather than
evidence that the libraries cannot support those features.

Recorded unwrapped widths with features enabled, in CSS pixels:

| Case | DOM | Rust/WASM Parley | Canvas |
| --- | ---: | ---: | ---: |
| ASCII source | 241.8125 | 241.8000 | 241.7981 |
| `a\tb\tc` | 72.5469 | 40.3000 | 40.2997 |
| Combining mark, CJK and emoji | 93.7500 | 80.6000 | 93.7395 |
| Mixed LTR/RTL | 175.2969 | 177.3200 | 175.2493 |
| 990,000-byte ASCII paragraph | 7,979,337 | 7,933,542 | 7,957,381.5 |

The large paragraph requires further precision investigation: its width differs
substantially even with the same font data. Wrapped heights match in this sample,
which does not establish matching glyph/caret positions. The Parley browser
measurement took 513 ms for the long unwrapped paragraph versus 45.65 ms for the
bare DOM row. These are single observations, not latency percentiles. The Rust
measurement includes constructing font/layout contexts and shaping; the DOM
measurement covers layout after text installation with a loaded font. Neither
measures the application renderer, styled-span parsing, input, or process memory.

Chrome 154 reports no extended canvas index, cluster or selection-rectangle
methods. Registering separate font faces with feature descriptors does change
canvas pixels, but canvas width still differs from DOM width. Future canvas
editing methods are described in the
[Chromium proposal](https://groups.google.com/a/chromium.org/g/blink-dev/c/Wf1iK1bc_00/m/JDtlgk-eAwAJ);
they cannot be assumed available in the browsers tested here.

**Decision:** keep the current DOM geometry adapter while developing bounded cold
preparation. A different shaper needs matching tab stops, explicit fallback-font
ownership, source/glyph precision, and a painter using the same geometry. Passing
this comparison probe only proves the candidate runs and produces measurements;
it does not complete cold rendering, bidi windows, native input, or either-mode
performance gates. Experimental dependencies stay outside the production
workspace, and the main Cargo lockfile is unchanged.

Raw records: [layout comparison](editor-performance/layout-candidate-parley.jsonl).
The tool freezes candidate dependencies, uses the repository's current WASM
bindings and lint policy, builds only in its shared `target/`, and removes its
temporary source directory afterward. With matching wasm-bindgen tools and Chrome
available, reproduce using the
[upstream font](https://github.com/githubnext/monaspace/blob/v1.400/fonts/Variable%20Fonts/Monaspace%20Neon/Monaspace%20Neon%20Var.ttf):

```bash
CHROMEDRIVER=/path/to/chromedriver \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=/path/to/wasm-bindgen-test-runner \
python3 tools/measure-editor-layout.py \
  --font /path/to/MonaspaceNeon.ttf --output /tmp/editor-layout.jsonl --lint
```


### Production cold-preparation phases

[Phase records](editor-performance/production-cold-phases.jsonl) cover the small
and 1 MiB Unicode long-line fixtures, wrapped and unwrapped, in both modes on
macOS Chrome 154. The recorded module identifies the release bundle built from
the working changes on parent `a9c4397`; these are instrumented single runs,
not percentiles or Linux PSS evidence. The checked-in records omit the existing
per-row `layoutProbes` array; batch phases and all other result fields remain.

For the long line, cold row layout/width reads took 2,687–3,668 ms per batch.
HTML generation took roughly 12–13 ms and DOM installation roughly 2 ms in the
unwrapped runs. Source geometry sampling took roughly 27–45 ms. Font/layout
scope changes caused repeated cold full-row measurements; input subsequently
repeated full-row layout despite using a bounded native input window. Load to
paint ranged from 11.6–17.8 seconds, and input to paint from 2.9–3.1 seconds.
These phase timings include the opt-in primitive hooks and do not account for
all startup/native-input work. Small-fixture row diagnostics hit their existing
256-record cap; batch phase records did not hit their separate cap.

**Next action:** target full-paragraph browser shaping and repeated layout scopes,
then repeat the same workloads without tracing. Merely speeding HTML assembly
or making its allocation cooperative cannot address the measured multi-second
layout calls. Exact source/glyph/caret geometry and wrapped extents remain
required; this evidence does not complete the cold-shaping gate.

The measurement harness now recognizes source-scoped bounded native bindings
and scrolls the active source scroll surface, following the production adapter's
selection rule. It records native binding/length separately instead of requiring
the textarea to hold the full file. Full-source editing correctness remains
covered by the shared component contracts, rather than inferred from this
performance probe.


### Pending/plain row reuse

The shared row comparator now represents a single `Plain` token and a borrowed
pending row by their exact rendered body. Both HTML paths call `paint_text` once;
CRLF normalization follows the renderer. Multiple token runs stay distinct,
because changing text/span boundaries can change glyph shaping. Row guides,
endings, whitespace, font/layout metrics, file/read/source/account ownership
still participate in the existing cache and publication checks. No transport or
DOM-measurement implementation was added.

[After records](editor-performance/production-plain-row-reuse.jsonl) repeat the
1 MiB Unicode paragraph in both modes, wrapped and unwrapped, against the
previous [phase baseline](editor-performance/production-cold-phases.jsonl).
All four cases record three full-row batches instead of four across load,
scroll and input: identical plain syntax no longer adds a redundant measurement.
The recorded module identifies the bundle from working changes on parent
`2431484`; per-row `layoutProbes` are omitted as in the baseline. These are
instrumented single observations, not latency percentiles. Each remaining full
layout still takes seconds, and font transitions and actual edits still require
new geometry; this does not complete initial shaping or incremental long-row
measurement.

The both-mode browser contract checks retained glyph anchors and completed
measurement plans after pending/plain resolution, while rejecting different
plain token boundaries. Existing stale source/read/account/font checks remain.
All 366 browser component tests pass with the change. Next, address first cold
paragraph shaping, font-transition duplication and actual long-row edits.


### Font settling before cold row probes

Cold measurement reads the primary computed CSS family and waits for its
registered faces through the browser's
[FontFace loading primitive](https://www.w3.org/TR/css-font-loading-3/#font-face-load).
It checks registered face status directly: family availability checks can succeed
through fallback and do not establish that the primary face has loaded. The
configured Monaspace families each register one variable face. Glyph shaping
still uses the full computed style of the existing DOM probe; font loading
supplies no substitute metrics.
Already available fonts proceed immediately. Pending loads race a 250 ms timer,
with the budget defined in shared Rust core. Failed or stalled requests preserve
browser fallback and the existing native editing surface.

After an actual wait, a frame and task let font notifications invalidate old
measurement tickets. Source/account/font/layout ownership and computed metrics
are checked again before allocating the probe. Font changes after a timeout
still invalidate fallback geometry through the existing observer. No
measurements or glyph positions are guessed while a font is pending.

The both-mode browser regression controls font completion, failure and an
unresolved request, and replaces source while the load is pending. It checks
that no probe is allocated immediately for pending fonts, old font/source jobs
cannot run after completion, and a request that never resolves still permits
fallback measurement. Initial paragraph shaping and native cold/touch input
remain separate gates.


Measurement identity now includes the registered primary faces, their lifetime
IDs, statuses and style/weight/stretch descriptors, alongside the existing CSS
metrics. A load can therefore obsolete a queued probe even when computed CSS is
unchanged. Preference observation compares CSS separately, so availability
changes are not misclassified as preference changes.

Native loading notifications go through the shared editor facade. If current
source/account-owned measurements already use identical actual font and layout
metrics, their geometry is retained. Missing or different metrics invalidate it.
Explicit/synthetic invalidation and preference changes retain forced refresh.
This handles native notifications delivered after a loaded face was already
used by a completed measurement, while preserving fallback invalidation after
a delayed load. Opt-in traces now record up to 16 face names/statuses per batch
and loading event, within the existing 256-record caps; no source text is added.


[Final production records](editor-performance/production-font-notifications.jsonl)
repeat the 1 MiB Unicode paragraph in both modes, wrapped and unwrapped, using
the bundle identified by its module on parent `c549010`. All four cases now
record one cold full-row batch plus one batch after typing, compared with two
cold/scroll batches plus one input batch in the
[pending/plain baseline](editor-performance/production-plain-row-reuse.jsonl).
Registered-face states and loading events retain provenance; per-row diagnostics
are omitted as in prior records. Unwrapped load to paint was about 8.9 seconds;
wrapped was about 10.5–10.6 seconds. These instrumented single observations
establish removal of redundant work, not latency percentiles or Linux PSS gates.
Initial shaping and long-row input remain multi-second work and are unfinished.

The final implementation passes all 367 browser component tests, including
the eight controlled font-load outcomes across both modes, and all-target WASM
Clippy. Production probes use disposable state; native folder permission,
physical PWA input, Linux PSS and uninstrumented latency gates remain open.

## Fresh Linux boundary observations

On 2026-10-07, the release build at `abc6e4a` completed sixteen fresh-process
Linux Chromium 154 cases: 2 MiB, near 8 MiB, 100,000 rows and a near 1 MiB Unicode
line, wrapped/unwrapped in local/remote projects. Each case retained bounded native
input and completed cold paint, destination scrolling and a trusted insertion.
The container retained the 10 GiB memory/four-core configuration and CJK/emoji
fonts. These are single observations per mode, without probe instrumentation;
input timing starts at `beforeinput`. The following ranges cover the two modes,
not repeated-run percentiles. PSS is the largest sampled peak, including startup.

| Layout | Case | Cold range ms | Input range ms | Scroll range ms | Peak Chrome PSS GiB |
| --- | --- | ---: | ---: | ---: | ---: |
| wrapped | medium | 3793–3985 | 726–745 | 26–34 | 1.23 |
| wrapped | byte-limit | 7486–7915 | 434–473 | 34–36 | 2.15 |
| wrapped | line-limit | 4389–4598 | 1658–1658 | 24–34 | 1.27 |
| wrapped | long-line | 16808–16923 | 4356–4634 | 36–41 | 2.79 |
| unwrapped | medium | 1745–2383 | 72–735 | 12–25 | 0.87 |
| unwrapped | byte-limit | 2387–2434 | 430–457 | 32–35 | 1.07 |
| unwrapped | line-limit | 3451–3504 | 1649–1669 | 25–34 | 0.97 |
| unwrapped | long-line | 11672–11822 | 3749–4130 | 20–22 | 0.83 |

Raw records: [wrapped](editor-performance/production-linux-native-chunks-wrapped.jsonl),
[unwrapped](editor-performance/production-linux-native-chunks-unwrapped.jsonl).
Chrome subprocess exits made one initial unwrapped PSS sample unavailable; that
case retained later, peak and final PSS samples. The records preserve the missing
value instead of substituting summed RSS.

The runs establish functional boundary coverage and current memory observations,
but contradict completion of cold/long-line responsiveness. The long paragraph
still causes multi-second tasks, and row-count input needs profiling. Isolated
phase traces and repeated runs are needed to attribute those costs; these results
cannot be compared directly to older Chrome/build/font observations. Geometry
validation's subsequent removal of temporary normalized source copies is a
separate change, not a measured speedup in these records. Keep cold shaping,
incremental measurement, long-row input and responsiveness verification open.

The opt-in `--trace` harness now also records native textarea value-write lengths
and `scrollWidth`/`scrollHeight` read durations, with the installed binding state
and phase. This distinguishes native layout from styled-row probe timings without
reading/copying the textarea value for diagnostics. A disposable small Linux case
verified all three event kinds and the complete-to-window value transition. The
trace check is instrumentation validation, not an isolated performance sample;
use it to profile long-line and row-count cases next.

## Row-count input scheduling and rules detection

Isolated release traces at parent `94ef898` separated the two remaining costs:
[row input](editor-performance/production-linux-row-input-trace.jsonl) and
[layout/native input](editor-performance/production-linux-layout-input-trace.jsonl).
On 100,000-row input, the native `input` handler took about 16 ms and the changed
row probe about 0.3 ms, but lexical fallback waited one animation frame per eight
128-row batches (roughly 98 frames). A near-1-MiB wrapped paragraph instead spent
about 3.5 seconds measuring its changed full row; that separate cost remains.

Lexical fallback now uses a four-millisecond preparation budget per animation
frame with a 64-batch hard cap, retaining task yields and ownership validation
between batches. Missing/invalid clocks retain the conservative eight-batch
fallback. An intermediate build reduced remote row-count input to about 243 ms,
but local input remained about 1.1 seconds even in isolated repeated runs.
Profiling ownership checks found repeated full-source rules detection when a
loaded configuration entry was unavailable. The shared facade now memoizes rules
and invalidates on source, preferences, loaded configuration, override and file/
project changes. Neither adapter implements its own rules or scheduling policy.

Three fresh-process samples per mode/layout on Linux Chromium 154 used the
working release module `3b3fccc397b4a4fd` on parent `94ef898` with both changes.
Input timing starts at trusted `beforeinput`; the same 10-GiB/four-core container
and CJK/emoji fonts were retained. Medians and observed input ranges follow:

| Layout | Mode | Cold median ms | Input median ms | Input range ms | Peak Chrome PSS GiB |
| --- | --- | ---: | ---: | ---: | ---: |
| Wrapped | Local | 3208 | 244 | 241–249 | 1.24 |
| Wrapped | Remote | 3112 | 244 | 241–250 | 1.27 |
| Unwrapped | Local | 1983 | 238 | 230–238 | 0.97 |
| Unwrapped | Remote | 2005 | 237 | 236–238 | 0.97 |

Raw records: [wrapped](editor-performance/production-linux-lexical-rules-wrapped.jsonl),
[unwrapped](editor-performance/production-linux-lexical-rules-unwrapped.jsonl).
These twelve observations establish the row-count improvement; they do not
establish interactive latency percentiles or resolve cold shaping and long-row
measurement. Local fixtures recover source without a browser directory handle,
so physical native folder permissions remain a separate gate. A later pointer
readiness fix flushes a pending validated source frame before a click; its release
module differs and is not included in these timing records.

Validation passed 438 core tests, all 408 WASM browser tests and strict all-targets
frontend Clippy. The updated release app also passed fourteen trusted pointer
cases (Rust/C#/JSON, short/scrolled and wrapped Rust) and four bounded-native
composition cases (local/remote, LF/CRLF). The cold unwrapped contract specifically
clicks a scrolled source row while full measurements are pending, then verifies
insertion at the source offset and retained whole-file extents.

## Retained paragraph mutation check

`tools/measure-editor-row-update.py` isolates paragraph mutation using the built
stylesheet, Monaspace Neon with texture healing/ligatures enabled, and fresh
Chromium 154 processes. The Linux container retains the 10-GiB/four-core limits
and CJK/emoji fallback fonts. It compares minimal `Text.replaceData`, replacing
text data and rebuilding nodes after initial layout. Each completed case checks
complete text and compares row width/height plus one sampled caret across methods.
These checks do not prove every glyph or end-to-end application input behavior.

The first single-text-node surrogate completed 27 cases, then timed out on the
first wrapped Unicode case even with a 180-second command deadline. Its missing
wrapped result is recorded explicitly. This surrogate does not match the editor's
existing 512-byte, grapheme-preserving `editor-text-run` markup, and its glyph
geometry differs; it must not be used as an application performance estimate.
A separate 4,096-unit, space-boundary split completed one wrapped Unicode sample
in about 4.5 seconds, but still retained a multi-second input task.

The matching plain-run fixture then completed all 36 cases: ASCII/Unicode,
wrapped/unwrapped, three update methods and three repetitions. A midpoint `X`
insertion retained row dimensions and sampled caret positions across methods
within 0.25 CSS pixels. Median elapsed times in milliseconds:

| Source/layout | Minimal edit total | Minimal edit mutation | Minimal edit layout | Set data total | Rebuild total |
| --- | ---: | ---: | ---: | ---: | ---: |
| ASCII, unwrapped | 112 | 110 | 3 | 124 | 947 |
| ASCII, wrapped | 120 | 88 | 32 | 163 | 937 |
| Unicode, unwrapped | 3483 | 3474 | 10 | 3272 | 3762 |
| Unicode, wrapped | 3508 | 3428 | 80 | 3432 | 3986 |

Rebuild totals include JavaScript `Intl.Segmenter` and DOM creation in this
isolated fixture; the application builds markup in Rust. Those totals cannot be
substituted for measured application rebuild costs. More importantly, minimal
mutation still performs multi-second work on Unicode even when the subsequent
rectangle read is short. A retained DOM cache alone therefore does not establish
responsive admitted paragraphs. Keep bounded cold/changed paragraph preparation,
font fallback, bidi windows and application latency/PSS verification open.
Chromium's [inline layout description](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/core/layout/inline/README.md)
and [LayoutNG overview](https://developer.chrome.com/docs/chromium/layoutng)
explain the paragraph-level shaping/cache boundary; DOM node identity alone is
not evidence that an update is local.

Raw records: [single-node surrogate, including timeout](editor-performance/layout-row-mutation-linux.jsonl),
[space-boundary split](editor-performance/layout-row-mutation-split-wrapped.jsonl),
[matching plain-run fixture](editor-performance/layout-row-mutation-production-runs.jsonl).
The timeout footer on the first record was added after the old harness terminated;
new runs emit timeout records directly. The body mutation/geometry phases of that
failed wrapped command remain unresolved. No PSS was collected by this isolated
probe, and no native folder handle was granted. Existing both-mode application
contracts and their performance gates remain separate.

Reproduce the matching fixture after building the frontend/backend, using the
existing measurement image (no new Cargo target):

```bash
docker run --rm --init --shm-size=1g --memory=10g --cpus=4 \
  --mount type=bind,src="$PWD",dst=/workspace/repos/openwebide \
  --entrypoint python3 openwebide:editor-view-measurements \
  tools/measure-editor-row-update.py --repeat 3 --source ascii unicode \
  --operations production-replace-data production-set-data production-replace-node \
  --timeout 60
```
