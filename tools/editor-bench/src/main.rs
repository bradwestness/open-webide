fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let start = std::time::Instant::now();
        let records = openwebide_editor_bench::measure(|| start.elapsed().as_secs_f64() * 1000.0);
        println!("operation,bytes,total_ms");
        for record in records {
            println!(
                "{},{},{:.3}",
                record.operation, record.bytes, record.milliseconds
            );
        }
    }
}
