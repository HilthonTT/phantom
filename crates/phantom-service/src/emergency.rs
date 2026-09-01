//! The way back in when every admin account is locked out.
//!
//! The server posts as a user of its own, and that account is normally
//! unusable — no password, deactivated. Setting `emergency_password` turns it
//! into an account an operator can log into and use to recover a real admin
//! account, since the server user is in the admin room already.
//!
//! It is applied once at startup rather than watched for, so recovery is
//! deliberate: setting it takes a restart, and clearing it takes another,
//! which is what closes the account again and logs out whatever was opened
//! with it.

use std::sync::Arc;

use async_trait::async_trait;
use phantom_core::{Result, error, implement, server::Server, warn};
use ruma::{
    events::{
        GlobalAccountDataEvent, GlobalAccountDataEventType, push_rules::PushRulesEventContent,
    },
    push::Ruleset,
};

use crate::{Dep, account_data, server_state, users};

pub struct Service {
    services: Services,
}

struct Services {
    server: Arc<Server>,
    account_data: Dep<account_data::Service>,
    server_state: Dep<server_state::Service>,
    users: Dep<users::Service>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                account_data: args.depend::<account_data::Service>("account_data"),
                server_state: args.depend::<server_state::Service>("server_state"),
                users: args.depend::<users::Service>("users"),
            },
        }))
    }

    async fn worker(self: Arc<Self>) -> Result {
        if self.services.server.config.rocksdb_read_only {
            return Ok(());
        }

        self.set_emergency_access().await.inspect_err(|e| {
            error!("Could not set the configured emergency password for the server user: {e}");
        })
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Opens or closes the server user's account, according to the config.
///
/// The push rules go with the password: an account nobody can log into has no
/// use for a ruleset, and one an operator is about to use needs the default
/// one so what happens in the admin room reaches them.
#[implement(Service)]
async fn set_emergency_access(&self) -> Result {
    let server_user = &self.services.server_state.server_user;
    let emergency_password = self.services.server.config.emergency_password.clone();

    self.services
        .users
        .set_password(server_user, emergency_password.as_deref())?;

    let (ruleset, pwd_set) = match emergency_password {
        Some(_) => (Ruleset::server_default(server_user), true),
        None => (Ruleset::new(), false),
    };

    self.services
        .account_data
        .update(
            None,
            server_user,
            GlobalAccountDataEventType::PushRules.to_string().into(),
            &serde_json::to_value(GlobalAccountDataEvent::new(PushRulesEventContent::new(
                ruleset,
            )))
            .expect("to json value always works"),
        )
        .await?;

    if pwd_set {
        warn!(
            "The server account emergency password is set! Please unset it as soon as you \
             finish admin account recovery! You will be logged out of the server service \
             account when you finish."
        );

        return Ok(());
    }

    self.services.users.deactivate_account(server_user).await
}
