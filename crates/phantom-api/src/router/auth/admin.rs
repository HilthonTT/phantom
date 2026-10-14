//! Authentication for the phantom admin API: a user's access token, presented
//! as `Authorization: Bearer`, whose owner is joined to the admin room.

use std::sync::{Arc, OnceLock};

use axum::extract::FromRequestParts;
use http::request::Parts;
use phantom_core::{Err, Error, Result};
use ruma::{OwnedDeviceId, OwnedUserId};

use super::{
    scheme::{expired_token, unknown_token},
    token::{Token, bearer},
};
use crate::router::State;

/// An admin the request authenticated as.
#[derive(Debug)]
pub struct AdminAuth {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
}

impl FromRequestParts<State> for AdminAuth {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, services: &State) -> Result<Self> {
        let (user_id, device_id) = match Token::lookup(services, bearer(&parts.headers)).await? {
            Token::User(user_id, device_id) => (user_id, device_id),
            Token::None => return Err!(Request(MissingToken("Missing access token."))),
            Token::Expired => return expired_token(),
            Token::Invalid => return unknown_token(),
            Token::Appservice(_) => {
                return Err!(Request(Forbidden("Appservices cannot use the admin API.")));
            }
        };

        if !services.admin.user_is_admin(&user_id).await {
            return Err!(Request(Forbidden(
                "Only server admins can use the admin API."
            )));
        }

        // The audit layer asks who acted; tell it, now that it is known.
        if let Some(witness) = parts.extensions.get::<AdminWitness>() {
            witness.0.set((services.clone(), user_id.clone())).ok();
        }

        Ok(Self { user_id, device_id })
    }
}

/// Left in a request's extensions by the admin API's audit layer, which runs
/// before any extractor and so cannot tell whose request it is. The extractor
/// fills it in once the request has proved to be an admin's, so a request
/// refused at authentication leaves it empty and is not recorded.
#[derive(Clone, Default)]
pub struct AdminWitness(Arc<OnceLock<(State, OwnedUserId)>>);

impl AdminWitness {
    /// The admin who acted, and the services to record it with, once the
    /// request authenticated.
    #[must_use]
    pub fn seen(&self) -> Option<&(State, OwnedUserId)> {
        self.0.get()
    }
}
