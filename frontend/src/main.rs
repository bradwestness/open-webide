mod api;
mod app;
mod components;
mod idb;
mod local_fs;
mod workspace;

fn main() {
    leptos::mount::mount_to_body(app::App);
}
