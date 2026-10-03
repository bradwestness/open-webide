fn main() {
    #[cfg(target_arch = "wasm32")]
    openwebide_frontend::mount();
}
