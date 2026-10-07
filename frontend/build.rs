fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("TARGET").as_deref() == Ok("wasm32-unknown-unknown") {
        // Nested Leptos views exceed the default 1 MiB stack in debug builds.
        // Keep app and browser-test links consistent without rebuilding dependencies.
        println!("cargo::rustc-link-arg=-z");
        println!("cargo::rustc-link-arg=stack-size=2097152");
    }
}
