use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::time::sleep;
use url::{Url, form_urlencoded};
use uuid::Uuid;

use crate::{
    cli::LoginArgs,
    config::{AuthMode, QboConfig},
    http::{self, Sleeper, Transport},
    secrets,
};

const AUTHORIZATION_ENDPOINT: &str = "https://appcenter.intuit.com/connect/oauth2";
const TOKEN_ENDPOINT: &str = "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer";
const ACCOUNTING_SCOPE: &str = "com.intuit.quickbooks.accounting";

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

pub async fn login(args: LoginArgs) -> Result<()> {
    if args.broker_url.is_some() {
        return broker_login(args).await;
    }

    direct_login(args).await
}

async fn direct_login(args: LoginArgs) -> Result<()> {
    let client_id = args
        .client_id
        .as_deref()
        .context("direct login requires --client-id or QBO_CLIENT_ID")?;
    let client_secret = if args.client_secret_stdin {
        tokio::task::spawn_blocking(|| {
            let mut value = String::new();
            std::io::stdin().take(4097).read_to_string(&mut value)?;
            if value.len() > 4096 || value.trim().is_empty() {
                bail!("invalid client secret on stdin");
            }
            Ok::<_, anyhow::Error>(value.trim_end().to_owned())
        })
        .await
        .context("secret input task failed")??
    } else {
        secrets::get_client_secret()
            .context("provide --client-secret-stdin or store a client secret in Keychain")?
    };
    if !args.callback_bind.ip().is_loopback() {
        bail!("direct callback must bind a loopback address");
    }
    let state = Uuid::new_v4().to_string();
    let redirect = Url::parse(&args.redirect_uri).context("redirect URI must be a valid URL")?;
    if redirect.scheme() != "http"
        || !matches!(
            redirect.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        )
        || redirect.port_or_known_default() != Some(args.callback_bind.port())
        || redirect.path() != "/callback"
        || redirect.query().is_some()
        || redirect.fragment().is_some()
        || !redirect.username().is_empty()
        || redirect.password().is_some()
    {
        bail!("direct redirect URI must use the loopback callback port and /callback path");
    }
    let listener = TcpListener::bind(args.callback_bind)
        .with_context(|| format!("failed to listen on {}", args.callback_bind))?;

    let auth_url = authorization_url(client_id, &args.redirect_uri, &state)?;
    println!("Open this URL to authorize QBO access:\n{auth_url}\n");
    println!(
        "Listening for OAuth callback on http://{}.",
        args.callback_bind
    );

    if !args.no_open_browser {
        open::that(auth_url.as_str()).context("failed to open browser")?;
    }

    listener.set_nonblocking(true)?;
    let callback = tokio::time::timeout(Duration::from_secs(300), async {
        let mut outcome: Option<Result<OAuthCallback>> = None;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let expected_state = state.clone();
                    let result_page = outcome.as_ref().map(Result::is_ok);
                    let reply = tokio::task::spawn_blocking(move || {
                        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                        read_oauth_callback(stream, &expected_state, result_page)
                    })
                    .await
                    .context("callback task failed")??;
                    match reply {
                        BrowserReply::Callback(result) => {
                            if outcome.is_none() {
                                outcome = Some(result);
                            }
                        }
                        BrowserReply::ResultPage => {
                            return outcome.context("missing callback result")?;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    sleep(Duration::from_millis(100)).await
                }
                Err(_) => bail!("failed to accept OAuth callback"),
            }
        }
    })
    .await
    .context("OAuth callback expired; start login again")??;
    let token_response = exchange_code(
        client_id,
        &client_secret,
        &args.redirect_uri,
        &callback.code,
    )
    .await?;

    secrets::set_client_secret(&client_secret)?;
    secrets::set_access_token(&token_response.access_token)?;
    secrets::set_refresh_token(&token_response.refresh_token)?;

    let config = QboConfig {
        environment: args.environment,
        auth_mode: AuthMode::Direct,
        redirect_uri: args.redirect_uri,
        client_id: Some(client_id.to_string()),
        realm_id: Some(callback.realm_id),
        broker_url: None,
    };
    config.save()?;

    println!(
        "Connected to QBO realm. Access token expires in {} seconds.",
        token_response.expires_in
    );
    Ok(())
}

async fn broker_login(args: LoginArgs) -> Result<()> {
    let broker_url = normalize_broker_url(
        args.broker_url
            .as_deref()
            .context("broker login requires --broker-url")?,
    )?;
    let session = create_broker_session(&broker_url).await?;

    let authorization =
        Url::parse(&session.authorization_url).context("invalid authorization URL")?;
    if authorization.scheme() != "https"
        || authorization.host_str() != Some("appcenter.intuit.com")
        || !authorization.username().is_empty()
        || authorization.password().is_some()
    {
        bail!("unexpected authorization URL from broker");
    }
    println!(
        "Open this URL to authorize QBO access:\n{}\n",
        session.authorization_url
    );

    if !args.no_open_browser {
        open::that(session.authorization_url.as_str()).context("failed to open browser")?;
    }

    println!("Waiting for brokered OAuth callback...");
    let login = poll_broker_session(&broker_url, &session).await?;

    secrets::set_access_token(&login.access_token)?;
    secrets::set_refresh_token(&login.refresh_token)?;

    let config = QboConfig {
        environment: login.environment.unwrap_or(args.environment),
        auth_mode: AuthMode::Broker,
        redirect_uri: login.redirect_uri.unwrap_or_default(),
        client_id: login.client_id,
        realm_id: Some(login.realm_id),
        broker_url: Some(broker_url),
    };
    config.save()?;

    println!(
        "Connected to QBO realm through auth broker. Access token expires in {} seconds.",
        login.expires_in
    );
    Ok(())
}

pub async fn refresh_access_token(config: &QboConfig) -> Result<TokenResponse> {
    match config.auth_mode {
        AuthMode::Direct => {
            let client_id = config
                .client_id
                .as_deref()
                .context("missing client ID; run `qbo auth login` first")?;
            refresh_access_token_direct(client_id).await
        }
        AuthMode::Broker => refresh_access_token_broker(config).await,
    }
}

async fn refresh_access_token_direct(client_id: &str) -> Result<TokenResponse> {
    let client_secret = secrets::get_client_secret()
        .context("missing client secret; run `qbo auth login` again")?;
    let refresh_token = secrets::get_refresh_token()
        .context("missing refresh token; run `qbo auth login` first")?;

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
    ];

    post_token_request(client_id, &client_secret, &params).await
}

async fn refresh_access_token_broker(config: &QboConfig) -> Result<TokenResponse> {
    let broker_url = config
        .broker_url
        .as_deref()
        .context("missing broker URL; run `qbo auth login --broker-url ...` again")?;
    let refresh_token = secrets::get_refresh_token()
        .context("missing refresh token; run `qbo auth login` first")?;
    let transport = http::client()?;
    broker_token(&transport, broker_url, "refresh", &refresh_token).await
}

fn authorization_url(client_id: &str, redirect_uri: &str, state: &str) -> Result<Url> {
    let mut url = Url::parse(AUTHORIZATION_ENDPOINT)?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", ACCOUNTING_SCOPE)
        .append_pair("state", state);
    Ok(url)
}

async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code: &str,
) -> Result<TokenResponse> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ];

    post_token_request(client_id, client_secret, &params).await
}

async fn post_token_request(
    client_id: &str,
    client_secret: &str,
    params: &[(&str, &str)],
) -> Result<TokenResponse> {
    let transport = http::client()?;
    let request = transport
        .post(TOKEN_ENDPOINT)
        .basic_auth(client_id, Some(client_secret))
        .form(params)
        .build()?;
    http::parse(transport.send(request).await?)
}

#[derive(Deserialize)]
struct BrokerSession {
    session_id: String,
    poll_secret: String,
    authorization_url: String,
}

#[derive(Deserialize)]
struct BrokerLoginComplete {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    realm_id: String,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    environment: Option<crate::config::QboEnvironment>,
}

async fn broker_token<T: Transport, R: for<'de> Deserialize<'de>>(
    transport: &T,
    broker_url: &str,
    operation: &str,
    refresh_token: &str,
) -> Result<R> {
    let request = http::client()?
        .post(format!(
            "{}/tokens/{operation}",
            normalize_broker_url(broker_url)?
        ))
        .json(&serde_json::json!({"refresh_token": refresh_token}))
        .build()?;
    http::parse(transport.send(request).await?)
}

async fn create_broker_session(broker_url: &str) -> Result<BrokerSession> {
    create_session(&http::client()?, broker_url).await
}

async fn create_session<T: Transport>(transport: &T, broker_url: &str) -> Result<BrokerSession> {
    let request = http::client()?
        .post(format!("{}/sessions", normalize_broker_url(broker_url)?))
        .build()?;
    http::parse(transport.send(request).await?)
}

async fn poll_broker_session(
    broker_url: &str,
    session: &BrokerSession,
) -> Result<BrokerLoginComplete> {
    poll_session(&http::client()?, &http::TokioSleeper, broker_url, session).await
}

async fn poll_session<T: Transport, S: Sleeper>(
    transport: &T,
    sleeper: &S,
    broker_url: &str,
    session: &BrokerSession,
) -> Result<BrokerLoginComplete> {
    // Never allow a broker response to turn a session ID into a URL path or query.
    if session.session_id.len() != 32 || !session.session_id.bytes().all(|c| c.is_ascii_hexdigit())
    {
        bail!("invalid broker session ID");
    }
    let request = http::client()?
        .get(format!(
            "{}/sessions/{}",
            normalize_broker_url(broker_url)?,
            session.session_id
        ))
        .bearer_auth(&session.poll_secret)
        .build()?;
    for _ in 0..150 {
        // Delivery is at most once. Transport errors are never retried automatically.
        let reply = http::get(
            transport,
            sleeper,
            request.try_clone().context("invalid poll request")?,
        )
        .await?;
        if reply.status == StatusCode::ACCEPTED {
            sleeper.sleep(Duration::from_secs(2)).await;
        } else {
            return http::parse(reply);
        }
    }
    bail!("authorization session expired; reconnect with `qbo auth login`")
}

#[derive(Deserialize)]
struct Revoked {
    status: String,
}

async fn revoke_then_clear<T: Transport>(
    transport: &T,
    broker_url: &str,
    token: &str,
    clear: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let result: Revoked = broker_token(transport, broker_url, "revoke", token).await?;
    if result.status != "revoked" {
        bail!("Revocation not confirmed; local credentials retained");
    }
    clear()
}

pub async fn disconnect() -> Result<()> {
    let config = QboConfig::load()?;
    let token = secrets::get_refresh_token()
        .context("missing refresh token; disconnect in QuickBooks Connected Apps")?;
    let transport = http::client()?;
    let clear = || {
        secrets::delete_all()?;
        QboConfig::remove()
    };
    match config.auth_mode {
        AuthMode::Broker => {
            revoke_then_clear(
                &transport,
                config.broker_url.as_deref().context("missing broker URL")?,
                &token,
                clear,
            )
            .await?
        }
        AuthMode::Direct => {
            let request = transport
                .post("https://developer.api.intuit.com/v2/oauth2/tokens/revoke")
                .basic_auth(
                    config.client_id.as_deref().context("missing client ID")?,
                    Some(secrets::get_client_secret()?),
                )
                .json(&serde_json::json!({"token": token}))
                .build()?;
            let reply = transport.send(request).await?;
            if !reply.status.is_success() {
                bail!(
                    "Revocation failed; credentials retained. Retry or disconnect in QuickBooks Connected Apps"
                );
            }
            clear()?;
        }
    }
    println!("Disconnected from QuickBooks and removed local credentials.");
    Ok(())
}

fn normalize_broker_url(value: &str) -> Result<String> {
    let url = Url::parse(value).context("broker URL must be a valid URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("broker URL must use HTTPS");
    }

    Ok(value.trim_end_matches('/').to_string())
}

#[derive(Debug)]
struct OAuthCallback {
    code: String,
    realm_id: String,
}

enum BrowserReply {
    Callback(Result<OAuthCallback>),
    ResultPage,
}

fn read_oauth_callback(
    mut stream: impl Read + Write,
    expected_state: &str,
    result_page: Option<bool>,
) -> Result<BrowserReply> {
    let mut buffer = [0_u8; 8192];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let headers = "cache-control: no-store, no-cache\r\npragma: no-cache\r\nreferrer-policy: no-referrer\r\nx-content-type-options: nosniff\r\ncontent-security-policy: default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\nconnection: close\r\n";
    if method == "GET"
        && matches!(target, "/auth/success" | "/auth/failure")
        && result_page.is_some()
    {
        let body = if result_page == Some(true) {
            "Authorization received. Return to the CLI to finish connecting."
        } else {
            "Authorization failed. Start login again in the CLI."
        };
        write!(
            stream,
            "HTTP/1.1 200 OK\r\n{headers}content-type: text/plain\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )?;
        return Ok(BrowserReply::ResultPage);
    }
    let result = (|| {
        if method != "GET" || bytes_read == buffer.len() || result_page.is_some() {
            bail!("invalid or repeated OAuth callback");
        }
        let (path, query) = target
            .split_once('?')
            .context("missing OAuth callback query")?;
        if path != "/callback" {
            bail!("unexpected OAuth callback path");
        }
        let mut params = HashMap::new();
        for (key, value) in form_urlencoded::parse(query.as_bytes()).into_owned() {
            if params.insert(key, value).is_some() {
                bail!("duplicate OAuth callback parameter");
            }
        }
        parse_callback_params(params, expected_state)
    })();
    let location = if result.is_ok() {
        "/auth/success"
    } else {
        "/auth/failure"
    };
    write!(
        stream,
        "HTTP/1.1 302 Found\r\n{headers}location: {location}\r\ncontent-length: 0\r\n\r\n"
    )?;
    Ok(BrowserReply::Callback(result))
}

fn parse_callback_params(
    params: HashMap<String, String>,
    expected_state: &str,
) -> Result<OAuthCallback> {
    let state = params
        .get("state")
        .context("OAuth callback did not include state")?;

    if state != expected_state {
        bail!("OAuth state mismatch; refusing callback");
    }

    if params.contains_key("error") {
        bail!("Intuit authorization failed; start login again");
    }
    let code = params
        .get("code")
        .context("OAuth callback did not include code")?
        .to_owned();
    let realm_id = params
        .get("realmId")
        .context("OAuth callback did not include realmId")?
        .to_owned();

    if code.is_empty()
        || code.len() > 4096
        || realm_id.is_empty()
        || !realm_id.bytes().all(|c| c.is_ascii_digit())
    {
        bail!("invalid OAuth callback fields");
    }
    Ok(OAuthCallback { code, realm_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{
        Reply,
        tests::{Clock, Fake},
    };
    use std::cell::{Cell, RefCell};

    fn contracts() -> serde_json::Value {
        serde_json::from_str(include_str!("../contracts/broker-v2.json")).unwrap()
    }
    fn reply(case: &serde_json::Value) -> Reply {
        Reply {
            status: StatusCode::from_u16(case["response"]["status"].as_u64().unwrap() as u16)
                .unwrap(),
            body: serde_json::to_vec(&case["response"]["body"]).unwrap(),
        }
    }
    fn fake(case: &serde_json::Value) -> Fake {
        Fake {
            requests: RefCell::default(),
            replies: RefCell::new([reply(case)].into()),
        }
    }

    #[tokio::test]
    async fn consumer_verifies_shared_executable_contracts() {
        let contract = contracts();
        assert_eq!(contract["version"], "qbo-broker-v2");
        let cases = contract["cases"].as_array().unwrap();
        let delivery = cases.iter().find(|c| c["name"] == "delivery").unwrap();
        for case in cases {
            let transport = fake(case);
            let operation = case["operation"].as_str().unwrap();
            let expected = case["response"]["status"].as_u64().unwrap();
            let result = match operation {
                "create" => create_session(&transport, "https://broker.invalid")
                    .await
                    .map(|v| {
                        assert_eq!(v.session_id, case["response"]["body"]["session_id"]);
                        assert_eq!(v.poll_secret, case["response"]["body"]["poll_secret"]);
                        assert_eq!(
                            v.authorization_url,
                            case["response"]["body"]["authorization_url"]
                        );
                    }),
                "poll" => {
                    if expected == 202 {
                        transport.replies.borrow_mut().push_back(reply(delivery));
                    }
                    let header = case["request"]["headers"]["authorization"]
                        .as_str()
                        .unwrap();
                    let session = BrokerSession {
                        session_id: "a".repeat(32),
                        poll_secret: header.strip_prefix("Bearer ").unwrap().into(),
                        authorization_url: String::new(),
                    };
                    poll_session(
                        &transport,
                        &Clock::default(),
                        "https://broker.invalid",
                        &session,
                    )
                    .await
                    .map(|v| {
                        assert_eq!(v.access_token, "test-access-placeholder");
                        assert_eq!(v.refresh_token, "test-refresh-placeholder");
                        assert_eq!(v.expires_in, 3600);
                        assert_eq!(v.realm_id, "123");
                    })
                }
                "refresh" => broker_token::<_, TokenResponse>(
                    &transport,
                    "https://broker.invalid",
                    "refresh",
                    "test-refresh-placeholder",
                )
                .await
                .map(|v| {
                    assert_eq!(v.access_token, "test-access-placeholder");
                    assert_eq!(v.refresh_token, "test-refresh-placeholder");
                    assert_eq!(v.expires_in, 3600);
                }),
                "revoke" => {
                    let cleared = Cell::new(false);
                    let result = revoke_then_clear(
                        &transport,
                        "https://broker.invalid",
                        "test-refresh-placeholder",
                        || {
                            cleared.set(true);
                            Ok(())
                        },
                    )
                    .await;
                    assert_eq!(
                        cleared.get(),
                        expected == 200,
                        "credentials retained on failed revocation"
                    );
                    result
                }
                _ => {
                    // Legacy v1 and malformed requests are explicitly rejected by v2.
                    let spec = &case["request"];
                    let mut request = http::client().unwrap().request(
                        spec["method"].as_str().unwrap().parse().unwrap(),
                        format!("https://broker.invalid{}", spec["path"].as_str().unwrap()),
                    );
                    if let Some(query) = spec.get("query") {
                        request = request.query(query);
                    }
                    if let Some(body) = spec.get("body") {
                        request = request.json(body);
                    }
                    http::parse::<serde_json::Value>(
                        transport.send(request.build().unwrap()).await.unwrap(),
                    )
                    .map(|_| ())
                }
            };
            assert_eq!(result.is_ok(), expected < 400, "{}", case["name"]);
            let requests = transport.requests.borrow();
            let request = &requests[0];
            assert_eq!(request.method().as_str(), case["request"]["method"]);
            assert_eq!(request.url().path(), case["request"]["path"]);
            if operation == "poll" {
                assert_eq!(request.url().query(), None);
                assert_eq!(
                    request.headers()["authorization"].to_str().unwrap(),
                    case["request"]["headers"]["authorization"]
                );
            }
            if let Some(body) = case["request"].get("body") {
                let actual: serde_json::Value =
                    serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
                assert_eq!(&actual, body);
            }
            assert!(transport.replies.borrow().is_empty());
        }
    }

    #[tokio::test]
    async fn optional_v1_response_fields_may_be_absent_and_new_fields_ignored() {
        let mut case = contracts()["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "delivery")
            .unwrap()
            .clone();
        let body = case["response"]["body"].as_object_mut().unwrap();
        for field in ["client_id", "redirect_uri", "environment"] {
            body.remove(field);
        }
        body.insert("future_field".into(), serde_json::json!(true));
        let session = BrokerSession {
            session_id: "a".repeat(32),
            poll_secret: "test-poll-placeholder".into(),
            authorization_url: String::new(),
        };
        let result = poll_session(
            &fake(&case),
            &Clock::default(),
            "https://broker.invalid",
            &session,
        )
        .await
        .unwrap();
        assert!(result.client_id.is_none());
        assert!(result.redirect_uri.is_none());
        assert!(result.environment.is_none());
    }

    #[tokio::test]
    async fn pending_polling_has_a_fixed_bound_without_real_time() {
        let pending = contracts()["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "pending")
            .unwrap()
            .clone();
        let transport = fake(&pending);
        for _ in 1..150 {
            transport.replies.borrow_mut().push_back(reply(&pending));
        }
        let clock = Clock::default();
        let session = BrokerSession {
            session_id: "a".repeat(32),
            poll_secret: "test-poll-placeholder".into(),
            authorization_url: String::new(),
        };
        assert!(
            poll_session(&transport, &clock, "https://broker.invalid", &session)
                .await
                .is_err()
        );
        assert_eq!(transport.requests.borrow().len(), 150);
        assert_eq!(clock.0.borrow().len(), 150);
    }

    #[test]
    fn browser_callbacks_redirect_and_clean_pages_never_echo_parameters() {
        struct Stream {
            input: std::io::Cursor<Vec<u8>>,
            output: Vec<u8>,
        }
        impl Read for Stream {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.input.read(buffer)
            }
        }
        impl Write for Stream {
            fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
                self.output.extend_from_slice(buffer);
                Ok(buffer.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for target in [
            "/callback?state=expected&code=test-code-placeholder&realmId=123",
            "/callback?state=wrong&error=sensitive-placeholder",
            "/callback?state=expected&state=wrong",
        ] {
            let mut stream = Stream {
                input: std::io::Cursor::new(format!("GET {target} HTTP/1.1\r\n\r\n").into_bytes()),
                output: vec![],
            };
            assert!(matches!(
                read_oauth_callback(&mut stream, "expected", None).unwrap(),
                BrowserReply::Callback(_)
            ));
            let output = String::from_utf8(stream.output).unwrap();
            assert!(output.starts_with("HTTP/1.1 302"));
            assert!(output.contains("location: /auth/"));
            assert!(output.contains("no-store"));
            assert!(!output.contains("placeholder"));
        }
        let mut stream = Stream {
            input: std::io::Cursor::new(b"GET /auth/success HTTP/1.1\r\n\r\n".to_vec()),
            output: vec![],
        };
        assert!(matches!(
            read_oauth_callback(&mut stream, "expected", Some(true)).unwrap(),
            BrowserReply::ResultPage
        ));
        assert!(
            String::from_utf8(stream.output)
                .unwrap()
                .contains("Return to the CLI")
        );
    }

    #[test]
    fn callback_checks_state_even_when_provider_returns_an_error() {
        let valid: HashMap<String, String> = [
            ("state", "expected"),
            ("code", "test-code-placeholder"),
            ("realmId", "123"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        assert!(parse_callback_params(valid.clone(), "expected").is_ok());
        assert!(parse_callback_params(valid.clone(), "wrong").is_err());
        let mut denied = valid;
        denied.insert("error".into(), "denied".into());
        denied.insert("error_description".into(), "sensitive-placeholder".into());
        let error = parse_callback_params(denied, "expected")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("sensitive"));
        for value in [
            "http://broker.invalid",
            "https://user:secret@broker.invalid",
            "https://broker.invalid?token=x",
        ] {
            assert!(normalize_broker_url(value).is_err());
        }
    }
}
