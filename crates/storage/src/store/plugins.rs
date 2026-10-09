//! Database transport for the shared plugin installation policy.
use super::*;
use openwebide_core::plugins::{
    PluginError, PluginInstallation, RecordPlugin, record_installation,
};
const KEY: &str = "plugin_installations";

impl<D: Db> Store<D> {
    pub async fn plugin_installations(
        &self,
        user: UserId,
    ) -> Result<Vec<PluginInstallation>, StorageError> {
        self.get_user_setting(user, KEY).await?.map_or_else(
            || Ok(Vec::new()),
            |json| serde_json::from_str(&json).map_err(|error| StorageError::Db(error.to_string())),
        )
    }
    pub async fn record_plugin(
        &self,
        user: UserId,
        request: &RecordPlugin,
        now: i64,
    ) -> Result<Vec<PluginInstallation>, StorageError> {
        self.db
            .transaction(|tx| async move {
                let store = Store::new(tx);
                let current = store.plugin_installations(user).await?;
                let next =
                    record_installation(current, request, now).map_err(|error| match error {
                        PluginError::Conflict(message) => StorageError::Conflict(message),
                        PluginError::Invalid(message) => StorageError::InvalidRequest(message),
                        PluginError::Host(message) => StorageError::Db(message),
                    })?;
                let json = serde_json::to_string(&next)
                    .map_err(|error| StorageError::Db(error.to_string()))?;
                store.set_user_setting(user, KEY, &json).await?;
                Ok(next)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use openwebide_core::plugins::testing::receipt;
    #[test]
    fn installation_records_are_user_owned_atomic_idempotent_and_revision_checked() {
        futures::executor::block_on(async {
            let store = Store::new(RusqliteDb::open_in_memory().unwrap());
            store.migrate().await.unwrap();
            let owner = store
                .insert_user("owner", "hash", UserRole::Admin, 0)
                .await
                .unwrap();
            let other = store
                .insert_user("other", "hash", UserRole::User, 0)
                .await
                .unwrap();
            let request = RecordPlugin {
                prepared: receipt(),
                revision: None,
            };
            let entries = store.record_plugin(owner.id, &request, 1).await.unwrap();
            assert_eq!(entries.len(), 1);
            assert!(
                store
                    .plugin_installations(other.id)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                store.record_plugin(owner.id, &request, 2).await.unwrap(),
                entries
            );
            let mut changed = request.clone();
            changed.prepared.source.commit = "b".repeat(40);
            assert!(matches!(
                store.record_plugin(owner.id, &changed, 3).await,
                Err(StorageError::Conflict(_))
            ));
            assert_eq!(store.plugin_installations(owner.id).await.unwrap(), entries);
            changed.revision = Some(entries[0].revision);
            let entries = store.record_plugin(owner.id, &changed, 4).await.unwrap();
            assert_eq!(entries[0].prepared.source.commit, "b".repeat(40));
            assert!(matches!(
                store.record_plugin(owner.id, &changed, 5).await,
                Err(StorageError::Conflict(_))
            ));
        });
    }
}
