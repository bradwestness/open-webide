use openwebide_core::editor::{Document, Edit, Selection};
use std::hint::black_box;
#[derive(Clone, Debug)]
pub struct Measurement {
    pub operation: &'static str,
    pub bytes: usize,
    pub milliseconds: f64,
}
fn time(
    records: &mut Vec<Measurement>,
    clock: &impl Fn() -> f64,
    label: &'static str,
    bytes: usize,
    f: impl FnOnce(),
) {
    let start = clock();
    f();
    records.push(Measurement {
        operation: label,
        bytes,
        milliseconds: clock() - start,
    });
}
/// Run identical storage workloads with a platform clock supplied by the caller.
/// Ropes are comparison dependencies only; the production document stays unchanged.
pub fn measure(clock: impl Fn() -> f64) -> Vec<Measurement> {
    let mut records = Vec::new();
    for size in [64 * 1024, 2 * 1024 * 1024, 16 * 1024 * 1024] {
        let pattern = "fn sample() { let text = \"文😀e\u{301}\"; }\r\n";
        let source = pattern.repeat(size / pattern.len());
        let mut at = source.len() / 2;
        while !source.is_char_boundary(at) {
            at -= 1;
        }
        let mut doc = Document::new(source.clone());
        time(
            &mut records,
            &clock,
            "document_100_edits",
            source.len(),
            || {
                for _ in 0..50 {
                    doc.apply(
                        vec![Edit::replace(at..at, "x")],
                        vec![Selection::caret(at + 1)],
                        None,
                    )
                    .unwrap();
                    doc.undo();
                }
            },
        );
        assert_eq!(doc.text(), source);
        let mut text = source.clone();
        time(
            &mut records,
            &clock,
            "string_copy_100_edits",
            source.len(),
            || {
                for _ in 0..50 {
                    text = [&text[..at], "x", &text[at..]].concat();
                    text = [&text[..at], &text[at + 1..]].concat();
                    black_box(&text);
                }
            },
        );
        assert_eq!(text, source);
        let expected = source[..at].replace("\r\n", "\n").encode_utf16().count();
        assert_eq!(
            openwebide_core::editor::byte_to_textarea(&source, at).unwrap(),
            expected
        );
        time(
            &mut records,
            &clock,
            "document_100_textarea_queries",
            source.len(),
            || {
                for _ in 0..100 {
                    black_box(
                        openwebide_core::editor::byte_to_textarea(&source, black_box(at)).unwrap(),
                    );
                }
            },
        );
        time(
            &mut records,
            &clock,
            "string_100_raw_utf16_queries",
            source.len(),
            || {
                for _ in 0..100 {
                    black_box(source[..black_box(at)].encode_utf16().count());
                }
            },
        );
        #[cfg(feature = "candidates")]
        for materialize in [false, true] {
            let mut crop = crop::Rope::from(source.as_str());
            time(
                &mut records,
                &clock,
                if materialize {
                    "crop_100_edits_with_view"
                } else {
                    "crop_100_edits"
                },
                source.len(),
                || {
                    for _ in 0..50 {
                        crop.insert(at, "x");
                        if materialize {
                            black_box(crop.to_string());
                        }
                        crop.delete(at..at + 1);
                        if materialize {
                            black_box(crop.to_string());
                        }
                    }
                },
            );
            assert_eq!(crop.to_string(), source);
            let mut rope = ropey::Rope::from_str(&source);
            time(
                &mut records,
                &clock,
                if materialize {
                    "ropey_100_edits_with_view"
                } else {
                    "ropey_100_edits"
                },
                source.len(),
                || {
                    for _ in 0..50 {
                        let char_at = rope.byte_to_char(at);
                        rope.insert(char_at, "x");
                        if materialize {
                            black_box(rope.to_string());
                        }
                        rope.remove(char_at..char_at + 1);
                        if materialize {
                            black_box(rope.to_string());
                        }
                    }
                },
            );
            assert_eq!(rope.to_string(), source);
        }
        #[cfg(feature = "candidates")]
        {
            let crop = crop::Rope::from(source.as_str());
            let rope = ropey::Rope::from_str(&source);
            let expected = source[..at].encode_utf16().count();
            assert_eq!(crop.utf16_code_unit_of_byte(at), expected);
            assert_eq!(rope.char_to_utf16_cu(rope.byte_to_char(at)), expected);
            time(
                &mut records,
                &clock,
                "crop_100_raw_utf16_queries",
                source.len(),
                || {
                    for _ in 0..100 {
                        black_box(crop.utf16_code_unit_of_byte(black_box(at)));
                    }
                },
            );
            time(
                &mut records,
                &clock,
                "ropey_100_raw_utf16_queries",
                source.len(),
                || {
                    for _ in 0..100 {
                        black_box(rope.char_to_utf16_cu(rope.byte_to_char(black_box(at))));
                    }
                },
            );
        }
    }
    records
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser {
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn shared_storage_workloads() {
        let clock = js_sys::Function::new_no_args("return performance.now()");
        let records = super::measure(|| clock.call0(&js_sys::global()).unwrap().as_f64().unwrap());
        for record in records {
            wasm_bindgen_test::console_log!(
                "{},{},{:.3}",
                record.operation,
                record.bytes,
                record.milliseconds
            );
            assert!(record.milliseconds.is_finite() && record.milliseconds >= 0.0);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native {
    #[test]
    fn shared_storage_workloads() {
        let start = std::time::Instant::now();
        let records = super::measure(|| start.elapsed().as_secs_f64() * 1000.0);
        let expected = if cfg!(feature = "candidates") { 30 } else { 12 };
        assert_eq!(records.len(), expected);
        assert!(
            records
                .iter()
                .all(|record| record.milliseconds.is_finite() && record.milliseconds >= 0.0)
        );
    }
}
