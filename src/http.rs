use std::{future::Future, time::Duration};

use anyhow::{Context, Result, bail};
use reqwest::{Request, StatusCode};
use serde::de::DeserializeOwned;

pub struct Reply {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

pub trait Transport {
    fn send(&self, request: Request) -> impl Future<Output = Result<Reply>>;
}

impl Transport for reqwest::Client {
    async fn send(&self, request: Request) -> Result<Reply> {
        let mut response = self.execute(request).await.map_err(|_| anyhow::anyhow!(
            "HTTP request failed; outcome may be unknown. Reconnect with `qbo auth login` if authorization expired"
        ))?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("HTTP response interrupted"))?
        {
            if body.len() + chunk.len() > 1024 * 1024 {
                bail!("HTTP response exceeds size limit");
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Reply { status, body })
    }
}

pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to initialize HTTP client")
}

pub fn parse<T: DeserializeOwned>(reply: Reply) -> Result<T> {
    if !reply.status.is_success() {
        if matches!(reply.status.as_u16(), 400 | 401 | 403 | 410) {
            bail!(
                "Authorization rejected ({}); reconnect with `qbo auth login`",
                reply.status
            );
        }
        bail!(
            "Endpoint returned {}; retry the operation explicitly if safe",
            reply.status
        );
    }
    // Do not attach serde errors: malformed upstream values may contain secrets.
    serde_json::from_slice(&reply.body).map_err(|_| anyhow::anyhow!("Invalid endpoint response"))
}

pub fn retry_delay(status: StatusCode, attempt: u32) -> Option<Duration> {
    if attempt < 3 && matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504) {
        Some(Duration::from_secs(1 << attempt))
    } else {
        None
    }
}

pub trait Sleeper {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()>;
}

pub struct TokioSleeper;
impl Sleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// Only repeatable GET requests may use this bounded retry policy.
pub async fn get<T: Transport, S: Sleeper>(
    transport: &T,
    sleeper: &S,
    request: Request,
) -> Result<Reply> {
    for attempt in 0..=3 {
        let reply = transport
            .send(request.try_clone().context("GET must be repeatable")?)
            .await?;
        match retry_delay(reply.status, attempt) {
            Some(delay) => sleeper.sleep(delay).await,
            None => return Ok(reply),
        }
    }
    unreachable!("bounded loop always returns on final attempt")
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;
    pub struct Fake {
        pub replies: RefCell<std::collections::VecDeque<Reply>>,
        pub requests: RefCell<Vec<Request>>,
    }
    impl Transport for Fake {
        async fn send(&self, request: Request) -> Result<Reply> {
            self.requests.borrow_mut().push(request);
            Ok(self
                .replies
                .borrow_mut()
                .pop_front()
                .expect("unexpected HTTP request"))
        }
    }
    #[derive(Default)]
    pub struct Clock(pub RefCell<Vec<Duration>>);
    impl Sleeper for Clock {
        async fn sleep(&self, duration: Duration) {
            self.0.borrow_mut().push(duration);
        }
    }
    #[tokio::test]
    async fn retries_only_transient_responses_with_a_bound() {
        let fake = Fake {
            requests: RefCell::default(),
            replies: RefCell::new(
                [429, 502, 503, 504]
                    .into_iter()
                    .map(|code| Reply {
                        status: StatusCode::from_u16(code).unwrap(),
                        body: vec![],
                    })
                    .collect(),
            ),
        };
        let clock = Clock::default();
        let reply = get(
            &fake,
            &clock,
            client()
                .unwrap()
                .get("https://example.invalid/query")
                .build()
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(reply.status, 504);
        assert_eq!(
            *clock.0.borrow(),
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4)
            ]
        );
        assert_eq!(fake.requests.borrow().len(), 4);
    }
    #[test]
    fn errors_do_not_expose_response_bodies() {
        let error = parse::<serde_json::Value>(Reply {
            status: StatusCode::UNAUTHORIZED,
            body: b"sensitive upstream data".to_vec(),
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("qbo auth login"));
        assert!(!error.contains("sensitive"));
        assert!(retry_delay(StatusCode::UNAUTHORIZED, 0).is_none());
    }
    #[test]
    fn rejected_queries_and_malformed_success_are_safe_errors() {
        for (status, body) in [
            (
                400,
                r#"{"Fault":{"Error":[{"code":"4000","Message":"QueryParserError","Detail":"private syntax"}]}}"#,
            ),
            (
                400,
                r#"{"Fault":{"Error":[{"code":"6000","Message":"ValidationFault","Detail":"private validation"}]}}"#,
            ),
            (200, "private invalid json"),
        ] {
            let error = parse::<serde_json::Value>(Reply {
                status: StatusCode::from_u16(status).unwrap(),
                body: body.as_bytes().to_vec(),
            })
            .unwrap_err()
            .to_string();
            assert!(!error.contains("private"));
            assert!(!error.is_empty());
        }
    }
}
