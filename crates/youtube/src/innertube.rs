//! Native guest InnerTube transport for public catalog reads.
//!
//! Anonymous and fixed-origin: no cookies, no account headers, no redirects,
//! proxies or retries, bounded responses and a local 429 cooldown. Stream
//! unlocking stays with the supervised extractor; this never calls `player`.
use reqwest::Client;
use serde_json::{Value, json};
use serein_core::{OperationContext, ProviderError};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

const ORIGIN: &str = "https://www.youtube.com";
/// Public web responses are ~0.5 MiB; this leaves headroom while staying finite.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const CLIENT_NAME: &str = "WEB";
const CLIENT_ID: &str = "1";
const CLIENT_VERSION: &str = "2.20260623.01.00";
/// Read-only public endpoints. `player` is deliberately absent.
/// `navigation/resolve_url` maps a public `@handle` address to its canonical
/// `UC…` channel ID (read-only, anonymous, same fixed origin); the handle is
/// never followed afterwards.
const ENDPOINTS: &[&str] = &["search", "browse", "next", "navigation/resolve_url"];

#[cfg(test)]
#[derive(Default)]
pub struct Fixture {
    pub replies: Mutex<std::collections::VecDeque<Result<Value, ProviderError>>>,
    pub calls: Mutex<Vec<(String, Value)>>,
}

/// Blocking API: call from a worker thread, never the UI thread.
pub struct GuestTransport {
    client: Client,
    runtime: tokio::runtime::Runtime,
    cooldown: Mutex<Option<Instant>>,
    #[cfg(test)]
    pub fixture: Option<Fixture>,
}

impl GuestTransport {
    pub fn new() -> Result<Self, ProviderError> {
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .pool_max_idle_per_host(2)
            .user_agent("Serein/0.1 (experimental native YouTube client)")
            .build()
            .map_err(|_| ProviderError::Offline)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| ProviderError::Offline)?;
        Ok(Self {
            client,
            runtime,
            cooldown: Mutex::new(None),
            #[cfg(test)]
            fixture: None,
        })
    }

    #[cfg(test)]
    pub fn with_fixture(replies: Vec<Result<Value, ProviderError>>) -> Self {
        let mut transport = Self::new().unwrap();
        transport.fixture = Some(Fixture {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
        });
        transport
    }

    fn cooling_down(&self) -> bool {
        let mut cooldown = self.cooldown.lock().unwrap_or_else(|e| e.into_inner());
        match *cooldown {
            Some(until) if Instant::now() < until => true,
            Some(_) => {
                *cooldown = None;
                false
            }
            None => false,
        }
    }

    fn cool_down(&self) {
        *self.cooldown.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Instant::now() + Duration::from_secs(60));
    }

    /// POST one public InnerTube request. The caller supplies only the
    /// endpoint-specific body; the client context is always set here.
    pub fn post(
        &self,
        endpoint: &str,
        mut payload: Value,
        operation: &OperationContext,
    ) -> Result<Value, ProviderError> {
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if !ENDPOINTS.contains(&endpoint) || !payload.is_object() {
            return Err(ProviderError::InvalidInput);
        }
        if self.cooling_down() {
            return Err(ProviderError::RateLimited);
        }
        payload["context"] = json!({
            "client": {"clientName": CLIENT_NAME, "clientVersion": CLIENT_VERSION, "hl": "en", "gl": "US"},
            "user": {"lockedSafetyMode": false}
        });
        #[cfg(test)]
        if let Some(fixture) = &self.fixture {
            fixture
                .calls
                .lock()
                .unwrap()
                .push((endpoint.to_owned(), payload));
            let value = fixture
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("synthetic reply missing")?;
            validate(&value)?;
            return Ok(value);
        }
        let request = self
            .client
            .post(format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"))
            .header("Origin", ORIGIN)
            .header("X-Youtube-Client-Name", CLIENT_ID)
            .header("X-Youtube-Client-Version", CLIENT_VERSION)
            .json(&payload);
        let result = self.runtime.block_on(async {
            tokio::select! {
                result = async {
                    let mut response = request.send().await.map_err(network)?;
                    let status = response.status().as_u16();
                    if status == 429 {
                        self.cool_down();
                        return Err(ProviderError::RateLimited);
                    }
                    if status == 403 {
                        return Err(ProviderError::ProofRequired);
                    }
                    if status == 404 {
                        return Err(ProviderError::Unavailable);
                    }
                    if !(200..300).contains(&status) {
                        return Err(ProviderError::ExtractorFailed);
                    }
                    if response.content_length().is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) {
                        return Err(ProviderError::OutputTooLarge);
                    }
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await.map_err(network)? {
                        if operation.cancel.is_cancelled() {
                            return Err(ProviderError::Cancelled);
                        }
                        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                            return Err(ProviderError::OutputTooLarge);
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    let value: Value =
                        serde_json::from_slice(&bytes).map_err(|_| ProviderError::MalformedOutput)?;
                    validate(&value)?;
                    Ok(value)
                } => result,
                cancelled = async {
                    loop {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        if operation.cancel.is_cancelled() {
                            break ProviderError::Cancelled;
                        }
                    }
                } => Err(cancelled),
            }
        });
        if operation.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        result
    }
}

fn network(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Offline
    }
}

fn validate(value: &Value) -> Result<(), ProviderError> {
    if !value.is_object() {
        return Err(ProviderError::MalformedOutput);
    }
    if let Some(error) = value.get("error") {
        return Err(match error.get("status").and_then(Value::as_str) {
            Some("RESOURCE_EXHAUSTED") => ProviderError::RateLimited,
            Some("NOT_FOUND") => ProviderError::Unavailable,
            _ => ProviderError::ExtractorFailed,
        });
    }
    Ok(())
}

/// Plain text from an InnerTube `{simpleText}` or `{runs:[{text}]}` value, or
/// a view-model `{content}` string.
pub fn text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(text) = value.get("content").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    let runs = value.get("runs")?.as_array()?;
    let joined: String = runs
        .iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .collect();
    (!joined.is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation() -> OperationContext {
        OperationContext {
            request_id: 1,
            session_generation: 1,
            cancel: serein_core::CancellationToken::default(),
        }
    }

    #[test]
    fn only_public_read_endpoints_are_admitted_and_context_is_forced() {
        let transport = GuestTransport::with_fixture(vec![Ok(json!({"ok": true}))]);
        assert_eq!(
            transport.post("player", json!({}), &operation()),
            Err(ProviderError::InvalidInput)
        );
        assert_eq!(
            transport.post("search", json!("text"), &operation()),
            Err(ProviderError::InvalidInput)
        );
        for endpoint in [
            "navigation",
            "navigation/resolve_url/../player",
            "search?x=1",
        ] {
            assert_eq!(
                transport.post(endpoint, json!({}), &operation()),
                Err(ProviderError::InvalidInput)
            );
        }
        let payload =
            json!({"query": "TEST FIXTURE", "context": {"client": {"clientName": "ANDROID"}}});
        transport.post("search", payload, &operation()).unwrap();
        let calls = transport.fixture.as_ref().unwrap().calls.lock().unwrap();
        assert_eq!(calls[0].0, "search");
        assert_eq!(calls[0].1["context"]["client"]["clientName"], "WEB");
        assert!(calls[0].1.get("cookie").is_none());
    }

    #[test]
    fn error_bodies_and_cancellation_are_classified() {
        let transport = GuestTransport::with_fixture(vec![
            Ok(json!({"error": {"status": "RESOURCE_EXHAUSTED"}})),
            Ok(json!({"error": {"status": "NOT_FOUND"}})),
        ]);
        assert_eq!(
            transport.post("browse", json!({}), &operation()),
            Err(ProviderError::RateLimited)
        );
        assert_eq!(
            transport.post("next", json!({}), &operation()),
            Err(ProviderError::Unavailable)
        );
        let cancelled = operation();
        cancelled.cancel.cancel();
        assert_eq!(
            transport.post("search", json!({}), &cancelled),
            Err(ProviderError::Cancelled)
        );
    }

    #[test]
    fn cooldown_blocks_until_it_expires() {
        let transport = GuestTransport::with_fixture(vec![]);
        transport.cool_down();
        assert_eq!(
            transport.post("search", json!({}), &operation()),
            Err(ProviderError::RateLimited)
        );
    }

    #[test]
    fn text_reads_simple_runs_and_view_model_shapes() {
        assert_eq!(text(&json!({"simpleText": "a"})).as_deref(), Some("a"));
        assert_eq!(
            text(&json!({"runs": [{"text": "a"}, {"text": "b"}]})).as_deref(),
            Some("ab")
        );
        assert_eq!(text(&json!({"content": "c"})).as_deref(), Some("c"));
        assert_eq!(text(&json!({"runs": []})), None);
    }
}
