//! Project-skills UI facade; database transport is identical for both workspace modes.
use crate::{
    backend::Api,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState, skills::SkillsState},
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::{ProjectSkill, SkillCommand};
#[derive(Clone, Copy)]
pub struct ProjectSkillActions {
    pub refresh: Callback<()>,
    pub command: Callback<SkillCommand>,
    pub edit: Callback<Option<ProjectSkill>>,
    pub save: Callback<()>,
    pub import: Callback<web_sys::FileList>,
    pub export: Callback<ProjectSkill>,
}
impl ProjectSkillActions {
    pub fn new(
        api: Api,
        state: SkillsState,
        auth: AuthState,
        projects: ProjectsState,
        chat: ChatState,
    ) -> Self {
        let generation = StoredValue::new(0u64);
        let refresh = Callback::new(move |()| {
            let Some(project) = projects.active_project.get_untracked() else {
                return;
            };
            if state.busy.get_untracked() {
                return;
            }
            generation.update_value(|value| *value += 1);
            let ticket = generation.get_value();
            let account = auth.generation.get_untracked();
            state.loading.set(true);
            spawn_local(async move {
                let Some(backend) = api.try_with_value(Clone::clone) else {
                    return;
                };
                let result = backend.project_skills(project).await;
                if auth.generation.try_get_untracked() != Some(account)
                    || projects.active_project.try_get_untracked() != Some(Some(project))
                    || generation.try_get_value() != Some(ticket)
                {
                    return;
                }
                state.loading.set(false);
                match result {
                    Ok(data) => {
                        state.data.set(Some(data));
                        state.error.set(None);
                    }
                    Err(error) => state.error.set(Some(error)),
                }
            });
        });
        let command = Callback::new(move |command: SkillCommand| {
            let Some(project) = projects.active_project.get_untracked() else {
                return;
            };
            if state.busy.get_untracked() {
                return;
            }
            if let Err(error) = command.validate() {
                state.error.set(Some(error));
                return;
            }
            generation.update_value(|value| *value += 1);
            let ticket = generation.get_value();
            let account = auth.generation.get_untracked();
            let saved = matches!(
                command,
                SkillCommand::Create { .. } | SkillCommand::Update { .. }
            );
            state.busy.set(true);
            state.loading.set(false);
            spawn_local(async move {
                let Some(backend) = api.try_with_value(Clone::clone) else {
                    return;
                };
                let result = backend.skill_command(project, &command, false).await;
                if auth.generation.try_get_untracked() != Some(account)
                    || projects.active_project.try_get_untracked() != Some(Some(project))
                    || generation.try_get_value() != Some(ticket)
                {
                    return;
                }
                state.busy.set(false);
                match result {
                    Ok(data) => {
                        state.data.set(Some(data));
                        state.error.set(None);
                        if saved {
                            state.editing.set(false);
                        }
                    }
                    Err(error) => {
                        // Keep the selected state after a failed persistence request.
                        state.data.update(|_| ());
                        state.error.set(Some(error));
                    }
                }
            });
        });
        let edit = Callback::new(move |entry: Option<ProjectSkill>| {
            state
                .edit_id
                .set(entry.as_ref().map(|entry| (entry.id, entry.revision)));
            state
                .draft
                .set(entry.map_or_else(crate::state::skills::empty_draft, |entry| entry.draft));
            state.editing.set(true);
            state.error.set(None);
        });
        let save = Callback::new(move |()| {
            let draft = state.draft.get_untracked();
            command.run(match state.edit_id.get_untracked() {
                Some((id, revision)) => SkillCommand::Update {
                    id,
                    revision,
                    draft,
                },
                None => SkillCommand::Create { draft },
            });
        });
        let import = Callback::new(move |files: web_sys::FileList| {
            if state.busy.get_untracked() {
                return;
            }
            let Some(project) = projects.active_project.get_untracked() else {
                return;
            };
            generation.update_value(|value| *value += 1);
            let ticket = generation.get_value();
            let account = auth.generation.get_untracked();
            let session = chat.active_session.get_untracked();
            let files = (0..files.length())
                .filter_map(|index| files.get(index))
                .collect::<Vec<_>>();
            state.busy.set(true);
            state.loading.set(false);
            spawn_local(async move {
                let result = read_import(files).await;
                if auth.generation.try_get_untracked() != Some(account)
                    || projects.active_project.try_get_untracked() != Some(Some(project))
                    || chat.active_session.try_get_untracked() != Some(session)
                    || generation.try_get_value() != Some(ticket)
                {
                    return;
                }
                state.busy.set(false);
                match result {
                    Ok(draft) => {
                        state.edit_id.set(None);
                        state.draft.set(draft);
                        state.editing.set(true);
                        state.error.set(None);
                    }
                    Err(error) => state.error.set(Some(error)),
                }
            });
        });
        let export = Callback::new(move |entry: ProjectSkill| {
            if state.busy.get_untracked() {
                return;
            }
            match openwebide_core::skills::archive::export_zip(&entry.draft)
                .and_then(|bytes| download_zip(&format!("{}.zip", entry.draft.name), &bytes))
            {
                Ok(()) => state.error.set(None),
                Err(error) => state.error.set(Some(error)),
            }
        });
        Effect::new(move |_| {
            auth.generation.get();
            projects.active_project.get();
            chat.active_session.get();
            generation.update_value(|value| *value += 1);
            state.data.set(None);
            state.error.set(None);
            state.editing.set(false);
            state.draft.set(crate::state::skills::empty_draft());
            state.edit_id.set(None);
            state.busy.set(false);
            state.loading.set(false);
            refresh.run(());
        });
        let was_streaming = StoredValue::new(false);
        Effect::new(move |_| {
            let streaming = chat.streaming.get();
            let previous = was_streaming.get_value();
            was_streaming.set_value(streaming);
            if previous && !streaming {
                refresh.run(());
            }
        });
        let timer = leptos::leptos_dom::helpers::set_interval_with_handle(
            move || refresh.run(()),
            std::time::Duration::from_secs(10),
        )
        .ok();
        on_cleanup(move || {
            if let Some(timer) = timer {
                timer.clear();
            }
        });
        Self {
            refresh,
            command,
            edit,
            save,
            import,
            export,
        }
    }
}

/// Browser picker primitive; decoding and format policy live in the shared core.
async fn read_import(files: Vec<web_sys::File>) -> Result<openwebide_core::SkillDraft, String> {
    use openwebide_core::skills::archive::{
        ImportFile, MAX_ARCHIVE_BYTES, import_archive, import_files,
    };
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;
    if files.is_empty() || files.len() > 17 {
        return Err("Choose one skill with at most 16 supporting files".into());
    }
    let mut imported = Vec::new();
    let mut total = 0.0;
    for file in files {
        total += file.size();
        if total > MAX_ARCHIVE_BYTES as f64 {
            return Err("Skill import is limited to 1 MiB".into());
        }
        let relative =
            js_sys::Reflect::get(file.as_ref(), &JsValue::from_str("webkitRelativePath"))
                .ok()
                .and_then(|value| value.as_string())
                .filter(|value| !value.is_empty());
        let name = relative.unwrap_or_else(|| file.name());
        let buffer = JsFuture::from(file.array_buffer())
            .await
            .map_err(|error| format!("Cannot read selected file: {error:?}"))?;
        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
        imported.push(ImportFile { name, bytes });
    }
    if imported.len() == 1
        && imported[0]
            .name
            .rsplit('.')
            .next()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        return openwebide_core::skills::archive::import_markdown(
            std::str::from_utf8(&imported[0].bytes)
                .map_err(|_| "Skill file must contain UTF-8 text")?,
        );
    }
    if imported.len() == 1 {
        return import_archive(&imported[0].name, &imported[0].bytes);
    }
    import_files(imported)
}
fn download_zip(name: &str, bytes: &[u8]) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or("Browser document unavailable")?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/zip");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|error| format!("{error:?}"))?;
    let url =
        web_sys::Url::create_object_url_with_blob(&blob).map_err(|error| format!("{error:?}"))?;
    let result = (|| {
        let anchor = document
            .create_element("a")
            .map_err(|error| format!("{error:?}"))?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "Download link unavailable")?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor.click();
        Ok(())
    })();
    spawn_local(async move {
        crate::util::sleep_ms(1000).await;
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    result
}
