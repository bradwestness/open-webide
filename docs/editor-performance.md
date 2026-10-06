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
scope so diagnostic timing cannot mislabel jobs with newer state. Traces record
style and revision attributes, never source text. These instrumented observations
perturb timings and establish probe counts rather than latency thresholds.
Both builds include concurrent branding/welcome changes; module identifiers
identify the builds and `checkoutHead` their parent commit. Initial full-row
shaping, native textarea/input and full String costs, bidirectional windows,
Linux PSS, percentiles and boundary/device gates remain outstanding.
