//! Sending a request to another homeserver.
//!
//! One request, sent and awaited. There is no queue here and no retry: a
//! caller that needs the request to survive a restart, or to be retried until
//! it lands, belongs behind the outgoing queue rather than here. What this
//! owns is the part every federation request shares — resolving the server
//! name to an address, signing the request with this server's key so the far
//! end will accept it, and turning what comes back into the typed response
//! ruma describes.

mod execute;

use std::sync::Arc;

use async_trait::async_trait;
use phantom_core::{Result, server::Server};

use crate::{Dep, client, resolver, server_keys};

pub struct Service {
    services: Services,
}

struct Services {
    server: Arc<Server>,
    client: Dep<client::Service>,
    resolver: Dep<resolver::Service>,
    server_keys: Dep<server_keys::Service>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                client: args.depend::<client::Service>("client"),
                resolver: args.depend::<resolver::Service>("resolver"),
                server_keys: args.depend::<server_keys::Service>("server_keys"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}
