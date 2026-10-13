pub mod api;
pub mod auth;
pub mod cast;
pub mod config;
pub mod context;
pub mod curation;
pub mod generation;
pub mod jobs;
pub mod local;
pub mod narrative;
pub mod presets;
pub mod privacy;
pub mod provenance;
pub mod review;
pub mod providers;
pub mod router_log;
pub mod rules;
pub mod store;
pub mod style_policy;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct Error {
    pub status: u16,
    pub message: String,
    pub extra: Value,
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            extra: json!({}),
        }
    }
    pub fn extra(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra[key] = value.into();
        self
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::new(500, format!("SQLite: {e}"))
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new(500, format!("Arquivo: {e}"))
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut body = self.extra;
        body["error"] = json!(self.message);
        (
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(body),
        )
            .into_response()
    }
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_owned()
}
pub fn n(v: &Value, k: &str) -> i64 {
    v[k].as_i64().unwrap_or_default()
}
pub fn parsed(v: &Value, k: &str, default: Value) -> Value {
    v[k].as_str()
        .and_then(|x| serde_json::from_str(x).ok())
        .unwrap_or(default)
}
