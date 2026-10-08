//! Bounded, user-owned cache for background assistance.
use super::*;
impl<D: Db> Store<D> {
    pub async fn cached_assistance(
        &self,
        user: UserId,
        request: &str,
    ) -> Result<Option<String>, StorageError> {
        let rows = self
            .db
            .execute(
                "SELECT result FROM assistance_cache WHERE user_id = ? AND request = ?",
                &[DbValue::Int(user.get()), DbValue::Text(request.into())],
            )
            .await?;
        Ok(rows
            .rows
            .first()
            .and_then(|row| row.get_text_opt(0))
            .map(str::to_owned))
    }
    pub fn cache_assistance<'a>(
        &'a self,
        user: UserId,
        request: &'a str,
        result: &'a str,
        now: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StorageError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.db.transaction(|tx| async move {
            tx.execute("INSERT INTO assistance_cache(user_id, request, result, created_at) VALUES (?, ?, ?, ?) ON CONFLICT(user_id, request) DO UPDATE SET result = excluded.result, created_at = excluded.created_at", &[DbValue::Int(user.get()), DbValue::Text(request.into()), DbValue::Text(result.into()), DbValue::Int(now)]).await?;
            tx.execute("DELETE FROM assistance_cache WHERE user_id = ? AND id NOT IN (SELECT id FROM assistance_cache WHERE user_id = ? ORDER BY created_at DESC, id DESC LIMIT 128)", &[DbValue::Int(user.get()), DbValue::Int(user.get())]).await?;
            Ok(())
        }).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    #[test]
    fn assistance_cache_is_owned_durable_and_bounded() {
        futures::executor::block_on(async {
            let store = Store::new(RusqliteDb::open_in_memory().unwrap());
            store.migrate().await.unwrap();
            store.migrate().await.unwrap();
            let owner = store
                .insert_user("owner", "hash", UserRole::Admin, 1)
                .await
                .unwrap()
                .id;
            let other = store
                .insert_user("other", "hash", UserRole::User, 1)
                .await
                .unwrap()
                .id;
            store
                .cache_assistance(owner, "same input", "Private recap", 1)
                .await
                .unwrap();
            assert_eq!(
                store.cached_assistance(owner, "same input").await.unwrap(),
                Some("Private recap".into())
            );
            assert_eq!(
                store.cached_assistance(other, "same input").await.unwrap(),
                None
            );
            store
                .cache_assistance(other, "same input", "Other recap", 1)
                .await
                .unwrap();
            for index in 0..128 {
                store
                    .cache_assistance(owner, &format!("input-{index}"), "Result", index + 2)
                    .await
                    .unwrap();
            }
            assert_eq!(
                store.cached_assistance(owner, "same input").await.unwrap(),
                None
            );
            assert_eq!(
                store.cached_assistance(owner, "input-127").await.unwrap(),
                Some("Result".into())
            );
            assert_eq!(
                store.cached_assistance(other, "same input").await.unwrap(),
                Some("Other recap".into())
            );
        });
    }
}
