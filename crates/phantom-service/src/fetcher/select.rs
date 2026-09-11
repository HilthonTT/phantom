use std::sync::Arc;

use async_trait::async_trait;

use super::opts::Opts;
use crate::{Services, federation::Candidates};

#[async_trait]
pub(super) trait Select: Send + Sync {
    async fn candidates(&self, opts: &Opts) -> Candidates;
}

pub(super) struct RoomCandidates {
    pub(super) services: Arc<Services>,
}

#[async_trait]
impl Select for RoomCandidates {
    async fn candidates(&self, _opts: &Opts) -> Candidates {
        let _ = &self.services;

        todo!("rank the hint, the room's servers, and the id origins")
    }
}
