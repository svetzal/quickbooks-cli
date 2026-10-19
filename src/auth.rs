use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use url::{Url, form_urlencoded};
use uuid::Uuid;

use crate::{
    cli::LoginArgs,
    config::{AuthMode, QboConfig},
    secrets,
};

const AUTHORIZATION_ENDPOINT: &str = "https://appcenter.intuit.com/connect/oauth2";
const TOKEN_ENDPOINT: &str = "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer";
const ACCOUNTING_SCOPE: &str = "com.intuit.quickbooks.accounting";

#[derive(Debug, Deserialize)]
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
    let client_secret = args
        .client_secret
        .as_deref()
        .context("direct login requires --client-secret or QBO_CLIENT_SECRET")?;
    let state = Uuid::new_v4().to_string();
    Url::parse(&args.redirect_uri).context("redirect URI must be a valid URL")?;
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

    let callback = wait_for_oauth_callback(listener, &state)?;
    let token_response =
        exchange_code(client_id, client_secret, &args.redirect_uri, &callback.code).await?;

    secrets::set_client_secret(client_secret)?;
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
    let url = format!("{}/tokens/refresh", normalize_broker_url(broker_url)?);
    let response = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&RefreshRequest { refresh_token })
        .send()
        .await
        .context("failed to call QBO auth broker refresh endpoint")?;

    parse_json_response(response, "QBO auth broker refresh endpoint").await
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
    let response = reqwest::Client::new()
        .post(TOKEN_ENDPOINT)
        .basic_auth(client_id, Some(client_secret))
        .header(reqwest::header::ACCEPT, "application/json")
        .form(params)
        .send()
        .await
        .context("failed to call Intuit token endpoint")?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if status != StatusCode::OK {
        bail!("Intuit token endpoint returned {status}: {body}");
    }

    serde_json::from_str(&body).context("failed to parse Intuit token response")
}

#[derive(Debug, Deserialize)]
struct BrokerSession {
    session_id: String,
    poll_secret: String,
    authorization_url: String,
}

#[derive(Debug, Deserialize)]
struct BrokerLoginComplete {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    realm_id: String,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    environment: Option<crate::config::QboEnvironment>,
}

#[derive(Debug, Serialize)]
struct RefreshRequest {
    refresh_token: String,
}

async fn create_broker_session(broker_url: &str) -> Result<BrokerSession> {
    let url = format!("{broker_url}/sessions");
    let response = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("failed to call QBO auth broker session endpoint")?;

    parse_json_response(response, "QBO auth broker session endpoint").await
}

async fn poll_broker_session(
    broker_url: &str,
    session: &BrokerSession,
) -> Result<BrokerLoginComplete> {
    let url = format!("{}/sessions/{}", broker_url, session.session_id);
    let http = reqwest::Client::new();

    for _ in 0..150 {
        let response = http
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/json")
            .query(&[("poll_secret", session.poll_secret.as_str())])
            .send()
            .await
            .context("failed to poll QBO auth broker session endpoint")?;

        if response.status() == StatusCode::ACCEPTED {
            sleep(Duration::from_secs(2)).await;
            continue;
        }

        return parse_json_response(response, "QBO auth broker session endpoint").await;
    }

    bail!("timed out waiting for QBO auth broker callback")
}

async fn parse_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    endpoint_name: &str,
) -> Result<T> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        bail!("{endpoint_name} returned {status}: {body}");
    }

    serde_json::from_str(&body).with_context(|| format!("failed to parse {endpoint_name} response"))
}

fn normalize_broker_url(value: &str) -> Result<String> {
    let url = Url::parse(value).context("broker URL must be a valid URL")?;
    if url.scheme() != "https" && url.host_str() != Some("localhost") {
        bail!("broker URL must use HTTPS");
    }

    Ok(value.trim_end_matches('/').to_string())
}

#[derive(Debug)]
struct OAuthCallback {
    code: String,
    realm_id: String,
}

fn wait_for_oauth_callback(listener: TcpListener, expected_state: &str) -> Result<OAuthCallback> {
    println!("Waiting for OAuth callback...");

    let (mut stream, _) = listener
        .accept()
        .context("failed while waiting for OAuth callback")?;

    let mut buffer = [0_u8; 8192];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_line = request
        .lines()
        .next()
        .context("OAuth callback request was empty")?;

    let target = request_line
        .split_whitespace()
        .nth(1)
        .context("OAuth callback request target was missing")?;

    let query = target
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default();
    let params = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect::<HashMap<String, String>>();

    let result = parse_callback_params(params, expected_state);
    let (status_line, response_body) = match &result {
        Ok(_) => (
            "HTTP/1.1 200 OK",
            "QBO authorization complete. You can close this tab.",
        ),
        Err(_) => (
            "HTTP/1.1 400 Bad Request",
            "QBO authorization failed. Return to the terminal for details.",
        ),
    };

    write!(
        stream,
        "{status_line}\r\ncontent-type: text/plain\r\ncontent-length: {}\r\n\r\n{}",
        response_body.len(),
        response_body
    )?;

    result
}

fn parse_callback_params(
    params: HashMap<String, String>,
    expected_state: &str,
) -> Result<OAuthCallback> {
    if let Some(error) = params.get("error") {
        return Err(anyhow!(
            "Intuit returned OAuth error: {}",
            params.get("error_description").unwrap_or(error)
        ));
    }

    let state = params
        .get("state")
        .context("OAuth callback did not include state")?;

    if state != expected_state {
        bail!("OAuth state mismatch; refusing callback");
    }

    let code = params
        .get("code")
        .context("OAuth callback did not include code")?
        .to_owned();
    let realm_id = params
        .get("realmId")
        .context("OAuth callback did not include realmId")?
        .to_owned();

    Ok(OAuthCallback { code, realm_id })
}
