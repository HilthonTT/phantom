use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

use async_trait::async_trait;
use ruma::{OwnedEventId, OwnedRoomAliasId, OwnedUserId};
use tokio::sync::mpsc::{Receiver, Sender, channel};

pub struct Service {
    services: Services,
    queue: RwLock<Queue>,
    pub command: RwLock<Option<Arc<dyn Command>>>,
    pub admin_alias: OwnedRoomAliasId,
    register_nonces: Mutex<BTreeMap<String, Instant>>,
    #[cfg(feature = "console")]
    pub console: Arc<console::Console>,
}

struct Services {}

enum Queue {
    Pending,
    Open(Sender<CommandInput>),
    Closed,
}

#[derive(Clone, Debug, Default)]
pub struct CommandInput {
    pub command: String,

    pub reply_id: Option<OwnedEventId>,

    pub sender: Option<OwnedUserId>,
}

#[async_trait]
pub trait Command: Send + Sync + 'static {
    fn clap(&self) -> clap::Command;
}
