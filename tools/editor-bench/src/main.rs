fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let start = std::time::Instant::now();
        let clock = || start.elapsed().as_secs_f64() * 1000.0;
        let records = if std::env::args().any(|argument| argument == "--lexical") {
            openwebide_editor_bench::measure_lexical(clock)
        } else {
            openwebide_editor_bench::measure(clock)
        };
        println!("operation,bytes,total_ms");
        for record in records {
            println!(
                "{},{},{:.3}",
                record.operation, record.bytes, record.milliseconds
            );
        }
    }
}
