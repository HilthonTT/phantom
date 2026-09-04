//! The appservices registered with this server.
//!
//! An appservice is a program the operator registers out of band: it hands the
//! server a registration naming two tokens and the users, aliases and rooms it
//! claims, as regular expressions. Everything the appservice is then allowed
//! to do follows from that file — a request is attributed to it by the
//! `as_token` it carries, and whether it may act as a user, claim an alias or
//! be told about a room is decided by matching the namespaces.
//!
//! Both of those are asked on the request path, and the namespaces are also
//! asked of every locally created user and alias, so the registrations are
//! held in memory with their patterns already compiled. The column is the
//! durable copy: it is read once at startup and written when a registration is
//! added or removed, and nothing reads it to serve a request.
//!
//! Registrations are stored as JSON of the parsed [`Registration`] rather than
//! as the YAML file they arrived in. The parsed form is what this server acts
//! on, so storing it is what keeps the stored copy and the enforced copy from
//! being able to disagree; parsing the operator's YAML is the caller's job,
//! one layer out.

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

/// Every registration this server serves, by registration id.
pub type Registrations = BTreeMap<String, RegistrationInfo>;

pub struct Service {
    /// The registrations as they are served. Written under the same guard the
    /// collision checks read, so two registrations racing cannot both be let
    /// through against a map neither of them saw the other in.
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

    /// Fills the map from the database.
    ///
    /// Until this has run the server behaves as though nothing is registered,
    /// which is why it is the whole of the worker rather than something the
    /// first lookup does: workers are started before the listeners are, so no
    /// appservice request can arrive ahead of it.
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

/// Registers an appservice, replacing whatever was filed under the same id.
///
/// Re-registering an id is how a registration is updated, so it is the one
/// case that is not a collision with itself. Anything else that would make two
/// registrations indistinguishable — a shared token, an exclusive claim
/// another appservice already holds — is refused here rather than left to be
/// resolved arbitrarily at match time. See [`check_collisions`].
///
/// The database is written before the map: a write that failed after the map
/// had been updated would have this server enforcing a registration that does
/// not survive a restart, while the other way round the next startup reads
/// back exactly what was stored.
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

/// Removes the registration of `appservice_id`.
///
/// Anything the appservice has queued to be sent to it outlives this: dropping
/// that queue belongs to the sending service, which phantom does not have yet.
/// Once it does, this is where its events for the appservice have to go, or
/// the queue is retried against a URL nothing is registered for.
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

/// The registration of `id`, if it is registered.
#[implement(Service)]
pub async fn get_registration(&self, id: &str) -> Option<Registration> {
    self.read()
        .await
        .get(id)
        .map(|info| info.registration.clone())
}

/// The id of every registered appservice, in order.
#[implement(Service)]
pub async fn iter_ids(&self) -> Vec<String> {
    self.read().await.keys().cloned().collect()
}

/// Every registration, in id order.
///
/// From the map rather than the database: the two are kept in step by
/// [`register_appservice`](Service::register_appservice) and
/// [`unregister_appservice`](Service::unregister_appservice), and the map is
/// what every other lookup here answers from.
#[implement(Service)]
pub async fn all(&self) -> Vec<(String, Registration)> {
    self.read()
        .await
        .iter()
        .map(|(id, info)| (id.clone(), info.registration.clone()))
        .collect()
}

/// The appservice a request bearing `token` is from.
///
/// A linear scan, because the map is keyed by registration id and an
/// installation runs a handful of appservices at most; the alternative is a
/// second index to keep in step with this one for no measurable gain.
#[implement(Service)]
pub async fn find_from_token(&self, token: &str) -> Option<RegistrationInfo> {
    self.read()
        .await
        .values()
        .find(|info| info.registration.as_token == token)
        .cloned()
}

/// Whether any appservice claims `user_id` exclusively.
#[implement(Service)]
pub async fn is_exclusive_user_id(&self, user_id: &UserId) -> bool {
    self.read()
        .await
        .values()
        .any(|info| info.is_exclusive_user_match(user_id))
}

/// Whether any appservice claims `alias` exclusively.
#[implement(Service)]
pub async fn is_exclusive_alias(&self, alias: &RoomAliasId) -> bool {
    self.read()
        .await
        .values()
        .any(|info| info.aliases.is_exclusive_match(alias.as_str()))
}

/// Whether any appservice claims `room_id` exclusively.
#[implement(Service)]
pub async fn is_exclusive_room_id(&self, room_id: &RoomId) -> bool {
    self.read()
        .await
        .values()
        .any(|info| info.rooms.is_exclusive_match(room_id.as_str()))
}

/// The registrations, for a caller doing something the methods above do not
/// cover. Held for as short a time as possible: a registration cannot be added
/// or removed while this guard is alive.
#[implement(Service)]
pub async fn read(&self) -> RwLockReadGuard<'_, Registrations> {
    self.registration_info.read().await
}

/// Checks what a registration has to satisfy to be filed at all, and compiles
/// its namespaces.
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

/// Reads every stored registration and compiles its namespaces.
///
/// A stored registration that no longer compiles — a pattern the regex crate
/// has since stopped accepting, a value written by an older version — is
/// logged and left out rather than taken down with the server: one appservice
/// that cannot be served is not a reason to serve none of the others, and the
/// operator has a name to go and fix.
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

/// Checks a registration against the ones already registered.
///
/// Two appservices may not share a token. `as_token` is what a request is
/// attributed by, so a shared one would credit a request to whichever
/// registration sorted first rather than to the one that sent it; `hs_token`
/// is what tells an appservice a request really came from this server, so one
/// shared between two of them lets either speak as this server to the other.
///
/// The rest is about exclusive namespaces, which the spec gives to a single
/// appservice. Whether two arbitrary regexes overlap is not a question the
/// regex crate can answer, so what is refused is the part that can be decided:
/// a pattern another appservice already claims exclusively, verbatim, and
/// either appservice's own sender user falling inside the other's exclusive
/// user namespace. Patterns that overlap without being equal are left to the
/// operator, who is the one holding both registration files.
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

/// The first pattern both registrations claim exclusively, if there is one.
///
/// Compared within a kind: the same string as a user namespace and as a room
/// namespace matches different things, and claiming both is not a conflict.
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

/// The patterns a registration claims exclusively, out of one kind of
/// namespace.
fn exclusive(namespaces: &[Namespace]) -> impl Iterator<Item = &str> {
    namespaces
        .iter()
        .filter(|namespace| namespace.exclusive)
        .map(|namespace| namespace.regex.as_str())
}
