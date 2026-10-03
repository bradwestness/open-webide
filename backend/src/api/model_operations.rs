//! Model runtime primitives for shared background workflows.
use super::*;
use crate::state::AppDb;
use openwebide_agent::compaction::CompactionSource;
use openwebide_storage::Store;

pub(crate) struct ModelSource {
    pub store: Arc<Store<AppDb>>,
    pub user: UserId,
}
impl CompactionSource for ModelSource {
    fn available(&self) -> bool {
        true
    }
    async fn runtime(
        &self,
        selection: &openwebide_core::ModelSelection,
    ) -> Result<openwebide_core::ModelRuntime, String> {
        super::model_setup::runtime_store(
            &self.store,
            self.user,
            selection.server_id,
            Some(&selection.model),
        )
        .await
        .map_err(|error| error.to_string())
    }
    async fn complete(
        &self,
        request: &ChatRequest,
    ) -> Result<openwebide_core::ChatCompletion, String> {
        complete(&self.store, self.user, request.clone())
            .await
            .map_err(|error| error.to_string())
    }
    async fn context_limit(&self, request: &ChatRequest) -> Option<usize> {
        let mut request = request.clone();
        provider(&self.store, self.user, &mut request, true)
            .await
            .ok()?
            .context_limit(request.model.as_deref())
            .await
            .ok()
            .flatten()
    }
    async fn tokens(&self, request: &ChatRequest) -> Option<usize> {
        tokens(&self.store, self.user, request.clone())
            .await
            .ok()
            .flatten()
    }
}
async fn provider(
    store: &Store<AppDb>,
    user: UserId,
    request: &mut ChatRequest,
    quick: bool,
) -> Result<Provider<SpinHttpClient>, ApiError> {
    request
        .model_settings
        .validate()
        .map_err(ApiError::bad_request)?;
    let mut runtime = super::model_setup::runtime_store(
        store,
        user,
        request.connection_id,
        request.model.as_deref(),
    )
    .await?;
    if quick {
        runtime.transport.timeout_seconds = runtime.transport.timeout_seconds.min(5);
    }
    // Explicit operation settings are already resolved by the shared workflow.
    request.model = runtime.connection.model.clone();
    request.model_settings.context_limit = runtime.settings.context_limit;
    request.model_settings.max_output_tokens =
        request.model_settings.max_output_tokens.map(|output| {
            output
                .min(runtime.settings.max_output_tokens.unwrap_or(usize::MAX))
                .min(runtime.settings.context_limit.unwrap_or(usize::MAX))
        });
    Ok(Provider::for_connection(
        &runtime.connection,
        SpinHttpClient::default().with_transport(runtime.transport),
    ))
}
pub(crate) async fn complete(
    store: &Store<AppDb>,
    user: UserId,
    mut request: ChatRequest,
) -> Result<openwebide_core::ChatCompletion, ApiError> {
    provider(store, user, &mut request, false)
        .await?
        .chat_tools(&request)
        .await
        .map_err(Into::into)
}
pub(crate) async fn tokens(
    store: &Store<AppDb>,
    user: UserId,
    mut request: ChatRequest,
) -> Result<Option<usize>, ApiError> {
    Ok(provider(store, user, &mut request, true)
        .await?
        .request_tokens(&request)
        .await)
}
pub(crate) async fn route(
    req: Request,
    state: &AppState,
    user: AuthedUser,
    count: bool,
) -> Result<JsonResp, ApiError> {
    let request = parse_json(read_body(req, CHAT_BODY_LIMIT).await?)?;
    if count {
        Ok(json_response(
            200,
            &tokens(&state.store, user.id, request).await?,
        ))
    } else {
        Ok(json_response(
            200,
            &complete(&state.store, user.id, request).await?,
        ))
    }
}
