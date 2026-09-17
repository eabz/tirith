//! Client for Jev, `TypeSafe` AI's evaluation model. Experimental
//! ([ADR-0024]).
//!
//! Jev does not generate text. It takes a JSON `state` and a map of named
//! questions (boolean, choice, or score) and returns one typed answer per
//! question, with probabilities, in a single round trip. That makes it a
//! fit for the small judgement calls a coordination server can make on an
//! agent's behalf: is this notice relevant to that task, which of these
//! tasks suits this agent, is this note a duplicate.
//!
//! [`Evaluator`] is the seam. [`crate::assist`] asks its questions through
//! it, tests plug in a scripted fake, and `JevClient` (cargo feature
//! `jev`) sends them to one of two [`Provider`]s:
//!
//! - [`Provider::TypeSafe`], the documented API at
//!   `POST https://api.typesafe.ai/v1/systemone`, key `TYPESAFE_API_KEY`.
//!   Boolean questions are called `noul` on this wire.
//! - [`Provider::Gateway`], the Vercel AI Gateway at
//!   `POST https://ai-gateway.vercel.sh/v4/ai/evaluation-model`, key
//!   `AI_GATEWAY_API_KEY`. Its format is not publicly documented; it is
//!   taken from the `@ai-sdk/gateway` source (evaluation specification 4).
//!
//! [`Request`], [`Answer`] and [`Response`] are provider-neutral; the
//! translation happens in [`Request::to_body`] and [`Response::parse`].
//!
//! [ADR-0024]: https://github.com/eabz/tirith/blob/main/docs/5-decisions/0024-jev-assist-experiment.md

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// The environment variable (or `.env` key) holding a `TypeSafe` API key.
pub const TYPESAFE_KEY_VAR: &str = "TYPESAFE_API_KEY";
/// The environment variable (or `.env` key) holding a Vercel AI Gateway key.
pub const GATEWAY_KEY_VAR: &str = "AI_GATEWAY_API_KEY";
/// How long one evaluation may take, retry included, before the caller
/// falls back.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// `TypeSafe`'s list price for Jev input tokens, per token (output is free).
/// The direct API reports no cost, so it is computed from usage.
const TYPESAFE_USD_PER_INPUT_TOKEN: f64 = 0.042 / 1_000_000.0;

/// Who Jev is reached through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// `TypeSafe` AI directly.
    TypeSafe,
    /// The Vercel AI Gateway.
    Gateway,
}

impl Provider {
    /// The evaluation endpoint.
    pub fn default_endpoint(self) -> &'static str {
        match self {
            Self::TypeSafe => "https://api.typesafe.ai/v1/systemone",
            Self::Gateway => "https://ai-gateway.vercel.sh/v4/ai/evaluation-model",
        }
    }

    /// The model id Jev is published under.
    pub fn default_model(self) -> &'static str {
        match self {
            Self::TypeSafe => "jev-latest",
            Self::Gateway => "typesafe-ai/jev",
        }
    }

    /// The variable its API key is read from.
    pub fn key_var(self) -> &'static str {
        match self {
            Self::TypeSafe => TYPESAFE_KEY_VAR,
            Self::Gateway => GATEWAY_KEY_VAR,
        }
    }

    /// Short name: `typesafe` or `gateway`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TypeSafe => "typesafe",
            Self::Gateway => "gateway",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where and how to reach Jev. The API key never appears in `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct JevConfig {
    provider: Provider,
    api_key: String,
    endpoint: String,
    model: String,
    timeout: Duration,
}

impl fmt::Debug for JevConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevConfig")
            .field("provider", &self.provider)
            .field("api_key", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl JevConfig {
    /// `provider`'s defaults with `api_key`.
    pub fn new(provider: Provider, api_key: impl Into<String>) -> Self {
        Self {
            provider,
            api_key: api_key.into(),
            endpoint: provider.default_endpoint().to_owned(),
            model: provider.default_model().to_owned(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Sends requests to `endpoint` instead of the provider's default.
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Asks `model` instead of the provider's default.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Gives up on an evaluation after `timeout`.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Who requests go to.
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// The endpoint requests go to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The model id sent with every request.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The per-evaluation timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Reads the configuration from the process environment, falling back
    /// to a `.env` file in `root` for anything the environment lacks.
    ///
    /// The provider is the one whose key is set, `TypeSafe` first because
    /// it is one hop closer; `TIRITH_JEV_PROVIDER` (`typesafe` or
    /// `gateway`) forces one. Returns `None` when the chosen provider has
    /// no key. Also read: `TIRITH_JEV_MODEL`, `TIRITH_JEV_ENDPOINT`,
    /// `TIRITH_JEV_TIMEOUT_MS`. Blocking: reads a file.
    pub fn from_env(root: &Path) -> Option<Self> {
        let dotenv = std::fs::read_to_string(root.join(".env")).unwrap_or_default();
        let lookup = |key: &str| {
            std::env::var(key)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| dotenv_value(&dotenv, key))
        };
        let keyed = |provider: Provider| lookup(provider.key_var()).map(|key| (provider, key));
        let (provider, key) = match lookup("TIRITH_JEV_PROVIDER").as_deref() {
            Some("typesafe") => keyed(Provider::TypeSafe)?,
            Some("gateway") => keyed(Provider::Gateway)?,
            _ => keyed(Provider::TypeSafe).or_else(|| keyed(Provider::Gateway))?,
        };
        let mut config = Self::new(provider, key);
        if let Some(model) = lookup("TIRITH_JEV_MODEL") {
            config = config.with_model(model);
        }
        if let Some(endpoint) = lookup("TIRITH_JEV_ENDPOINT") {
            config = config.with_endpoint(endpoint);
        }
        if let Some(ms) = lookup("TIRITH_JEV_TIMEOUT_MS").and_then(|v| v.parse().ok()) {
            config = config.with_timeout(Duration::from_millis(ms));
        }
        Some(config)
    }
}

/// The value of `key` in `.env` file contents: `KEY=value` lines, an
/// optional leading `export`, optional matching quotes, `#` comments.
fn dotenv_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value.trim();
        let unquoted = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q))
            .unwrap_or(value);
        (!unquoted.is_empty()).then(|| unquoted.to_owned())
    })
}

/// One question. Jev answers every question of a request in parallel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    /// Answered with P(true).
    Boolean {
        /// What to decide about the state.
        instructions: String,
    },
    /// Answered with one option name and a probability per option.
    Choice {
        /// What to decide about the state.
        instructions: String,
        /// Option name to an optional description. At most 255 options.
        criteria: BTreeMap<String, Option<String>>,
    },
    /// Answered with a fractional level and a probability per level.
    Score {
        /// What to rate about the state.
        instructions: String,
        /// Between two and ten ordered levels, lowest first.
        criteria: Vec<Option<String>>,
    },
}

impl Question {
    /// A yes/no question.
    pub fn boolean(instructions: impl Into<String>) -> Self {
        Self::Boolean {
            instructions: instructions.into(),
        }
    }

    /// A pick among named `options`, each with an optional description.
    pub fn choice<K: Into<String>>(
        instructions: impl Into<String>,
        options: impl IntoIterator<Item = (K, Option<String>)>,
    ) -> Self {
        Self::Choice {
            instructions: instructions.into(),
            criteria: options.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    /// A rating on ordered `levels`, lowest first.
    pub fn score(
        instructions: impl Into<String>,
        levels: impl IntoIterator<Item = Option<String>>,
    ) -> Self {
        Self::Score {
            instructions: instructions.into(),
            criteria: levels.into_iter().collect(),
        }
    }
}

/// One evaluation: a state and the questions asked about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Request {
    /// Anything JSON; Jev reads it as the situation to judge.
    pub state: Value,
    /// Question id to question. Ids come back as answer keys.
    pub questions: BTreeMap<String, Question>,
}

impl Request {
    /// A request about `state` with no questions yet.
    pub fn new(state: Value) -> Self {
        Self {
            state,
            questions: BTreeMap::new(),
        }
    }

    /// Adds a question under `id`.
    #[must_use]
    pub fn ask(mut self, id: impl Into<String>, question: Question) -> Self {
        self.questions.insert(id.into(), question);
        self
    }

    /// Whether there is nothing to ask. Jev refuses an empty question map.
    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    /// The JSON body `provider` expects. `TypeSafe` takes the model in the
    /// body and calls boolean questions `noul`; the gateway takes the
    /// model as a header.
    pub fn to_body(&self, provider: Provider, model: &str) -> Value {
        let mut body = serde_json::to_value(self).unwrap_or(Value::Null);
        if provider == Provider::TypeSafe
            && let Some(object) = body.as_object_mut()
        {
            object.insert("model".to_owned(), Value::from(model));
            if let Some(Value::Object(questions)) = object.get_mut("questions") {
                for question in questions.values_mut() {
                    if question["type"] == "boolean" {
                        question["type"] = Value::from("noul");
                    }
                }
            }
        }
        body
    }
}

/// One typed answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// P(true), in [0, 1].
    Boolean {
        /// Model-estimated probability that the answer is true.
        probability: f64,
    },
    /// The most likely option.
    Choice {
        /// The option picked.
        choice: String,
        /// Probability per option name.
        probabilities: BTreeMap<String, f64>,
        /// Jev's confidence in the whole distribution, when reported.
        confidence: Option<f64>,
    },
    /// A position between the first and last level.
    Score {
        /// Weighted mean level, in [0, levels - 1].
        score: f64,
        /// Probability per level index, as a string.
        probabilities: BTreeMap<String, f64>,
        /// Jev's confidence in the whole distribution, when reported.
        confidence: Option<f64>,
    },
}

/// Jev's answers plus what the call cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// Question id to answer.
    pub answers: BTreeMap<String, Answer>,
    /// Input tokens billed.
    pub input_tokens: u64,
    /// Output tokens reported (not billed at the time of writing).
    pub output_tokens: u64,
    /// Cost in USD: reported by the gateway, computed for `TypeSafe`.
    pub cost_usd: Option<f64>,
    /// Wall time of the round trip.
    pub latency: Duration,
}

/// Both providers' answer shapes: the gateway sends `boolean` with
/// `probability`, `TypeSafe` sends `noul` with `noul`.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WireAnswer {
    Boolean {
        probability: f64,
    },
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: Option<f64>,
    },
}

#[derive(Deserialize)]
struct WireResponse {
    answers: BTreeMap<String, WireAnswer>,
    #[serde(default)]
    usage: WireUsage,
    #[serde(default, rename = "providerMetadata")]
    provider_metadata: Value,
}

#[derive(Default, Deserialize)]
struct WireUsage {
    #[serde(default, rename = "inputTokens", alias = "input_tokens")]
    input_tokens: u64,
    #[serde(default, rename = "outputTokens", alias = "output_tokens")]
    output_tokens: u64,
}

impl Response {
    /// Parses a successful response body from `provider`.
    pub fn parse(provider: Provider, body: &[u8], latency: Duration) -> Result<Self, JevError> {
        let wire: WireResponse = serde_json::from_slice(body)?;
        // The gateway moves TypeSafe's confidence into provider metadata.
        let metadata_confidence = |id: &str| {
            wire.provider_metadata
                .pointer(&format!("/typesafe/confidence/{id}"))?
                .as_f64()
        };
        let answers = wire
            .answers
            .into_iter()
            .map(|(id, answer)| {
                let answer = match answer {
                    WireAnswer::Boolean { probability }
                    | WireAnswer::Noul { noul: probability } => Answer::Boolean { probability },
                    WireAnswer::Choice {
                        choice,
                        probabilities,
                        confidence,
                    } => Answer::Choice {
                        choice,
                        probabilities,
                        confidence: confidence.or_else(|| metadata_confidence(&id)),
                    },
                    WireAnswer::Score {
                        score,
                        probabilities,
                        confidence,
                    } => Answer::Score {
                        score,
                        probabilities,
                        confidence: confidence.or_else(|| metadata_confidence(&id)),
                    },
                };
                (id, answer)
            })
            .collect();
        let cost_usd = match provider {
            Provider::Gateway => wire
                .provider_metadata
                .pointer("/gateway/cost")
                .and_then(|c| {
                    c.as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| c.as_f64())
                }),
            Provider::TypeSafe => Some(
                u32::try_from(wire.usage.input_tokens).map_or(f64::from(u32::MAX), f64::from)
                    * TYPESAFE_USD_PER_INPUT_TOKEN,
            ),
        };
        Ok(Self {
            answers,
            input_tokens: wire.usage.input_tokens,
            output_tokens: wire.usage.output_tokens,
            cost_usd,
            latency,
        })
    }

    /// P(true) for a boolean question.
    pub fn probability(&self, id: &str) -> Option<f64> {
        match self.answers.get(id)? {
            Answer::Boolean { probability } => Some(*probability),
            _ => None,
        }
    }

    /// The option picked for a choice question and its probability.
    pub fn choice(&self, id: &str) -> Option<(&str, f64)> {
        match self.answers.get(id)? {
            Answer::Choice {
                choice,
                probabilities,
                ..
            } => Some((
                choice.as_str(),
                probabilities.get(choice).copied().unwrap_or(1.0),
            )),
            _ => None,
        }
    }

    /// The level for a score question.
    pub fn score(&self, id: &str) -> Option<f64> {
        match self.answers.get(id)? {
            Answer::Score { score, .. } => Some(*score),
            _ => None,
        }
    }
}

/// Why an evaluation produced no answers. Callers fall back to Tirith's
/// deterministic behavior on any of these.
#[derive(Debug, Error)]
pub enum JevError {
    /// The request could not be sent or the body not read.
    #[error("jev transport: {0}")]
    Transport(String),
    /// The provider answered with an error status.
    #[error("jev returned {status}: {message}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// The provider's error message, or the start of the body.
        message: String,
    },
    /// The body was not the expected JSON.
    #[error("jev response: {0}")]
    Decode(#[from] serde_json::Error),
    /// No answer within the configured timeout.
    #[error("jev timed out")]
    Timeout,
}

impl JevError {
    /// The error for an unsuccessful HTTP `status` with response `body`:
    /// the provider's `error.message` (or `detail`, or `message`), or the
    /// start of the body when it has none of those.
    pub fn from_status(status: u16, body: &[u8]) -> Self {
        let found = serde_json::from_slice::<Value>(body).ok().and_then(|v| {
            ["/error/message", "/error", "/detail", "/message"]
                .iter()
                .find_map(|p| v.pointer(p).filter(|m| !m.is_object()).cloned())
        });
        let message = found.map_or_else(
            || String::from_utf8_lossy(body).chars().take(300).collect(),
            |m| m.as_str().map_or_else(|| m.to_string(), str::to_owned),
        );
        Self::Status { status, message }
    }

    /// Whether trying again within the same evaluation may succeed:
    /// request timeouts, server and overload errors (`TypeSafe` uses 529),
    /// and dropped connections. Not a rate limit (429), whose backoff is
    /// longer than an evaluation's budget, nor a client timeout or a bad
    /// request.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Status { status, .. } => *status == 408 || *status >= 500,
            Self::Transport(_) => true,
            Self::Decode(_) | Self::Timeout => false,
        }
    }
}

/// A boxed evaluation future, so [`Evaluator`] stays object safe.
pub type EvalFuture<'a> = Pin<Box<dyn Future<Output = Result<Response, JevError>> + Send + 'a>>;

/// Something that answers [`Request`]s: the HTTP client, or a fake.
pub trait Evaluator: fmt::Debug + Send + Sync {
    /// Asks every question in `request` in one round trip.
    fn evaluate(&self, request: Request) -> EvalFuture<'_>;

    /// Who is asked, for reports, e.g. `typesafe/jev-latest`.
    fn model(&self) -> &str;
}

/// Pause before the one retry of a transient failure.
#[cfg(feature = "jev")]
const RETRY_AFTER: Duration = Duration::from_millis(150);

/// Sends evaluations to a [`Provider`] over HTTPS, reusing one connection
/// pool for the daemon's lifetime.
///
/// It runs on its own hyper client with rustls on the ring provider and
/// Mozilla's root certificates, not on reqwest: reqwest's rustls without a
/// built-in provider would make every other `reqwest::Client` in the
/// process (CLI, stdio shim, tray, tests) need one installed first, and
/// its built-in provider (aws-lc) is harder to cross-compile.
#[cfg(feature = "jev")]
#[derive(Debug, Clone)]
pub struct JevClient {
    http: hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<hyper::body::Bytes>,
    >,
    config: JevConfig,
    label: String,
}

#[cfg(feature = "jev")]
impl JevClient {
    /// A client for `config`.
    pub fn new(config: JevConfig) -> Result<Self, JevError> {
        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_provider_and_webpki_roots(rustls::crypto::ring::default_provider())
            .map_err(|e| JevError::Transport(e.to_string()))?
            .https_only()
            .enable_http1()
            .build();
        let http =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .pool_idle_timeout(Duration::from_secs(300))
                .build(connector);
        let label = format!("{}/{}", config.provider, config.model);
        Ok(Self {
            http,
            config,
            label,
        })
    }

    /// Opens a pooled connection so the first real evaluation does not pay
    /// the TLS handshake. Best effort; the answer is ignored.
    pub async fn warm_up(&self) {
        let request =
            hyper::Request::head(&self.config.endpoint).body(http_body_util::Full::default());
        if let Ok(request) = request {
            let _ = tokio::time::timeout(self.config.timeout, self.http.request(request)).await;
        }
    }

    /// One HTTP attempt. `started` is when the evaluation began, so a
    /// retried call reports its whole latency and shares one timeout.
    async fn send(&self, body: &Value, started: std::time::Instant) -> Result<Response, JevError> {
        use http_body_util::BodyExt as _;
        use hyper::header::{AUTHORIZATION, CONTENT_TYPE};

        let transport = |e: &dyn fmt::Display| JevError::Transport(e.to_string());
        let mut builder = hyper::Request::post(&self.config.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", self.config.api_key))
            .header(CONTENT_TYPE, "application/json");
        if self.config.provider == Provider::Gateway {
            builder = builder
                .header("ai-gateway-protocol-version", "0.0.1")
                .header("ai-gateway-auth-method", "api-key")
                .header("ai-evaluation-model-specification-version", "4")
                .header("ai-model-id", &self.config.model);
        }
        let request = builder
            .body(http_body_util::Full::new(serde_json::to_vec(body)?.into()))
            .map_err(|e| transport(&e))?;
        let exchange = async {
            let response = self
                .http
                .request(request)
                .await
                .map_err(|e| transport(&e))?;
            let status = response.status();
            let bytes = response
                .into_body()
                .collect()
                .await
                .map_err(|e| transport(&e))?
                .to_bytes();
            Ok::<_, JevError>((status, bytes))
        };
        let remaining = self.config.timeout.saturating_sub(started.elapsed());
        let (status, bytes) = tokio::time::timeout(remaining, exchange)
            .await
            .map_err(|_| JevError::Timeout)??;
        if !status.is_success() {
            return Err(JevError::from_status(status.as_u16(), &bytes));
        }
        Response::parse(self.config.provider, &bytes, started.elapsed())
    }
}

#[cfg(feature = "jev")]
impl Evaluator for JevClient {
    fn evaluate(&self, request: Request) -> EvalFuture<'_> {
        Box::pin(async move {
            let body = request.to_body(self.config.provider, &self.config.model);
            let started = std::time::Instant::now();
            match self.send(&body, started).await {
                Err(error)
                    if error.is_transient()
                        && started.elapsed() + RETRY_AFTER < self.config.timeout =>
                {
                    tokio::time::sleep(RETRY_AFTER).await;
                    self.send(&body, started).await
                }
                result => result,
            }
        })
    }

    fn model(&self) -> &str {
        &self.label
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> Request {
        Request::new(json!("The capital of France is Paris."))
            .ask(
                "correct",
                Question::boolean("Is the statement factually correct?"),
            )
            .ask(
                "tone",
                Question::choice(
                    "What is the tone?",
                    [
                        ("neutral", Some("Plain and factual.".to_owned())),
                        ("playful", None),
                    ],
                ),
            )
            .ask(
                "quality",
                Question::score(
                    "Rate the quality.",
                    ["Poor.", "Acceptable.", "Excellent."].map(|s| Some(s.to_owned())),
                ),
            )
    }

    #[test]
    fn gateway_bodies_match_the_sdk_shape() {
        assert_eq!(
            sample().to_body(Provider::Gateway, "typesafe-ai/jev"),
            json!({
                "state": "The capital of France is Paris.",
                "questions": {
                    "correct": { "type": "boolean", "instructions": "Is the statement factually correct?" },
                    "tone": { "type": "choice", "instructions": "What is the tone?",
                              "criteria": { "neutral": "Plain and factual.", "playful": null } },
                    "quality": { "type": "score", "instructions": "Rate the quality.",
                                 "criteria": ["Poor.", "Acceptable.", "Excellent."] }
                }
            })
        );
    }

    #[test]
    fn typesafe_bodies_carry_the_model_and_call_booleans_noul() {
        let body = sample().to_body(Provider::TypeSafe, "jev-latest");
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["questions"]["correct"]["type"], "noul");
        assert_eq!(body["questions"]["tone"]["type"], "choice");
        assert_eq!(body["questions"]["quality"]["type"], "score");
    }

    #[test]
    fn gateway_responses_parse_answers_usage_cost_and_confidence() {
        // Shape captured from a live call on 2026-09-16.
        let body = br#"{"answers":{"relevant":{"type":"boolean","probability":0.47},
            "next":{"type":"choice","choice":"read_notice","probabilities":{"read_notice":0.73,"edit_now":0.26,"wait":0.01}},
            "risk":{"type":"score","score":1,"probabilities":{"0":0.13,"1":0.74,"2":0.13}}},
            "rounding":{"probabilityDecimals":2,"scoreDecimals":2},"usage":{"inputTokens":440,"outputTokens":73},
            "warnings":[],"providerMetadata":{"typesafe":{"confidence":{"next":0.59}},"gateway":{"cost":"0.00001848"}}}"#;
        let response =
            Response::parse(Provider::Gateway, body, Duration::from_millis(315)).unwrap();
        assert_eq!(response.probability("relevant"), Some(0.47));
        assert_eq!(response.choice("next"), Some(("read_notice", 0.73)));
        assert!(matches!(
            response.answers["next"],
            Answer::Choice { confidence: Some(c), .. } if (c - 0.59).abs() < f64::EPSILON
        ));
        assert_eq!(response.score("risk"), Some(1.0));
        assert_eq!(response.probability("next"), None);
        assert_eq!(response.input_tokens, 440);
        assert_eq!(response.output_tokens, 73);
        assert_eq!(response.cost_usd, Some(0.000_018_48));
    }

    #[test]
    fn typesafe_responses_parse_noul_snake_case_usage_and_computed_cost() {
        // Shape from docs.typesafe.ai/api (2026-09-16).
        let body = br#"{"model":"jev-latest","answers":{
            "is_urgent":{"type":"noul","noul":0.82},
            "frustration":{"type":"score","score":1.6,"legend":{"0":"Calm","1":"Frustrated","2":"Very angry"},
                           "probabilities":{"0":0.05,"1":0.3,"2":0.65},"confidence":0.78}},
            "usage":{"input_tokens":1000000,"output_tokens":48}}"#;
        let response =
            Response::parse(Provider::TypeSafe, body, Duration::from_millis(90)).unwrap();
        assert_eq!(response.probability("is_urgent"), Some(0.82));
        assert_eq!(response.score("frustration"), Some(1.6));
        assert_eq!(response.input_tokens, 1_000_000);
        assert!((response.cost_usd.unwrap() - 0.042).abs() < 1e-9);
    }

    #[test]
    fn error_bodies_surface_their_message() {
        let body = br#"{"error":{"message":"Model 'x' not found","type":"model_not_found"}}"#;
        assert_eq!(
            JevError::from_status(404, body).to_string(),
            "jev returned 404: Model 'x' not found"
        );
        assert_eq!(
            JevError::from_status(422, br#"{"detail":"questions: required"}"#).to_string(),
            "jev returned 422: questions: required"
        );
        assert_eq!(
            JevError::from_status(502, b"plain").to_string(),
            "jev returned 502: plain"
        );
        assert!(JevError::from_status(529, b"").is_transient());
        assert!(!JevError::from_status(429, b"").is_transient());
        assert!(!JevError::from_status(401, b"").is_transient());
    }

    #[test]
    fn dotenv_values_handle_export_quotes_and_comments() {
        let file = "# comment\nexport A=\"one\"\nB='two'\nC=three\nD=\n";
        assert_eq!(dotenv_value(file, "A").as_deref(), Some("one"));
        assert_eq!(dotenv_value(file, "B").as_deref(), Some("two"));
        assert_eq!(dotenv_value(file, "C").as_deref(), Some("three"));
        assert_eq!(dotenv_value(file, "D"), None);
        assert_eq!(dotenv_value(file, "E"), None);
    }

    #[test]
    fn debug_never_prints_the_key() {
        let config = JevConfig::new(Provider::TypeSafe, "ts_secret");
        assert!(!format!("{config:?}").contains("ts_secret"));
    }
}
