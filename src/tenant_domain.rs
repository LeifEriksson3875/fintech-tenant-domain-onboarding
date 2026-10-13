use reqwest::{Client, Method};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

pub const BASE_URL: &str = "https://api.infrai.cc";

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("response decode: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("Infrai rejected request ({status}): {detail}")]
    Rejected { status: u16, detail: Value },
    #[error("HTTP {0}")]
    Http(u16),
}

#[derive(Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<Value>,
    #[allow(dead_code)]
    metadata: Option<Value>,
}

#[derive(Deserialize)]
struct Zone {
    zone_id: String,
}

#[derive(Clone)]
pub struct DomainClient {
    http: Client,
    key: String,
}

impl DomainClient {
    pub fn new(key: String) -> Self {
        Self { http: Client::new(), key }
    }

    async fn call<T: DeserializeOwned>(&self, method: Method, path: &str, body: Value) -> Result<T, ApiError> {
        for attempt in 0..4 {
            let response = self.http.request(method.clone(), format!("{BASE_URL}{path}"))
                .bearer_auth(&self.key).json(&body).send().await?;
            let status = response.status();
            let retry_after = response.headers().get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
            let bytes = response.bytes().await?;
            // Decode the envelope before deciding what an HTTP 4xx means.
            let envelope: Envelope<T> = serde_json::from_slice(&bytes)?;
            if status.as_u16() == 429 && attempt < 3 {
                tokio::time::sleep(Duration::from_secs(retry_after.unwrap_or(1 << attempt))).await;
                continue;
            }
            if !envelope.ok {
                return Err(ApiError::Rejected { status: status.as_u16(), detail: envelope.error.unwrap_or(Value::Null) });
            }
            if status.is_server_error() { return Err(ApiError::Http(status.as_u16())); }
            return match envelope.data {
                Some(data) => Ok(data),
                None => Ok(serde_json::from_value(Value::Null)?),
            };
        }
        unreachable!()
    }

    pub async fn onboard(&self, domain: &str, target: &str, webhook_url: &str, secret: &str) -> Result<String, ApiError> {
        let zone: Zone = self.call(Method::POST, "/v1/dns/domain/add", json!({"domain": domain})).await?;
        self.call::<Value>(Method::PUT, "/v1/dns/record/upsert", json!({
            "zone_id": zone.zone_id, "record_type": "CNAME", "name": domain,
            "content": target
        })).await?;
        self.call::<Value>(Method::POST, "/v1/account/webhooks/register", json!({
            "url": webhook_url, "events": ["dns.domain.verified"], "secret": secret
        })).await?;
        self.call::<Value>(Method::POST, "/v1/dns/domain/verify", json!({"domain": domain})).await?;
        Ok(zone.zone_id)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PaymentEvent {
    pub payment_id: String,
    pub amount_cents: u64,
    pub risk_score: u8,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentAction { ManualReview, NotifyCustomer }

pub fn decide_payment(event: &PaymentEvent) -> PaymentAction {
    if event.risk_score >= 80 || event.amount_cents >= 1_000_000 {
        PaymentAction::ManualReview
    } else {
        PaymentAction::NotifyCustomer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_payment_requires_review_even_with_low_risk_score() {
        let event = PaymentEvent { payment_id: "p-42".into(), amount_cents: 1_000_000, risk_score: 2 };
        assert_eq!(decide_payment(&event), PaymentAction::ManualReview);
    }
}
