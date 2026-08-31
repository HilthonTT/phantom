//! User-interactive authentication: the extra steps before a sensitive
//! request is allowed through.
//!
//! An endpoint that needs UIAA answers the first request with a session id and
//! the flows that would satisfy it. The client then re-sends the same request
//! with an `auth` object, and each stage it completes is recorded against that
//! session until one flow's stages are all done.
//!
//! Two pieces of state are kept per session. The stages completed so far go in
//! the database, since a session outlives a restart. The original request body
//! is held in memory instead: it is only needed to replay the request once the
//! last stage passes, and a client that gives up mid-flow should not leave the
//! body it sent behind on disk.

use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, RwLock},
};

use phantom_core::{Err, Result, err, error, hash, implement, rand, text::EMPTY};
use phantom_database::{Deserialized, Json, Map};
use ruma::{
    CanonicalJsonValue, DeviceId, OwnedDeviceId, OwnedUserId, UserId,
    api::{
        client::uiaa::{
            AuthData, AuthType, MatrixUserIdentifier, Password, UiaaInfo, UserIdentifier,
        },
        error::{ErrorKind, StandardErrorBody},
    },
};

use crate::{Dep, config, server_state, users};

type RequestMap = BTreeMap<RequestKey, CanonicalJsonValue>;
type RequestKey = (OwnedUserId, OwnedDeviceId, String);

pub struct Service {
    userdevicesessionid_uiaarequest: RwLock<RequestMap>,
    db: Data,
    services: Services,
}

struct Services {
    server_state: Dep<server_state::Service>,
    users: Dep<users::Service>,
    config: Dep<config::Service>,
}

struct Data {
    userdevicesessionid_uiaainfo: Arc<Map>,
}

pub const SESSION_ID_LENGTH: usize = 32;

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            userdevicesessionid_uiaarequest: RwLock::new(RequestMap::new()),
            db: Data {
                userdevicesessionid_uiaainfo: args.db["userdevicesessionid_uiaainfo"].clone(),
            },
            services: Services {
                server_state: args.depend::<server_state::Service>("server_state"),
                users: args.depend::<users::Service>("users"),
                config: args.depend::<config::Service>("config"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub async fn read_tokens(&self) -> Result<HashSet<String>> {
    let mut tokens = HashSet::new();
    if let Some(file) = self.services.config.registration_token_file.as_ref() {
        match std::fs::read_to_string(file) {
            Ok(text) => {
                text.split_ascii_whitespace().for_each(|token| {
                    tokens.insert(token.to_owned());
                });
            }
            Err(e) => error!("Failed to read the registration token file: {e}"),
        }
    }
    if let Some(token) = &self.services.config.registration_token {
        tokens.insert(token.to_owned());
    }

    Ok(tokens)
}

/// Creates a new Uiaa session. Make sure the session token is unique.
#[implement(Service)]
pub fn create(
    &self,
    user_id: &UserId,
    device_id: &DeviceId,
    uiaainfo: &UiaaInfo,
    json_body: &CanonicalJsonValue,
) -> Result<()> {
    // TODO: better session error handling (why is uiaainfo.session optional in
    // ruma?)
    self.set_uiaa_request(
        user_id,
        device_id,
        uiaainfo.session.as_ref().expect("session should be set"),
        json_body,
    );

    self.update_uiaa_session(
        user_id,
        device_id,
        uiaainfo.session.as_ref().expect("session should be set"),
        Some(uiaainfo),
    )
}

#[implement(Service)]
pub async fn try_auth(
    &self,
    user_id: &UserId,
    device_id: &DeviceId,
    auth: &AuthData,
    uiaainfo: &UiaaInfo,
) -> Result<(bool, UiaaInfo)> {
    let mut uiaainfo = if let Some(session) = auth.session() {
        self.get_uiaa_session(user_id, device_id, session).await?
    } else {
        uiaainfo.clone()
    };

    if uiaainfo.session.is_none() {
        uiaainfo.session = Some(rand::string(SESSION_ID_LENGTH));
    }

    match auth {
        // Find out what the user completed
        AuthData::Password(Password {
            identifier,
            password,
            ..
        }) => {
            let UserIdentifier::Matrix(MatrixUserIdentifier { user: username, .. }) = identifier
            else {
                return Err!(Request(Unrecognized("Identifier type not recognized.")));
            };

            let user_id_from_username = UserId::parse_with_server_name(
                username.clone(),
                self.services.server_state.server_name(),
            )
            .map_err(|_| err!(Request(InvalidParam("User ID is invalid."))))?;

            // Check if the access token being used matches the credentials used for UIAA
            if user_id.localpart() != user_id_from_username.localpart() {
                return Err!(Request(Forbidden("User ID and access token mismatch.")));
            }
            let user_id = user_id_from_username;

            // Check if password is correct
            if let Ok(hash) = self.services.users.password_hash(&user_id).await {
                let hash_matches = hash::verify_password(password, &hash).is_ok();
                if !hash_matches {
                    uiaainfo.auth_error = Some(StandardErrorBody::new(
                        ErrorKind::Forbidden,
                        "Invalid username or password.".to_owned(),
                    ));
                    return Ok((false, uiaainfo));
                }
            }

            // Password was correct! Let's add it to `completed`
            uiaainfo.completed.push(AuthType::Password);
        }
        AuthData::RegistrationToken(t) => {
            let tokens = self.read_tokens().await?;
            if tokens.contains(t.token.trim()) {
                uiaainfo.completed.push(AuthType::RegistrationToken);
            } else {
                uiaainfo.auth_error = Some(StandardErrorBody::new(
                    ErrorKind::Forbidden,
                    "Invalid registration token.".to_owned(),
                ));
                return Ok((false, uiaainfo));
            }
        }
        AuthData::Dummy(_) => {
            uiaainfo.completed.push(AuthType::Dummy);
        }
        k => error!("type not supported: {k:?}"),
    }

    // Check if a flow now succeeds
    let mut completed = false;
    'flows: for flow in &uiaainfo.flows {
        for stage in &flow.stages {
            if !uiaainfo.completed.contains(stage) {
                continue 'flows;
            }
        }
        // We didn't break, so this flow succeeded!
        completed = true;
        break;
    }

    if !completed {
        self.update_uiaa_session(
            user_id,
            device_id,
            uiaainfo.session.as_ref().expect("session is always set"),
            Some(&uiaainfo),
        )?;

        return Ok((false, uiaainfo));
    }

    // UIAA was successful! Remove this session and return true
    self.update_uiaa_session(
        user_id,
        device_id,
        uiaainfo.session.as_ref().expect("session is always set"),
        None,
    )?;

    Ok((true, uiaainfo))
}

#[implement(Service)]
fn set_uiaa_request(
    &self,
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
    request: &CanonicalJsonValue,
) {
    let key = (user_id.to_owned(), device_id.to_owned(), session.to_owned());
    self.userdevicesessionid_uiaarequest
        .write()
        .expect("locked for writing")
        .insert(key, request.to_owned());
}

#[implement(Service)]
pub fn get_uiaa_request(
    &self,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    session: &str,
) -> Option<CanonicalJsonValue> {
    let key = (
        user_id.to_owned(),
        device_id.unwrap_or_else(|| EMPTY.into()).to_owned(),
        session.to_owned(),
    );

    self.userdevicesessionid_uiaarequest
        .read()
        .expect("locked for reading")
        .get(&key)
        .cloned()
}

#[implement(Service)]
fn update_uiaa_session(
    &self,
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
    uiaainfo: Option<&UiaaInfo>,
) -> Result<()> {
    let key = (user_id, device_id, session);

    if let Some(uiaainfo) = uiaainfo {
        self.db
            .userdevicesessionid_uiaainfo
            .put(key, Json(uiaainfo))
    } else {
        self.db.userdevicesessionid_uiaainfo.del(key)
    }
}

#[implement(Service)]
async fn get_uiaa_session(
    &self,
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
) -> Result<UiaaInfo> {
    let key = (user_id, device_id, session);
    self.db
        .userdevicesessionid_uiaainfo
        .qry(&key)
        .await
        .deserialized()
        .map_err(|_| err!(Request(Forbidden("UIAA session does not exist."))))
}
