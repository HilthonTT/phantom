use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use phantom_core::Result;
use ruma::ServerName;

use super::opts::{Op, Opts};
use crate::Services;

#[async_trait]
pub(super) trait Transport: Send + Sync {
    async fn fetch_raw(&self, op: Op, server: &ServerName, opts: &Opts) -> Result<Bytes>;
}

pub(super) struct FederationTransport {
    pub(super) services: Arc<Services>,
}

#[async_trait]
impl Transport for FederationTransport {
    async fn fetch_raw(&self, _op: Op, _server: &ServerName, _opts: &Opts) -> Result<Bytes> {
        let _ = &self.services;

        todo!("route each Op onto federation::execute")
    }
}
