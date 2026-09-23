mod api;
mod app;
mod components;
mod idb;
mod local_agent;
mod local_fs;
mod workspace;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into());
    }));
    leptos::mount::mount_to_body(app::App);
}
