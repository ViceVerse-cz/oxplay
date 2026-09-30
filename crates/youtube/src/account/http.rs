//! Fixed-origin authenticated transport. Redirects, proxy inheritance, retries and logs are disabled.
use super::{AccountError, SessionControl, SessionCookies};
use oxplay_core::OperationContext;
use reqwest::{
    Client,
    header::{AUTHORIZATION, COOKIE, HeaderValue},
};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{
    cell::Cell,
    time::{Duration, Instant, SystemTime},
};
use zeroize::Zeroizing;
const ORIGIN: &str = "https://www.youtube.com";
const MAX_RESPONSE: usize = 8 * 1024 * 1024;
const ENDPOINTS: &[&str] = &[
    "account/accounts_list",
    "browse",
    "next",
    "subscription/subscribe",
    "subscription/unsubscribe",
    "like/like",
    "like/dislike",
    "like/removelike",
    "browse/edit_playlist",
];

#[cfg(test)]
pub(super) struct Fixture {
    pub replies: std::cell::RefCell<std::collections::VecDeque<Result<Value, AccountError>>>,
    pub calls: std::cell::RefCell<Vec<String>>,
}
#[derive(Clone, Copy)]
enum Cooldown {
    Until(Instant),
    Indefinite,
}
pub(super) struct Transport {
    cooldown: Cell<Option<Cooldown>>,
    client: Client,
    runtime: tokio::runtime::Runtime,
    #[cfg(test)]
    pub fixture: Option<Fixture>,
}
impl Transport {
    pub fn new() -> Result<Self, AccountError> {
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(0)
            .user_agent("Oxplay/0.1 (experimental native YouTube client)")
            .build()
            .map_err(|_| AccountError::Offline)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| AccountError::Offline)?;
        Ok(Self {
            cooldown: Cell::new(None),
            client,
            runtime,
            #[cfg(test)]
            fixture: None,
        })
    }
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        match self.cooldown.get() {
            Some(Cooldown::Until(deadline)) => deadline.checked_duration_since(Instant::now()),
            Some(Cooldown::Indefinite) => Some(Duration::MAX),
            None => None,
        }
    }
    pub(super) fn limit_requests(&self, header: Option<&str>) {
        let duration = retry_delay(header, SystemTime::now());
        self.cooldown.set(Some(
            Instant::now()
                .checked_add(duration)
                .map(Cooldown::Until)
                .unwrap_or(Cooldown::Indefinite),
        ));
    }
    #[allow(clippy::too_many_arguments)]
    pub fn post(
        &self,
        cookies: &SessionCookies,
        account_index: u8,
        endpoint: &str,
        mut payload: Value,
        tv: bool,
        control: &SessionControl,
        operation: &OperationContext,
    ) -> Result<Value, AccountError> {
        control.check(operation)?;
        if self.cooldown_remaining().is_some() {
            return Err(AccountError::RateLimited);
        }
        if !ENDPOINTS.contains(&endpoint) {
            return Err(AccountError::InvalidInput);
        }
        #[cfg(test)]
        if let Some(fixture) = &self.fixture {
            fixture.calls.borrow_mut().push(endpoint.to_owned());
            let value = fixture
                .replies
                .borrow_mut()
                .pop_front()
                .expect("synthetic reply missing")?;
            validate_response(&value)?;
            return Ok(value);
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| AccountError::InvalidInput)?
            .as_secs();
        let path = format!("/youtubei/v1/{endpoint}");
        let (cookie, sid) = cookies.request_values(&path, now)?;
        let authorization = sid_authorization(&sid, now);
        let mut cookie_header =
            HeaderValue::from_str(&cookie).map_err(|_| AccountError::InvalidCookieFile)?;
        cookie_header.set_sensitive(true);
        let mut auth_header =
            HeaderValue::from_str(&authorization).map_err(|_| AccountError::InvalidCookieFile)?;
        auth_header.set_sensitive(true);
        let (client_name, version, id) = if tv {
            ("TVHTML5", "7.20260311.12.00", "7")
        } else {
            ("WEB", "2.20260623.01.00", "1")
        };
        payload["context"] = json!({"client":{"clientName":client_name,"clientVersion":version,"hl":"en","gl":"US"},"user":{"lockedSafetyMode":false}});
        let request = self
            .client
            .post(format!("{ORIGIN}{path}?prettyPrint=false&alt=json"))
            .header(COOKIE, cookie_header)
            .header(AUTHORIZATION, auth_header)
            .header("Origin", ORIGIN)
            .header("X-Goog-Authuser", account_index.to_string())
            .header("X-Youtube-Client-Name", id)
            .header("X-Youtube-Client-Version", version)
            .json(&payload);
        let result=self.runtime.block_on(async{
            tokio::select! {
                result=async{
                    let mut response=request.send().await.map_err(map_network)?;
                    let status=response.status().as_u16();
                    if (300..400).contains(&status){return Err(AccountError::RedirectRejected)}
                    if status==401{return Err(AccountError::SessionExpired)}
                    if status==403{return Err(AccountError::ChallengeRequired)}
                    if status==429{
                        self.limit_requests(response.headers().get("retry-after").and_then(|h|h.to_str().ok()));
                        return Err(AccountError::RateLimited)
                    }
                    if status>=500{
                        if let Some(header)=response.headers().get("retry-after").and_then(|h|h.to_str().ok()){self.limit_requests(Some(header));}
                        return Err(AccountError::ServiceUnavailable)
                    }
                    if !(200..300).contains(&status){return Err(AccountError::RemoteRejected)}
                    if response.content_length().is_some_and(|n|n>MAX_RESPONSE as u64){return Err(AccountError::ResponseTooLarge)}
                    let mut bytes=Zeroizing::new(Vec::new());
                    while let Some(chunk)=response.chunk().await.map_err(map_network)?{
                        control.check(operation)?;
                        if bytes.len()+chunk.len()>MAX_RESPONSE{return Err(AccountError::ResponseTooLarge)}
                        bytes.extend_from_slice(&chunk);
                    }
                    let value:Value=serde_json::from_slice(&bytes).map_err(|_|AccountError::UnsupportedResponse)?;
                    control.check(operation)?;
                    validate_response(&value)?;
                    Ok(value)
                }=>result,
                error=async{loop{
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    if let Err(error)=control.check(operation){break error}
                }}=>Err(error),
            }
        });
        if matches!(result, Err(AccountError::RateLimited)) && self.cooldown_remaining().is_none() {
            self.limit_requests(None);
        }
        result
    }
}
fn retry_delay(header: Option<&str>, now: SystemTime) -> Duration {
    let minimum = Duration::from_secs(60);
    let Some(value) = header.map(str::trim) else {
        return minimum;
    };
    let delay = if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        Duration::from_secs(value.parse::<u64>().unwrap_or(u64::MAX))
    } else {
        httpdate::parse_http_date(value)
            .ok()
            .and_then(|deadline| deadline.duration_since(now).ok())
            .unwrap_or(minimum)
    };
    delay.max(minimum)
}
fn sid_authorization(sid: &str, now: u64) -> Zeroizing<String> {
    let input = Zeroizing::new(format!("{now} {sid} {ORIGIN}"));
    let digest = Sha1::digest(input.as_bytes());
    Zeroizing::new(format!("SAPISIDHASH {now}_{digest:x}"))
}
fn map_network(error: reqwest::Error) -> AccountError {
    if error.is_timeout() {
        AccountError::Timeout
    } else {
        AccountError::Offline
    }
}
fn validate_response(value: &Value) -> Result<(), AccountError> {
    if value
        .pointer("/responseContext/mainAppWebResponseContext/loggedOut")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(AccountError::SessionExpired);
    }
    if let Some(error) = value.get("error") {
        return Err(match error.get("status").and_then(Value::as_str) {
            Some("UNAUTHENTICATED") => AccountError::SessionExpired,
            Some("RESOURCE_EXHAUSTED") => AccountError::RateLimited,
            _ => AccountError::RemoteRejected,
        });
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_after_seconds_dates_and_local_cooldown_are_honored() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        assert_eq!(retry_delay(Some("120"), now), Duration::from_secs(120));
        let date = httpdate::fmt_http_date(now + Duration::from_secs(300));
        assert_eq!(retry_delay(Some(&date), now), Duration::from_secs(300));
        assert_eq!(retry_delay(Some("garbage"), now), Duration::from_secs(60));
        assert_eq!(
            retry_delay(Some("999999999999999999999999"), now),
            Duration::from_secs(u64::MAX)
        );
        let transport = Transport::new().unwrap();
        transport.limit_requests(Some("120"));
        assert!(transport.cooldown_remaining().unwrap() >= Duration::from_secs(119));
    }
    #[test]
    fn sid_hash_matches_independent_synthetic_vector() {
        assert_eq!(
            &*sid_authorization("synthetic", 123),
            "SAPISIDHASH 123_c2d039db44a56533d498c2c77c8d16b456dd82ef"
        );
    }
    #[test]
    fn guest_or_expired_response_is_never_account_success() {
        assert_eq!(
            validate_response(
                &json!({"responseContext":{"mainAppWebResponseContext":{"loggedOut":true}}})
            ),
            Err(AccountError::SessionExpired)
        );
        assert_eq!(
            validate_response(&json!({"error":{"status":"UNAUTHENTICATED"}})),
            Err(AccountError::SessionExpired)
        );
    }
}
