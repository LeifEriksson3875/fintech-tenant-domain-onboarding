mod tenant_domain;

use axum::{body::Bytes, extract::State, http::{HeaderMap, StatusCode}, routing::post, Json, Router};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{env, sync::Arc};
use tenant_domain::{decide_payment, ApiError, DomainClient, PaymentAction, PaymentEvent};

#[derive(Clone)]
struct Config {
    client: DomainClient,
    target: String,
    webhook_url: String,
    secret: String,
}

#[derive(Deserialize)]
struct Onboard { domain: String }

#[derive(Serialize)]
struct OnboardResult { domain: String, zone_id: String, status: &'static str }

async fn onboard(State(config): State<Arc<Config>>, Json(input): Json<Onboard>) -> Result<Json<OnboardResult>, (StatusCode, Json<Value>)> {
    let zone_id = config.client.onboard(&input.domain, &config.target, &config.webhook_url, &config.secret)
        .await.map_err(|err| match err {
            ApiError::Rejected { status, detail } => (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), Json(json!({"error": detail}))),
            other => (StatusCode::BAD_GATEWAY, Json(json!({"error": other.to_string()}))),
        })?;
    Ok(Json(OnboardResult { domain: input.domain, zone_id, status: "awaiting_verification" }))
}

async fn verified(State(config): State<Arc<Config>>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, StatusCode> {
    let signature = headers.get("x-infrai-signature").and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let signature = hex::decode(signature.strip_prefix("sha256=").unwrap_or(signature))
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(config.secret.as_bytes()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    mac.update(&body);
    mac.verify_slice(&signature).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let event: Value = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(json!({"received": event})))
}

async fn payment(Json(event): Json<PaymentEvent>) -> Json<Value> {
    let action: PaymentAction = decide_payment(&event);
    // Preserve the payment ID and decision for an audit-friendly notification sink.
    Json(json!({"payment_id": event.payment_id, "action": action}))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config {
        client: DomainClient::new(env::var("INFRAI_API_KEY")?),
        target: env::var("CNAME_TARGET")?,
        webhook_url: env::var("WEBHOOK_URL")?,
        secret: env::var("WEBHOOK_SECRET")?,
    });
    let app = Router::new().route("/domains", post(onboard))
        .route("/webhooks/domain", post(verified))
        .route("/payments/events", post(payment)).with_state(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
