//! SSH previews need a separate Windows transport port.
use crate::{config::Config, session::Manager};
use axum::Router;
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
pub const SKILL: &str = "SSH previews are unavailable in the native Windows local runtime.";
pub struct Previews;
impl Previews {
    pub fn open(_: &Config) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self))
    }
    pub(crate) fn agent(&self) -> Option<(PathBuf, u32)> {
        None
    }
    pub fn enabled(&self) -> bool {
        false
    }
    pub async fn listen(&self) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Peer(pub(crate) Option<u32>, pub(crate) Option<i32>);
pub fn routes() -> Router<Arc<Manager>> {
    Router::new()
}
pub async fn cli(_: &[String]) -> io::Result<()> {
    Err(io::Error::other(SKILL))
}
pub fn authorized_keys(_: &Path, _: u32) -> io::Result<()> {
    Err(io::Error::other(SKILL))
}
