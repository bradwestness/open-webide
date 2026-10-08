use openwebide_editor_layout_probe::{FEATURES, advance_diagnostics, shape};
fn main() {
    for (name, text) in [
        ("ascii", "fn main() { let value = 123; }".into()),
        ("tab", "a\tb\tc".into()),
        ("unicode", "e\u{301} 文🦀 mixed".into()),
        ("bidi", "LTR אבג 123 مرحبا text".into()),
        ("long", "word -> value === ".repeat(55000)),
    ] {
        for wrap in [None, Some(300.0)] {
            let begin = std::time::Instant::now();
            let result = shape(&text, wrap, FEATURES);
            println!(
                "{}",
                serde_json::json!({"case":name,"bytes":text.len(),"wrap":wrap,"ms":begin.elapsed().as_secs_f64()*1000.0,"width":result.width(),"height":result.height(),"lines":result.lines().len(),"advances":advance_diagnostics(&result)})
            );
        }
    }
}
