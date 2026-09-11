mod namespace_regex;
mod registration_info;
mod request;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, fmt::Write, sync::Arc};

use async_trait::async_trait;
use futures::StreamExt;
use phantom_core::{Err, Result, err, implement, server::Server, stream::ReadyExt, warn};
use phantom_database::{Json, Map};
use ruma::{
    RoomAliasId, RoomId, ServerName, UserId,
    api::appservice::{Namespace, Namespaces, Registration},
};
use tokio::sync::{RwLock, RwLockReadGuard};

pub use self::{namespace_regex::NamespaceRegex, registration_info::RegistrationInfo};
use crate::{Dep, client};

pub type Registrations = BTreeMap<String, RegistrationInfo>;

pub struct Service {
    registration_info: RwLock<Registrations>,
    server: Arc<Server>,
    services: Services,
    db: Data,
}

struct Services {
    client: Dep<client::Service>,
}

struct Data {
    id_appserviceregistrations: Arc<Map>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            registration_info: RwLock::new(BTreeMap::new()),
            server: args.server.clone(),
            services: Services {
                client: args.depend::<client::Service>("client"),
            },
            db: Data {
                id_appserviceregistrations: args.db["id_appserviceregistrations"].clone(),
            },
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        let loaded = self.load_from_db().await?;

        *self.registration_info.write().await = loaded;

        Ok(())
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        let count = self.registration_info.read().await.len();

        writeln!(out, "appservice_registrations: {count}")?;

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub async fn register_appservice(&self, registration: &Registration) -> Result {
    let info = self.validate(registration)?;
    let id = info.registration.id.clone();

    let mut registrations = self.registration_info.write().await;

    check_collisions(&registrations, &info, &self.server.name)?;

    self.db
        .id_appserviceregistrations
        .raw_put(&id, Json(&info.registration))?;

    registrations.insert(id, info);

    Ok(())
}

#[implement(Service)]
pub async fn unregister_appservice(&self, appservice_id: &str) -> Result {
    let mut registrations = self.registration_info.write().await;

    if !registrations.contains_key(appservice_id) {
        return Err!(Request(NotFound(
            "Appservice {appservice_id:?} is not registered."
        )));
    }

    self.db.id_appserviceregistrations.remove(appservice_id)?;

    registrations.remove(appservice_id);

    Ok(())
}

#[implement(Service)]
pub async fn get_registration(&self, id: &str) -> Option<Registration> {
    self.read()
        .await
        .get(id)
        .map(|info| info.registration.clone())
}

#[implement(Service)]
pub async fn iter_ids(&self) -> Vec<String> {
    self.read().await.keys().cloned().collect()
}

#[implement(Service)]
pub async fn all(&self) -> Vec<(String, Registration)> {
    self.read()
        .await
        .iter()
        .map(|(id, info)| (id.clone(), info.registration.clone()))
        .collect()
}

#[implement(Service)]
pub async fn find_from_token(&self, token: &str) -> Option<RegistrationInfo> {
    self.read()
        .await
        .values()
        .find(|info| info.registration.as_token == token)
        .cloned()
}

#[implement(Service)]
pub async fn is_exclusive_user_id(&self, user_id: &UserId) -> bool {
    self.read()
        .await
        .values()
        .any(|info| info.is_exclusive_user_match(user_id))
}

#[implement(Service)]
pub async fn is_exclusive_alias(&self, alias: &RoomAliasId) -> bool {
    self.read()
        .await
        .values()
        .any(|info| info.aliases.is_exclusive_match(alias.as_str()))
}

#[implement(Service)]
pub async fn is_exclusive_room_id(&self, room_id: &RoomId) -> bool {
    self.read()
        .await
        .values()
        .any(|info| info.rooms.is_exclusive_match(room_id.as_str()))
}

#[implement(Service)]
pub async fn read(&self) -> RwLockReadGuard<'_, Registrations> {
    self.registration_info.read().await
}

#[implement(Service)]
fn validate(&self, registration: &Registration) -> Result<RegistrationInfo> {
    let id = &registration.id;

    if id.is_empty() {
        return Err!(Request(InvalidParam("Appservice registration has no id.")));
    }

    if registration.as_token.is_empty() || registration.hs_token.is_empty() {
        return Err!(Request(InvalidParam(
            "Appservice {id:?} has an empty as_token or hs_token."
        )));
    }

    let info = RegistrationInfo::try_from(registration.clone()).map_err(|e| {
        err!(Request(InvalidParam(
            "Appservice {id:?} has a namespace regex that does not compile: {e}"
        )))
    })?;

    info.sender_user(&self.server.name).map_err(|e| {
        err!(Request(InvalidParam(
            "Appservice {id:?} has an invalid sender_localpart: {e}"
        )))
    })?;

    Ok(info)
}

#[implement(Service)]
async fn load_from_db(&self) -> Result<Registrations> {
    let loaded: Registrations = self
        .db
        .id_appserviceregistrations
        .stream::<&str, Registration>()
        .ready_filter_map(|entry| match entry {
            Ok((id, registration)) => match RegistrationInfo::try_from(registration) {
                Ok(info) => Some((id.to_owned(), info)),
                Err(e) => {
                    warn!("Ignoring appservice {id:?}: its namespaces no longer compile: {e}");
                    None
                }
            },
            Err(e) => {
                warn!("Ignoring an unreadable appservice registration: {e}");
                None
            }
        })
        .collect()
        .await;

    Ok(loaded)
}

fn check_collisions(
    registered: &Registrations,
    new: &RegistrationInfo,
    server_name: &ServerName,
) -> Result {
    let new_id = &new.registration.id;
    let new_sender = new.sender_user(server_name).ok();

    for (id, other) in registered {
        if id == new_id {
            continue;
        }

        if other.registration.as_token == new.registration.as_token
            || other.registration.hs_token == new.registration.hs_token
        {
            return Err!(Request(InvalidParam(warn!(
                "Appservice {id:?} is already registered with one of these tokens."
            ))));
        }

        if let Some(sender) = new_sender.as_deref()
            && other.is_exclusive_user_match(sender)
        {
            return Err!(Request(InvalidParam(warn!(
                "Appservice {id:?} exclusively claims {sender}, which this registration sends as."
            ))));
        }

        if let Ok(sender) = other.sender_user(server_name)
            && new.is_exclusive_user_match(&sender)
        {
            return Err!(Request(InvalidParam(warn!(
                "This registration exclusively claims {sender}, which appservice {id:?} sends as."
            ))));
        }

        if let Some(regex) =
            exclusive_overlap(&other.registration.namespaces, &new.registration.namespaces)
        {
            return Err!(Request(InvalidParam(warn!(
                "Appservice {id:?} already claims the exclusive namespace {regex:?}."
            ))));
        }
    }

    Ok(())
}

fn exclusive_overlap<'a>(lhs: &'a Namespaces, rhs: &Namespaces) -> Option<&'a str> {
    [
        (&lhs.users, &rhs.users),
        (&lhs.aliases, &rhs.aliases),
        (&lhs.rooms, &rhs.rooms),
    ]
    .into_iter()
    .find_map(|(lhs, rhs)| {
        exclusive(lhs).find(|pattern| exclusive(rhs).any(|other| other == *pattern))
    })
}

fn exclusive(namespaces: &[Namespace]) -> impl Iterator<Item = &str> {
    namespaces
        .iter()
        .filter(|namespace| namespace.exclusive)
        .map(|namespace| namespace.regex.as_str())
}
