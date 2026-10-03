use super::*;
use openwebide_core::{
    ModelDefaults, ModelProfile, ModelSelection, ModelSettings, ModelSetup, ServerSettings,
    ServerSettingsUpdate, ServerTransport,
};

fn encode(value: &impl serde::Serialize) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|error| StorageError::Db(error.to_string()))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, StorageError> {
    serde_json::from_str(value).map_err(|error| StorageError::Db(error.to_string()))
}
impl<D: Db> Store<D> {
    pub async fn model_setup(&self, user: UserId) -> Result<ModelSetup, StorageError> {
        let defaults = self
            .get_user_setting(user, "model_defaults")
            .await?
            .map(|value| decode::<ModelDefaults>(&value))
            .transpose()?
            .unwrap_or_default();
        let rows = self
            .db
            .execute(
                "SELECT server_id, model, settings FROM model_profiles ORDER BY server_id, model",
                &[],
            )
            .await?;
        let profiles = rows
            .rows
            .iter()
            .map(|row| {
                Ok(ModelProfile {
                    selection: ModelSelection {
                        server_id: row.get_int(0)?,
                        model: row.get_text(1)?.into(),
                    },
                    settings: decode::<ModelSettings>(row.get_text(2)?)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(ModelSetup { defaults, profiles })
    }
    async fn validate_model_selection(
        &self,
        selection: &ModelSelection,
    ) -> Result<(), StorageError> {
        if selection.model.trim().is_empty() {
            return Err(StorageError::InvalidValue("Model name is required.".into()));
        }
        if !self.get_connection(selection.server_id).await?.enabled {
            return Err(StorageError::InvalidValue("Server is disabled.".into()));
        }
        Ok(())
    }
    pub async fn save_model_defaults(
        &self,
        user: UserId,
        defaults: &ModelDefaults,
    ) -> Result<(), StorageError> {
        for selection in [&defaults.primary, &defaults.fast].into_iter().flatten() {
            self.validate_model_selection(selection).await?;
        }
        self.set_user_setting(user, "model_defaults", &encode(defaults)?)
            .await
    }
    pub async fn save_model_profile(&self, profile: &ModelProfile) -> Result<(), StorageError> {
        self.validate_model_selection(&profile.selection).await?;
        profile
            .settings
            .validate()
            .map_err(StorageError::InvalidValue)?;
        let mut settings = profile.settings.clone();
        settings.fast = None;
        self.db
            .execute(
                "INSERT INTO model_profiles (server_id, model, settings) VALUES (?, ?, ?)
            ON CONFLICT(server_id, model) DO UPDATE SET settings = excluded.settings",
                &[
                    DbValue::Int(profile.selection.server_id),
                    DbValue::Text(profile.selection.model.clone()),
                    DbValue::Text(encode(&settings)?),
                ],
            )
            .await?;
        Ok(())
    }
    pub async fn server_transport(&self, id: i64) -> Result<ServerTransport, StorageError> {
        self.get_connection(id).await?;
        let rows = self
            .db
            .execute(
                "SELECT settings FROM server_transport WHERE server_id = ?",
                &[DbValue::Int(id)],
            )
            .await?;
        rows.rows
            .first()
            .map(|row| decode::<ServerTransport>(row.get_text(0)?))
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub async fn server_settings(&self, id: i64) -> Result<ServerSettings, StorageError> {
        let transport = self.server_transport(id).await?;
        Ok(ServerSettings {
            has_api_key: transport.api_key.is_some(),
            header_names: transport.headers.keys().cloned().collect(),
            timeout_seconds: transport.timeout_seconds,
            keep_alive: transport.keep_alive,
        })
    }
    pub async fn save_server_settings(
        &self,
        id: i64,
        update: &ServerSettingsUpdate,
    ) -> Result<(), StorageError> {
        let mut transport = self.server_transport(id).await?;
        if update.clear_api_key {
            transport.api_key = None;
        } else if let Some(key) = &update.api_key {
            if key.contains(['\r', '\n']) {
                return Err(StorageError::InvalidValue("Invalid API key.".into()));
            }
            transport.api_key = Some(key.clone());
        }
        if let Some(headers) = &update.headers {
            for (name, value) in headers {
                if name.is_empty()
                    || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || value.contains(['\r', '\n'])
                    || [
                        "host",
                        "authorization",
                        "cookie",
                        "content-length",
                        "connection",
                        "transfer-encoding",
                    ]
                    .contains(&name.to_ascii_lowercase().as_str())
                {
                    return Err(StorageError::InvalidValue(
                        "Invalid extra header; use the API key field for authorization.".into(),
                    ));
                }
            }
            transport.headers.clone_from(headers);
        }
        if let Some(timeout) = update.timeout_seconds {
            if !(1..=3600).contains(&timeout) {
                return Err(StorageError::InvalidValue(
                    "Timeout must be 1–3600 seconds.".into(),
                ));
            }
            transport.timeout_seconds = timeout;
        }
        if let Some(keep_alive) = &update.keep_alive {
            transport.keep_alive = (!keep_alive.trim().is_empty()).then(|| keep_alive.clone());
        }
        self.db.execute("INSERT INTO server_transport (server_id, settings) VALUES (?, ?) ON CONFLICT(server_id) DO UPDATE SET settings = excluded.settings", &[DbValue::Int(id), DbValue::Text(encode(&transport)?)]).await?;
        Ok(())
    }
}
