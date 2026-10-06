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

Both ropes make isolated edits and indexed queries much cheaper. In this workload, recreating the full display string makes candidate edits slower than the current document. Retain current production storage while implementing worker/viewport rendering; choose storage against the resulting access pattern rather than adding a mirrored rope alongside a full string. Current whole-source offset conversions remain a measured cost.

## Long wrapped lines

The existing both-mode Unicode/CRLF regression keeps two cursors deep inside the same large wrapped line. It checks exact Down/Up restoration, unchanged source/history and fewer than 2,000 DOM range measurements. Paint now splits oversized tokens into Unicode-safe text runs, retaining complete grapheme clusters and escaped source. In the local debug browser fixture, splitting runs reduced elapsed fixture time from 9.05 seconds to 1.27 seconds; a Down press used 225 ranges in approximately 130 ms. This is a regression observation, not a claim of final large-file responsiveness.

Syntax preparation now runs in a Rust/WASM worker, with bounded shared cache retention, validated coordinates and coalesced requests. The built-worker Chrome check exercises all providers, incremental Unicode/CRLF, a UI event during preparation and oversized-source fallback. These correctness checks do not measure total editor memory or latency.

Remaining work: full viewport paint, end-to-end input/scroll latency and memory benchmarks, full-editor large-file fallbacks and real-device verification. Storage microbenchmarks do not prove any of those complete.
