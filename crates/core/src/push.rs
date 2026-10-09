//! Web Push contracts and notification shaping, independent of run and HTTP hosts.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushSubscription {
    pub endpoint: String,
    pub keys: PushKeys,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}
impl PushSubscription {
    pub fn validate(&self) -> Result<(), String> {
        if self.endpoint.len() > 4096 {
            return Err("Push endpoint is too long".into());
        }
        let url = url::Url::parse(&self.endpoint).map_err(|_| "Invalid push endpoint")?;
        let host = url.host_str().unwrap_or_default();
        let provider = host == "fcm.googleapis.com"
            || host == "updates.push.services.mozilla.com"
            || host.ends_with(".push.services.mozilla.com")
            || host == "web.push.apple.com"
            || host.ends_with(".push.apple.com")
            || host.ends_with(".notify.windows.com");
        if url.scheme() != "https"
            || !provider
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.port().is_some_and(|port| port != 443)
        {
            return Err("Use a supported browser's HTTPS push endpoint".into());
        }
        let public = URL_SAFE_NO_PAD
            .decode(&self.keys.p256dh)
            .map_err(|_| "Invalid push public key")?;
        let auth = URL_SAFE_NO_PAD
            .decode(&self.keys.auth)
            .map_err(|_| "Invalid push authentication key")?;
        if public.len() != 65 || public[0] != 4 || auth.len() != 16 {
            return Err("Invalid push subscription keys".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunNotification {
    Finished { message_id: i64 },
    Approval { tool_call_id: String },
}
impl RunNotification {
    pub fn from_event(event: &crate::RunEvent) -> Option<Self> {
        match event {
            crate::RunEvent::Done { message } => Some(Self::Finished {
                message_id: message.id,
            }),
            crate::RunEvent::PermissionRequest { id, .. } => Some(Self::Approval {
                tool_call_id: id.clone(),
            }),
            crate::RunEvent::Task { update } => match &update.event {
                crate::TaskEvent::Run { event }
                    if !matches!(**event, crate::RunEvent::Done { .. }) =>
                {
                    Self::from_event(event)
                }
                _ => None,
            },
            _ => None,
        }
    }
    pub fn key(&self) -> String {
        match self {
            Self::Finished { message_id } => format!("done:{message_id}"),
            Self::Approval { tool_call_id } => format!("permission:{tool_call_id}"),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Finished { message_id } if *message_id > 0 => Ok(()),
            Self::Approval { tool_call_id }
                if !tool_call_id.is_empty() && tool_call_id.len() <= 256 =>
            {
                Ok(())
            }
            _ => Err("Invalid notification event".into()),
        }
    }
    pub fn payload(
        &self,
        user: crate::UserId,
        session: i64,
        project: Option<&str>,
        session_name: &str,
    ) -> PushPayload {
        let title = match self {
            Self::Finished { .. } => "Run finished",
            Self::Approval { .. } => "Approval needed",
        };
        let title = project.map_or_else(
            || title.to_string(),
            |project| format!("{title} · {}", project.chars().take(80).collect::<String>()),
        );
        let body = session_name.chars().take(160).collect::<String>();
        PushPayload {
            title,
            body,
            tag: format!("openwebide-{session}-{}", self.key()),
            user_id: user,
            session_id: session,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushPayload {
    pub title: String,
    pub body: String,
    pub tag: String,
    pub user_id: crate::UserId,
    pub session_id: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushConfig {
    pub public_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushEndpoint {
    pub endpoint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushStatus {
    pub subscribed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscriptions_only_target_push_providers_with_valid_keys() {
        let mut bytes = [0u8; 65];
        bytes[0] = 4;
        let mut subscription = PushSubscription {
            endpoint: "https://web.push.apple.com/device".into(),
            keys: PushKeys {
                p256dh: URL_SAFE_NO_PAD.encode(bytes),
                auth: URL_SAFE_NO_PAD.encode([1; 16]),
            },
        };
        assert!(subscription.validate().is_ok());
        for endpoint in [
            "https://127.0.0.1/device",
            "http://fcm.googleapis.com/device",
            "https://fcm.googleapis.com.evil.test/device",
            "https://fcm.googleapis.com:123/device",
            "https://evil@web.push.apple.com/device",
        ] {
            subscription.endpoint = endpoint.into();
            assert!(subscription.validate().is_err());
        }
        subscription.endpoint = "https://fcm.googleapis.com/device".into();
        subscription.keys.auth = "bad".into();
        assert!(subscription.validate().is_err());
    }
}
