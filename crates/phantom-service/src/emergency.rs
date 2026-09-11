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
        if self.services.server.config.database.rocksdb_read_only {
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

#[implement(Service)]
async fn set_emergency_access(&self) -> Result {
    let server_user = &self.services.server_state.server_user;
    let emergency_password = self.services.server.config.admin.emergency_password.clone();

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
