mod api;
mod app;
mod components;

fn main() {
    leptos::mount::mount_to_body(app::App);
}
