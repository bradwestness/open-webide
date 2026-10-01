#[cfg(target_arch = "wasm32")]
pub mod api;
#[cfg(target_arch = "wasm32")]
pub mod app;
#[cfg(target_arch = "wasm32")]
pub mod backend;
#[cfg(target_arch = "wasm32")]
pub mod bridge;
#[cfg(target_arch = "wasm32")]
pub mod components;
pub mod conversation;
#[cfg(target_arch = "wasm32")]
pub mod idb;
#[cfg(target_arch = "wasm32")]
pub mod local_agent;
#[cfg(target_arch = "wasm32")]
pub mod local_fs;
pub mod markdown;
pub mod pending;
pub mod sse;
pub mod state;
#[cfg(target_arch = "wasm32")]
pub mod state_actions;
#[cfg(all(target_arch = "wasm32", any(test, feature = "test-support")))]
pub mod testing;
pub mod text;
pub mod vfs_err;
#[cfg(target_arch = "wasm32")]
pub mod workspace;

#[cfg(target_arch = "wasm32")]
pub fn mount() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into());
    }));
    leptos::mount::mount_to_body(app::App);
}
