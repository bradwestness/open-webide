//! Shared Web Push delivery facade; Spin supplies only outbound HTTP.
use crate::error::ApiError;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use openwebide_core::push::PushSubscription;
use openwebide_storage::{Store, db::Db};
use web_push_native::{
    Auth, WebPushBuilder, jwt_simple::algorithms::ES256KeyPair, p256::PublicKey,
};

const KEY: &str = "web_push_private_key";
const DEFAULT_CONTACT: &str = "mailto:webpush@openwebide.invalid";

pub async fn key_pair<D: Db>(store: &Store<D>) -> Result<ES256KeyPair, ApiError> {
    if store.get_setting(KEY).await?.is_none() {
        let key = ES256KeyPair::generate();
        store
            .insert_setting_if_absent(KEY, &URL_SAFE_NO_PAD.encode(key.to_bytes()))
            .await?;
    }
    let encoded = store
        .get_setting(KEY)
        .await?
        .ok_or_else(|| ApiError::internal("Web Push key missing"))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| ApiError::internal("Invalid Web Push key encoding"))?;
    ES256KeyPair::from_bytes(&bytes).map_err(|_| ApiError::internal("Invalid Web Push key"))
}
pub fn public_key(key: &ES256KeyPair) -> String {
    {
        use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;
        let public = PublicKey::from_sec1_bytes(&key.public_key().to_bytes())
            .expect("generated VAPID key is valid");
        URL_SAFE_NO_PAD.encode(public.to_encoded_point(false).as_bytes())
    }
}
async fn contact() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        spin_sdk::variables::get("web_push_contact")
            .await
            .unwrap_or_else(|_| DEFAULT_CONTACT.into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        DEFAULT_CONTACT.into()
    }
}
pub fn subscription_key(subscription: &PushSubscription) -> Result<PublicKey, ApiError> {
    subscription.validate().map_err(ApiError::bad_request)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(&subscription.keys.p256dh)
        .map_err(|_| ApiError::bad_request("Invalid subscription key"))?;
    PublicKey::from_sec1_bytes(&bytes)
        .map_err(|_| ApiError::bad_request("Invalid subscription public key"))
}
pub fn request(
    key: &ES256KeyPair,
    subscription: &PushSubscription,
    payload: &str,
    contact: &str,
) -> Result<spin_sdk::http::Request<Vec<u8>>, ApiError> {
    let public = subscription_key(subscription)?;
    let auth = URL_SAFE_NO_PAD
        .decode(&subscription.keys.auth)
        .map_err(|_| ApiError::bad_request("Invalid subscription authentication key"))?;
    let endpoint = subscription
        .endpoint
        .parse()
        .map_err(|_| ApiError::bad_request("Invalid push endpoint"))?;
    WebPushBuilder::new(endpoint, public, Auth::clone_from_slice(&auth))
        .with_valid_duration(std::time::Duration::from_secs(300))
        .with_vapid(key, contact)
        .build(payload.as_bytes().to_vec())
        .map_err(|_| ApiError::internal("Could not encrypt push notification"))
}

pub trait PushTransport: Send + Sync {
    fn send(
        &self,
        request: spin_sdk::http::Request<Vec<u8>>,
    ) -> impl std::future::Future<Output = Result<u16, String>> + Send;
}
pub struct SpinPushTransport;
impl PushTransport for SpinPushTransport {
    async fn send(&self, request: spin_sdk::http::Request<Vec<u8>>) -> Result<u16, String> {
        #[cfg(target_arch = "wasm32")]
        {
            let request =
                request.map(|bytes| spin_sdk::http::FullBody::new(bytes::Bytes::from(bytes)));
            let send = spin_sdk::http::send(request);
            let timeout = spin_sdk::time::sleep(std::time::Duration::from_secs(8));
            futures::pin_mut!(send, timeout);
            match futures::future::select(send, timeout).await {
                futures::future::Either::Left((result, _)) => result
                    .map(|response: spin_sdk::http::Response| response.status().as_u16())
                    .map_err(|_| "Push service could not be reached".into()),
                futures::future::Either::Right(_) => Err("Push delivery timed out".into()),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = request;
            Err("Push HTTP requires a WASI host".into())
        }
    }
}
pub async fn dispatch<D: Db>(
    store: &Store<D>,
    transport: &impl PushTransport,
    now: i64,
) -> Result<usize, ApiError> {
    let deliveries = store.claim_push_deliveries(now).await?;
    if deliveries.is_empty() {
        return Ok(0);
    }
    let key = key_pair(store).await?;
    let contact = contact().await;
    for delivery in &deliveries {
        let status = match request(&key, &delivery.subscription, &delivery.payload, &contact) {
            Ok(request) => transport.send(request).await.ok(),
            Err(_) => Some(400),
        };
        // Endpoint tokens and subscription keys must never appear in logs.
        if !status.is_some_and(|status| (200..300).contains(&status)) {
            eprintln!("Web Push delivery {} returned {:?}", delivery.id, status);
        }
        store.finish_push_delivery(delivery, status, now).await?;
    }
    Ok(deliveries.len())
}
