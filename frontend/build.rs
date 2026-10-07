fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=../tools/open_source.py");
    println!("cargo::rerun-if-env-changed=OPENWEBIDE_BUILD_COMMIT");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let result = std::process::Command::new("python3")
        .args(["../tools/open_source.py", "--fragment-output"])
        .arg(output.join("about.html"))
        .output()
        .expect("Python 3 is required to generate the open-source notices");
    assert!(
        result.status.success(),
        "Notice generation failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    print!("{}", String::from_utf8_lossy(&result.stdout));
    if std::env::var("TARGET").as_deref() == Ok("wasm32-unknown-unknown") {
        // Nested Leptos views exceed the default 1 MiB stack in debug builds.
        // Keep app and browser-test links consistent without rebuilding dependencies.
        println!("cargo::rustc-link-arg=-z");
        println!("cargo::rustc-link-arg=stack-size=2097152");
    }
}
