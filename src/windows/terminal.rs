//! ConPTY support is not yet available; never advertise a working terminal.
use axum::{Router, http::StatusCode, routing::get};
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Default)]
pub struct Terminals;
impl Terminals {
    pub fn summary(&self) -> Value {
        json!({"open":0,"lastActivity":null,"available":false})
    }
}
pub fn routes() -> Router<Arc<crate::session::Manager>> {
    Router::new().route(
        "/v1/terminals/{id}",
        get(|| async {
            (
                StatusCode::NOT_IMPLEMENTED,
                "ConPTY terminals are unavailable on Windows",
            )
        }),
    )
}
