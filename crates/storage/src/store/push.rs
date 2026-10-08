//! User-owned subscriptions and durable, leased Web Push deliveries.
use super::*;
use openwebide_core::push::{PushSubscription, RunNotification};

#[derive(Debug)]
pub struct PushDelivery {
    pub id: i64,
    pub subscription_id: i64,
    pub user_id: UserId,
    pub subscription: PushSubscription,
    pub payload: String,
    pub attempts: i64,
}
impl<D: Db> Store<D> {
    pub async fn save_push_subscription(
        &self,
        user: UserId,
        subscription: &PushSubscription,
        now: i64,
    ) -> Result<(), StorageError> {
        subscription
            .validate()
            .map_err(StorageError::InvalidValue)?;
        let data = serde_json::to_string(subscription)
            .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
        self.db.transaction(move |tx| async move {
            // A browser subscription belongs to one account. Reassignment invalidates old queued deliveries.
            tx.execute("DELETE FROM push_subscriptions WHERE endpoint = ? AND user_id != ?", &[DbValue::Text(subscription.endpoint.clone()), DbValue::Int(user.get())]).await?;
            tx.execute("INSERT INTO push_subscriptions (user_id, endpoint, subscription, created_at)
                SELECT ?, ?, ?, ? WHERE (SELECT COUNT(*) FROM push_subscriptions WHERE user_id = ?) < 32
                OR EXISTS (SELECT 1 FROM push_subscriptions WHERE user_id = ? AND endpoint = ?)
                ON CONFLICT(endpoint) DO UPDATE SET subscription = excluded.subscription", &[
                    DbValue::Int(user.get()), DbValue::Text(subscription.endpoint.clone()), DbValue::Text(data), DbValue::Int(now),
                    DbValue::Int(user.get()), DbValue::Int(user.get()), DbValue::Text(subscription.endpoint.clone())]).await?;
            let found = tx.execute("SELECT 1 FROM push_subscriptions WHERE user_id = ? AND endpoint = ?", &[DbValue::Int(user.get()), DbValue::Text(subscription.endpoint.clone())]).await?;
            if found.rows.is_empty() { return Err(StorageError::InvalidValue("At most 32 notification devices are supported".into())); }
            Ok(())
        }).await
    }
    pub async fn has_push_subscription(
        &self,
        user: UserId,
        endpoint: &str,
    ) -> Result<bool, StorageError> {
        Ok(!self
            .db
            .execute(
                "SELECT 1 FROM push_subscriptions WHERE user_id = ? AND endpoint = ?",
                &[DbValue::Int(user.get()), DbValue::Text(endpoint.into())],
            )
            .await?
            .rows
            .is_empty())
    }
    pub async fn remove_push_subscription(
        &self,
        user: UserId,
        endpoint: &str,
    ) -> Result<(), StorageError> {
        self.db
            .execute(
                "DELETE FROM push_subscriptions WHERE user_id = ? AND endpoint = ?",
                &[DbValue::Int(user.get()), DbValue::Text(endpoint.into())],
            )
            .await?;
        Ok(())
    }
    pub async fn remove_user_push_subscriptions(&self, user: UserId) -> Result<(), StorageError> {
        self.db
            .execute(
                "DELETE FROM push_subscriptions WHERE user_id = ?",
                &[DbValue::Int(user.get())],
            )
            .await?;
        Ok(())
    }
    pub fn queue_run_notification<'a>(
        &'a self,
        user: UserId,
        session: i64,
        event: &'a RunNotification,
        now: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StorageError>> + Send + 'a>>
    {
        Box::pin(async move {
            event.validate().map_err(StorageError::InvalidValue)?;
            let entry = self.get_session(session, user).await?;
            let project = match entry.project_id {
                Some(id) => Some(self.get_project(id, user).await?),
                None => None,
            };
            let valid = match event {
            RunNotification::Finished { message_id } => self.db.execute("SELECT 1 FROM messages WHERE id = ? AND session_id = ? AND role = 'assistant' AND (tool_calls IS NULL OR tool_calls = '[]')", &[DbValue::Int(*message_id), DbValue::Int(session)]).await?,
            RunNotification::Approval { tool_call_id } => self.db.execute("SELECT 1 FROM tool_steps WHERE session_id = ? AND tool_call_id = ? AND ok IS NULL", &[DbValue::Int(session), DbValue::Text(tool_call_id.clone())]).await?,
        };
            if valid.rows.is_empty() {
                return Err(StorageError::InvalidValue(
                    "Notification event is not persisted in this session".into(),
                ));
            }
            let event_key = event.key();
            let event_json = serde_json::to_string(event)
                .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
            let mut payload = event.payload(
                user,
                session,
                project.as_ref().map(|project| project.name.as_str()),
                &entry.name,
            );
            if let RunNotification::Finished { message_id } = event {
                let messages = self.list_messages(session).await?;
                if let Some(message) = messages.iter().find(|message| message.id == *message_id) {
                    let summary = openwebide_core::assistance::completion_excerpt(&message.content);
                    if !summary.is_empty() {
                        payload.body = format!(
                            "{} — {summary}",
                            entry.name.chars().take(80).collect::<String>()
                        );
                    }
                }
            }
            let payload = serde_json::to_string(&payload)
                .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
            let ttl = if matches!(event, RunNotification::Approval { .. }) {
                300
            } else {
                3600
            };
            self.db.transaction(move |tx| async move {
            // Host-backed runs in every workspace share durable delivery.
            let enabled = tx.execute("SELECT 1 FROM sessions s LEFT JOIN projects p ON p.id = s.project_id
                WHERE s.id = ? AND s.user_id = ?
                AND EXISTS (SELECT 1 FROM user_settings WHERE user_id = ? AND key = 'browser_notifications' AND value = 'true')", &[DbValue::Int(session), DbValue::Int(user.get()), DbValue::Int(user.get())]).await?;
            if enabled.rows.is_empty() { return Ok(()); }
            tx.execute("DELETE FROM push_notifications WHERE created_at < ?", &[DbValue::Int(now - 7 * 86400)]).await?;
            let inserted = tx.execute("INSERT OR IGNORE INTO push_notifications (user_id, session_id, event_key, created_at) VALUES (?, ?, ?, ?)", &[DbValue::Int(user.get()), DbValue::Int(session), DbValue::Text(event_key), DbValue::Int(now)]).await?;
            if inserted.changes == 0 { return Ok(()); }
            tx.execute("INSERT INTO push_deliveries (subscription_id, session_id, payload, event, next_attempt, expires_at)
                SELECT id, ?, ?, ?, ?, ? FROM push_subscriptions WHERE user_id = ?", &[
                    DbValue::Int(session), DbValue::Text(payload), DbValue::Text(event_json), DbValue::Int(now), DbValue::Int(now + ttl), DbValue::Int(user.get())]).await?;
            Ok(())
        }).await
        })
    }
    pub async fn claim_push_deliveries(&self, now: i64) -> Result<Vec<PushDelivery>, StorageError> {
        self.db.transaction(move |tx| async move {
            tx.execute("DELETE FROM push_deliveries WHERE expires_at <= ? OR attempts >= 6", &[DbValue::Int(now)]).await?;
            tx.execute("DELETE FROM push_deliveries WHERE subscription_id IN (SELECT id FROM push_subscriptions s WHERE NOT EXISTS (SELECT 1 FROM user_settings u WHERE u.user_id = s.user_id AND u.key = 'browser_notifications' AND u.value = 'true'))", &[]).await?;
            // Approval results or decisions invalidate queued approval alerts.
            tx.execute("DELETE FROM push_deliveries WHERE json_extract(event, '$.kind') = 'approval' AND (
                NOT EXISTS (SELECT 1 FROM tool_steps t WHERE t.session_id = push_deliveries.session_id AND t.tool_call_id = json_extract(push_deliveries.event, '$.tool_call_id') AND t.ok IS NULL)
                OR EXISTS (SELECT 1 FROM tool_permissions p WHERE p.session_id = push_deliveries.session_id AND p.tool_call_id = json_extract(push_deliveries.event, '$.tool_call_id')))", &[]).await?;
            let rows = tx.execute("SELECT d.id, s.id, s.user_id, s.subscription, d.payload, d.attempts FROM push_deliveries d JOIN push_subscriptions s ON s.id = d.subscription_id WHERE d.next_attempt <= ? AND d.lease_until <= ? ORDER BY d.id LIMIT 4", &[DbValue::Int(now), DbValue::Int(now)]).await?;
            let mut deliveries = Vec::new();
            for row in rows.rows {
                let id = row.get_int(0)?;
                tx.execute("UPDATE push_deliveries SET lease_until = ?, attempts = attempts + 1 WHERE id = ?", &[DbValue::Int(now + 60), DbValue::Int(id)]).await?;
                deliveries.push(PushDelivery { id, subscription_id: row.get_int(1)?, user_id: UserId::new(row.get_int(2)?), subscription: serde_json::from_str(row.get_text(3)?).map_err(|error| StorageError::InvalidValue(error.to_string()))?, payload: row.get_text(4)?.into(), attempts: row.get_int(5)? + 1 });
            }
            Ok(deliveries)
        }).await
    }
    pub async fn finish_push_delivery(
        &self,
        delivery: &PushDelivery,
        status: Option<u16>,
        now: i64,
    ) -> Result<(), StorageError> {
        if matches!(status, Some(404 | 410)) {
            self.db
                .execute(
                    "DELETE FROM push_subscriptions WHERE id = ? AND user_id = ?",
                    &[
                        DbValue::Int(delivery.subscription_id),
                        DbValue::Int(delivery.user_id.get()),
                    ],
                )
                .await?;
        } else if status.is_some_and(|status| {
            (200..300).contains(&status)
                || (400..500).contains(&status) && status != 408 && status != 429
        }) {
            self.db
                .execute(
                    "DELETE FROM push_deliveries WHERE id = ?",
                    &[DbValue::Int(delivery.id)],
                )
                .await?;
        } else {
            let delay = (5i64 * (1 << delivery.attempts.min(6))).min(300);
            self.db
                .execute(
                    "UPDATE push_deliveries SET lease_until = 0, next_attempt = ? WHERE id = ?",
                    &[DbValue::Int(now + delay), DbValue::Int(delivery.id)],
                )
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use futures::executor::block_on;
    fn subscription() -> PushSubscription {
        let mut key = [1u8; 65];
        key[0] = 4;
        PushSubscription {
            endpoint: "https://fcm.googleapis.com/device".into(),
            keys: openwebide_core::push::PushKeys {
                p256dh: URL_SAFE_NO_PAD.encode(key),
                auth: URL_SAFE_NO_PAD.encode([2; 16]),
            },
        }
    }
    #[test]
    fn push_queue_contract_covers_modes_dedup_retries_and_ownership() {
        block_on(async {
            let store = Store::new(RusqliteDb::open_in_memory().unwrap());
            store.migrate().await.unwrap();
            let alice = store
                .insert_user("alice", "hash", UserRole::Admin, 1)
                .await
                .unwrap()
                .id;
            let bob = store
                .insert_user("bob", "hash", UserRole::User, 1)
                .await
                .unwrap()
                .id;
            let subscription = subscription();
            store
                .save_push_subscription(alice, &subscription, 1)
                .await
                .unwrap();
            store
                .set_user_setting(alice, "browser_notifications", "true")
                .await
                .unwrap();
            for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
                let project = store
                    .create_project(
                        &NewProject {
                            name: "Project".into(),
                            mode,
                            path: Some("p".into()),
                        },
                        alice,
                        1,
                    )
                    .await
                    .unwrap();
                let session = store
                    .create_session("Session", None, None, Some(project.id), alice, 1)
                    .await
                    .unwrap();
                let message = store
                    .insert_message(session.id, Role::Assistant, "done", 2)
                    .await
                    .unwrap();
                let event = RunNotification::Finished {
                    message_id: message.id,
                };
                assert!(
                    store
                        .queue_run_notification(bob, session.id, &event, 3)
                        .await
                        .is_err()
                );
                store
                    .queue_run_notification(alice, session.id, &event, 3)
                    .await
                    .unwrap();
                store
                    .queue_run_notification(alice, session.id, &event, 3)
                    .await
                    .unwrap();
                let deliveries = store.claim_push_deliveries(3).await.unwrap();
                assert_eq!(deliveries.len(), 1);
                {
                    let payload: openwebide_core::push::PushPayload =
                        serde_json::from_str(&deliveries[0].payload).unwrap();
                    assert_eq!(payload.title, "Run finished · Project");
                    assert_eq!(payload.body, "Session — done");
                    assert!(store.claim_push_deliveries(4).await.unwrap().is_empty());
                    store
                        .finish_push_delivery(&deliveries[0], Some(503), 3)
                        .await
                        .unwrap();
                    assert!(store.claim_push_deliveries(12).await.unwrap().is_empty());
                    let retry = store.claim_push_deliveries(13).await.unwrap();
                    assert_eq!(retry.len(), 1);
                    assert_eq!(retry[0].attempts, 2);
                    store
                        .finish_push_delivery(&retry[0], Some(201), 13)
                        .await
                        .unwrap();
                    assert!(store.claim_push_deliveries(74).await.unwrap().is_empty());
                }
            }
            let session = store
                .create_session("Projectless", None, None, None, alice, 1)
                .await
                .unwrap();
            let message = store
                .insert_message(session.id, Role::Assistant, "done", 2)
                .await
                .unwrap();
            store
                .queue_run_notification(
                    alice,
                    session.id,
                    &RunNotification::Finished {
                        message_id: message.id,
                    },
                    100,
                )
                .await
                .unwrap();
            store
                .save_push_subscription(bob, &subscription, 101)
                .await
                .unwrap();
            assert!(
                !store
                    .has_push_subscription(alice, &subscription.endpoint)
                    .await
                    .unwrap()
            );
            assert!(store.claim_push_deliveries(101).await.unwrap().is_empty());
            store
                .remove_push_subscription(alice, &subscription.endpoint)
                .await
                .unwrap();
            assert!(
                store
                    .has_push_subscription(bob, &subscription.endpoint)
                    .await
                    .unwrap()
            );
        });
    }
    #[test]
    fn push_approval_decisions_opt_out_expiry_and_gone_endpoints() {
        block_on(async {
            let store = Store::new(RusqliteDb::open_in_memory().unwrap());
            store.migrate().await.unwrap();
            let user = store
                .insert_user("alice", "hash", UserRole::Admin, 1)
                .await
                .unwrap()
                .id;
            let subscription = subscription();
            store
                .save_push_subscription(user, &subscription, 1)
                .await
                .unwrap();
            store
                .set_user_setting(user, "browser_notifications", "true")
                .await
                .unwrap();
            let session = store
                .create_session("Session", None, None, None, user, 1)
                .await
                .unwrap();
            let anchor = store
                .insert_message(session.id, Role::Assistant, "tool", 2)
                .await
                .unwrap();
            store
                .upsert_tool_step(
                    session.id,
                    anchor.id,
                    "call",
                    "run_command",
                    "command",
                    2,
                    None,
                )
                .await
                .unwrap();
            store
                .queue_run_notification(
                    user,
                    session.id,
                    &RunNotification::Approval {
                        tool_call_id: "call".into(),
                    },
                    3,
                )
                .await
                .unwrap();
            store
                .complete_tool_step(user, session.id, "call", true, "done", None)
                .await
                .unwrap();
            assert!(store.claim_push_deliveries(4).await.unwrap().is_empty());
            for (now, status) in [(10, None), (4000, Some(410))] {
                let message = store
                    .insert_message(session.id, Role::Assistant, "done", now)
                    .await
                    .unwrap();
                store
                    .queue_run_notification(
                        user,
                        session.id,
                        &RunNotification::Finished {
                            message_id: message.id,
                        },
                        now,
                    )
                    .await
                    .unwrap();
                if let Some(status) = status {
                    let delivery = store.claim_push_deliveries(now).await.unwrap().remove(0);
                    store
                        .finish_push_delivery(&delivery, Some(status), now)
                        .await
                        .unwrap();
                    assert!(
                        !store
                            .has_push_subscription(user, &subscription.endpoint)
                            .await
                            .unwrap()
                    );
                } else {
                    assert!(
                        store
                            .claim_push_deliveries(now + 3600)
                            .await
                            .unwrap()
                            .is_empty()
                    );
                }
            }
            store
                .save_push_subscription(user, &subscription, 5000)
                .await
                .unwrap();
            let message = store
                .insert_message(session.id, Role::Assistant, "done", 5000)
                .await
                .unwrap();
            store
                .queue_run_notification(
                    user,
                    session.id,
                    &RunNotification::Finished {
                        message_id: message.id,
                    },
                    5000,
                )
                .await
                .unwrap();
            store
                .set_user_setting(user, "browser_notifications", "false")
                .await
                .unwrap();
            assert!(store.claim_push_deliveries(5001).await.unwrap().is_empty());
        });
    }
}
