//! Mac pairing uses Unix peer credentials upstream and is unavailable on this target.
use crate::session::Manager;
use axum::Router;
use serde_json::Value;
use std::{io, sync::Arc};
pub const SKILL: &str = "Mac pairing is unavailable in the native Windows local runtime.";
pub const ROOM_CLI_SKILL: &str = SKILL;
pub(crate) const SOCKET: &str = "";
#[derive(Default)]
pub struct Mac {
    _unavailable: (),
}
impl Mac {
    pub(crate) fn expect(&self) {}
}
pub fn routes() -> Router<Arc<Manager>> {
    Router::new()
}
pub async fn listen(_: &Arc<Manager>) -> io::Result<()> {
    Ok(())
}
pub async fn cli(_: &[String]) -> io::Result<i32> {
    Err(io::Error::other(SKILL))
}
pub(crate) async fn request(_: &str, _: &str, _: &str, _: Option<Value>) -> io::Result<Value> {
    Err(io::Error::other(
        "Agent-local room-cli transport is unavailable on Windows; use the authenticated HTTP API",
    ))
}
