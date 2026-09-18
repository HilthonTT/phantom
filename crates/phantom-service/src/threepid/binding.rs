use futures::{Stream, StreamExt};
use phantom_core::{Result, implement, stream::TryIgnore};
use phantom_database::{Cbor, Deserialized, Ignore, Interfix};
use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedUserId, UserId,
    thirdparty::{Medium, ThirdPartyIdentifier, ThirdPartyIdentifierInit},
};

use super::Binding;

#[implement(super::Service)]
#[tracing::instrument(
	level = "debug",
	skip(self),
	fields(
		%user_id,
	),
)]
pub async fn put_binding(
    &self,
    user_id: &UserId,
    email_canon: &str,
    medium: Medium,
    validated_at: MilliSecondsSinceUnixEpoch,
    added_at: MilliSecondsSinceUnixEpoch,
) -> Result {
    let binding = Binding {
        medium,
        validated_at,
        added_at,
    };

    self.db
        .userid_email
        .put((user_id, email_canon), Cbor(binding))?;

    self.db.email_userid.insert(email_canon, user_id)
}

#[implement(super::Service)]
#[tracing::instrument(
	level = "debug",
	skip(self),
	fields(
		%user_id,
	),
)]
pub fn get_bindings<'a>(
    &'a self,
    user_id: &'a UserId,
) -> impl Stream<Item = ThirdPartyIdentifier> + Send + 'a {
    type KeyVal = ((Ignore, String), Cbor<Binding>);

    self.db
        .userid_email
        .stream_prefix(&(user_id, Interfix))
        .ignore_err()
        .map(|((_, address), Cbor(binding)): KeyVal| {
            ThirdPartyIdentifierInit {
                address,
                medium: binding.medium,
                validated_at: binding.validated_at,
                added_at: binding.added_at,
            }
            .into()
        })
}

#[implement(super::Service)]
#[tracing::instrument(
	level = "debug",
	skip(self),
	fields(
		%user_id,
	),
)]
pub async fn del_binding(&self, user_id: &UserId, email_canon: &str) -> Result {
    self.db.userid_email.del((user_id, email_canon))?;

    if self
        .user_id_for_email(email_canon)
        .await?
        .is_some_and(|bound| bound == user_id)
    {
        self.db.email_userid.remove(email_canon)?;
    }

    Ok(())
}

#[implement(super::Service)]
#[tracing::instrument(
	level = "debug",
	skip(self),
	fields(
		%user_id,
	),
)]
pub async fn bound_elsewhere(&self, user_id: &UserId, email_canon: &str) -> Result<bool> {
    self.user_id_for_email(email_canon)
        .await
        .map(|bound| bound.is_some_and(|bound| bound != user_id))
}

#[implement(super::Service)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn user_id_for_email(&self, email_canon: &str) -> Result<Option<OwnedUserId>> {
    match self.db.email_userid.get(email_canon).await {
        Ok(handle) => handle.deserialized().map(Some),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error),
    }
}

#[implement(super::Service)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn address_in_use(&self, email_canon: &str) -> bool {
    self.db.email_userid.get(email_canon).await.is_ok()
}
