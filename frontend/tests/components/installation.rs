use super::support::{mount_test, settle, wait_until};
use leptos::prelude::*;
use openwebide_core::WorkspaceMode;
use openwebide_frontend::components::InstallApp;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
let prompts = 0;
export function install_fixture(secure, installed, available) {
    prompts = 0;
    window.webideInstall = {
        state: () => ({secure, installed, available}),
        prompt: async () => { prompts++; available = false; return true; }
    };
    window.dispatchEvent(new Event('webide-install-change'));
}
export function install_prompts() { return prompts; }
export function clear_install_fixture() { delete window.webideInstall; }
"#)]
extern "C" {
    fn install_fixture(secure: bool, installed: bool, available: bool);
    fn install_prompts() -> u32;
    fn clear_install_fixture();
}

#[wasm_bindgen_test]
async fn installation_guidance_and_browser_prompt_are_shared_across_workspaces() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        // HTTP LAN, secure localhost/HTTPS without a browser prompt, and installed app.
        for (secure, installed, available) in [
            (false, false, false),
            (true, false, false),
            (true, false, true),
            (false, true, false),
        ] {
            install_fixture(secure, installed, available);
            let mounted = mount_test(move |state| {
                if let Some(mode) = mode {
                    state.seed_project();
                    state
                        .projects
                        .projects
                        .update(|projects| projects[0].mode = mode);
                }
                view! { <InstallApp /> }
            });
            settle().await;
            let text = mounted.root.text_content().unwrap();
            if installed {
                assert!(text.contains("Open WebIDE is installed"));
                assert!(!text.contains("Set up HTTPS"));
            } else if !secure {
                assert!(text.contains("open it over HTTPS"));
                assert!(
                    mounted
                        .element("a")
                        .get_attribute("href")
                        .unwrap()
                        .ends_with("docs/tailscale.md")
                );
            } else if available {
                assert_eq!(install_prompts(), 0);
                mounted.click(".install-app-setting button");
                wait_until("consumed installation gesture", || {
                    install_prompts() == 1
                        && mounted.root.query_selector("button").unwrap().is_none()
                })
                .await;
                assert!(
                    mounted
                        .root
                        .text_content()
                        .unwrap()
                        .contains("browser’s Install app")
                );
            } else {
                assert!(text.contains("Add to Home Screen"));
                assert!(!text.contains("Set up HTTPS"));
            }
            drop(mounted);
            clear_install_fixture();
        }
    }
}

#[wasm_bindgen_test]
async fn installed_display_change_hides_the_https_nudge() {
    install_fixture(false, false, false);
    let mounted = mount_test(|_| view! { <InstallApp /> });
    settle().await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Set up HTTPS")
    );
    install_fixture(false, true, false);
    settle().await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Open WebIDE is installed")
    );
    assert!(
        !mounted
            .root
            .text_content()
            .unwrap()
            .contains("Set up HTTPS")
    );
    drop(mounted);
    clear_install_fixture();
}
