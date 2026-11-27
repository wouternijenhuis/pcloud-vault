//! pCloud's documented public-client token flow. The loopback page relays the
//! URL fragment in a POST; credentials never appear in an HTTP request URL.

use crate::{Account, Client, Error, Region, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use rand::{rngs::SysRng, TryRng};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};
use url::Url;
use zeroize::Zeroizing;

pub const CALLBACK_URL: &str = "http://127.0.0.1:43187/callback";
const CALLBACK_ORIGIN: &str = "http://127.0.0.1:43187";
const CALLBACK_HOST: &str = "127.0.0.1:43187";
const MAX_FRAGMENT: usize = 8192;
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone)]
struct CallbackState {
    expected_state: String,
    nonce: String,
    completed: Arc<Mutex<Option<oneshot::Sender<Result<Account>>>>>,
}

pub fn login(client_id: &str) -> Result<Account> {
    if client_id.trim().is_empty()
        || client_id.len() > 256
        || client_id.chars().any(char::is_control)
    {
        return Err(Error::MissingClientId);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| Error::CallbackUnavailable)?;
    let mut account = runtime.block_on(login_async(client_id))?;
    let user = Client::new(account.clone())?.user_info()?;
    if user.user_id != account.user_id {
        return Err(Error::InvalidLogin);
    }
    account.email = user.email;
    Ok(account)
}

async fn login_async(client_id: &str) -> Result<Account> {
    let listener = TcpListener::bind(CALLBACK_HOST)
        .await
        .map_err(|_| Error::CallbackUnavailable)?;
    let (send, receive) = oneshot::channel();
    let state = CallbackState {
        expected_state: random_hex(),
        nonce: random_hex(),
        completed: Arc::new(Mutex::new(Some(send))),
    };
    let url = authorization_url(client_id, &state.expected_state)?;
    let router = Router::new()
        .route("/callback", get(callback))
        .route("/complete", post(complete))
        .layer(DefaultBodyLimit::max(MAX_FRAGMENT))
        .with_state(state);
    let (shutdown_send, shutdown_receive) = oneshot::channel::<()>();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_receive.await;
            })
            .await
    });
    if open::that(url.as_str()).is_err() {
        server.abort();
        return Err(Error::Browser);
    }
    let result = tokio::time::timeout(LOGIN_TIMEOUT, receive).await;
    let _ = shutdown_send.send(());
    // Give the accepted completion response time to flush; no new connections
    // are accepted after graceful shutdown starts. Stalled clients are bounded.
    if tokio::time::timeout(Duration::from_secs(2), &mut server)
        .await
        .is_err()
    {
        server.abort();
    }
    match result {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(Error::InvalidLogin),
        Err(_) => Err(Error::LoginTimeout),
    }
}

fn authorization_url(client_id: &str, state: &str) -> Result<Url> {
    let mut url =
        Url::parse("https://my.pcloud.com/oauth2/authorize").map_err(|_| Error::InvalidLogin)?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "token")
        .append_pair("redirect_uri", CALLBACK_URL)
        .append_pair("state", state);
    Ok(url)
}

async fn callback(State(state): State<CallbackState>, headers: HeaderMap, uri: Uri) -> Response {
    if !valid_host(&headers) || uri.query().is_some() {
        return fixed_response(
            StatusCode::BAD_REQUEST,
            "Invalid callback.",
            "text/plain; charset=utf-8",
        );
    }
    let html = include_str!("oauth_callback.html").replace("SCRIPT_NONCE", &state.nonce);
    let mut response = fixed_response(StatusCode::OK, &html, "text/html; charset=utf-8");
    let policy = format!("default-src 'none'; script-src 'nonce-{}'; connect-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",state.nonce);
    if let Ok(header) = policy.parse() {
        response
            .headers_mut()
            .insert("Content-Security-Policy", header);
    }
    response
}

async fn complete(State(state): State<CallbackState>, headers: HeaderMap, body: Bytes) -> Response {
    if !valid_host(&headers)
        || headers.get("origin").and_then(|v| v.to_str().ok()) != Some(CALLBACK_ORIGIN)
        || !headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|value| value.starts_with("text/plain"))
    {
        return fixed_response(
            StatusCode::FORBIDDEN,
            "Invalid callback origin.",
            "text/plain",
        );
    }
    let text = match std::str::from_utf8(&body) {
        Ok(value) => value,
        Err(_) => {
            return fixed_response(StatusCode::BAD_REQUEST, "Invalid callback.", "text/plain")
        }
    };
    let result = parse_fragment(text, &state.expected_state);
    if matches!(result, Err(Error::InvalidLogin)) {
        return fixed_response(StatusCode::BAD_REQUEST, "Invalid callback.", "text/plain");
    }
    let Some(sender) = state.completed.lock().ok().and_then(|mut slot| slot.take()) else {
        return fixed_response(StatusCode::GONE, "Sign-in already completed.", "text/plain");
    };
    let success = result.is_ok();
    let _ = sender.send(result);
    fixed_response(
        if success {
            StatusCode::OK
        } else {
            StatusCode::UNAUTHORIZED
        },
        if success {
            "Connected. You can close this tab."
        } else {
            "Sign-in declined."
        },
        "text/plain; charset=utf-8",
    )
}

fn fixed_response(status: StatusCode, body: &str, content_type: &'static str) -> Response {
    let mut response = (status, body.to_owned()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        "Content-Type",
        content_type.parse().expect("static content type"),
    );
    headers.insert(
        "Cache-Control",
        "no-store, max-age=0".parse().expect("static cache header"),
    );
    headers.insert("Pragma", "no-cache".parse().expect("static pragma"));
    headers.insert(
        "Referrer-Policy",
        "no-referrer".parse().expect("static referrer policy"),
    );
    headers.insert(
        "X-Content-Type-Options",
        "nosniff".parse().expect("static nosniff"),
    );
    response
}

fn valid_host(headers: &HeaderMap) -> bool {
    headers.get("host").and_then(|value| value.to_str().ok()) == Some(CALLBACK_HOST)
}
fn random_hex() -> String {
    let mut random = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut random)
        .expect("the operating system random generator is available");
    hex::encode(random)
}

fn parse_fragment(fragment: &str, expected_state: &str) -> Result<Account> {
    if fragment.is_empty() || fragment.len() > MAX_FRAGMENT || fragment.starts_with('#') {
        return Err(Error::InvalidLogin);
    }
    let mut parameters: Vec<(String, Zeroizing<String>)> = Vec::new();
    for (name, value) in url::form_urlencoded::parse(fragment.as_bytes()) {
        if parameters.iter().any(|(key, _)| key == &name) {
            return Err(Error::InvalidLogin);
        }
        parameters.push((name.into_owned(), Zeroizing::new(value.into_owned())));
    }
    let parameter = |name: &str| -> Result<&str> {
        parameters
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .ok_or(Error::InvalidLogin)
    };
    if !equal_state(parameter("state")?, expected_state) {
        return Err(Error::InvalidLogin);
    }
    if parameters.iter().any(|(key, _)| key == "error") {
        return Err(Error::LoginDeclined);
    }
    if parameter("token_type")? != "bearer" {
        return Err(Error::InvalidLogin);
    }
    let token = parameter("access_token")?;
    if token.is_empty() || token.len() > 4096 || token.chars().any(char::is_control) {
        return Err(Error::InvalidLogin);
    }
    let location = parameter("locationid")?
        .parse::<u64>()
        .map_err(|_| Error::InvalidLogin)?;
    let region = Region::from_oauth(location, parameter("hostname")?)?;
    let user_id = parameter("uid")?
        .parse::<u64>()
        .map_err(|_| Error::InvalidLogin)?;
    if user_id == 0 {
        return Err(Error::InvalidLogin);
    }
    Ok(Account {
        token: token.to_owned(),
        region,
        user_id,
        email: String::new(),
        auth: crate::AuthKind::OAuth,
    })
}

// State has a fixed random length; avoid exposing its matched prefix to a local requester.
fn equal_state(received: &str, expected: &str) -> bool {
    if received.len() != expected.len() {
        return false;
    }
    received
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |different, (a, b)| different | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> String {
        "access_token=secret&token_type=bearer&uid=42&state=one-time-state&locationid=2&hostname=eapi.pcloud.com".into()
    }
    #[test]
    fn valid_callback_and_authorize_url_are_exact() {
        let account = parse_fragment(&valid(), "one-time-state").unwrap();
        assert_eq!(account.user_id, 42);
        assert_eq!(account.region, Region::Eu);
        let url = authorization_url("client-123", "random").unwrap();
        assert_eq!(url.host_str(), Some("my.pcloud.com"));
        assert!(url
            .query_pairs()
            .any(|(key, value)| key == "redirect_uri" && value == CALLBACK_URL));
        assert!(!url.as_str().contains("client_secret"));
    }
    #[test]
    fn callback_rejects_state_mismatch_duplicate_and_host_injection() {
        assert!(parse_fragment(&valid(), "wrong").is_err());
        assert!(parse_fragment(&(valid() + "&state=one-time-state"), "one-time-state").is_err());
        assert!(parse_fragment(
            &valid().replace("eapi.pcloud.com", "eapi.pcloud.com.attacker.test"),
            "one-time-state"
        )
        .is_err());
        assert!(parse_fragment(
            &valid().replace("locationid=2", "locationid=1"),
            "one-time-state"
        )
        .is_err());
        assert!(parse_fragment(&valid().replace("uid=42", "uid=-1"), "one-time-state").is_err());
        assert!(parse_fragment(
            &valid().replace("access_token=secret", "access_token=%00"),
            "one-time-state"
        )
        .is_err());
        assert!(parse_fragment(&"x".repeat(MAX_FRAGMENT + 1), "one-time-state").is_err());
    }
    #[test]
    fn denial_requires_valid_state() {
        assert!(matches!(
            parse_fragment("error=access_denied&state=one-time-state", "one-time-state"),
            Err(Error::LoginDeclined)
        ));
        assert!(matches!(
            parse_fragment("error=access_denied&state=wrong", "one-time-state"),
            Err(Error::InvalidLogin)
        ));
    }
    #[test]
    fn loopback_host_is_not_suffix_matched() {
        let mut headers = HeaderMap::new();
        headers.insert("host", CALLBACK_HOST.parse().unwrap());
        assert!(valid_host(&headers));
        headers.insert("host", "127.0.0.1:43187.attacker.test".parse().unwrap());
        assert!(!valid_host(&headers));
    }
    #[tokio::test]
    async fn completion_is_one_time() {
        let (send, receive) = oneshot::channel();
        let state = CallbackState {
            expected_state: "one-time-state".into(),
            nonce: "nonce".into(),
            completed: Arc::new(Mutex::new(Some(send))),
        };
        let mut headers = HeaderMap::new();
        headers.insert("host", CALLBACK_HOST.parse().unwrap());
        headers.insert("origin", CALLBACK_ORIGIN.parse().unwrap());
        headers.insert("content-type", "text/plain".parse().unwrap());
        let first = complete(State(state.clone()), headers.clone(), Bytes::from(valid())).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert!(receive.await.unwrap().is_ok());
        assert_eq!(
            complete(State(state), headers, Bytes::from(valid()))
                .await
                .status(),
            StatusCode::GONE
        );
    }
}
