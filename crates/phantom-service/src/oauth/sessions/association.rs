//! Binding a provider identity to an account that already exists.
//!
//! Normally a provider identity that has not been seen before registers a new
//! account. An association inverts that: an admin — or the user, through an
//! interactive-auth flow — says in advance which existing Matrix account the
//! *next* authorization matching a set of claims should bind to.
//!
//! It is deliberately in memory and nowhere else. A pending association is a
//! standing offer to hand an account to whoever authorizes next, which is not
//! something that should outlive the process that was asked for it.

use std::collections::BTreeMap;

use phantom_core::{debug, implement, trace};
use ruma::{OwnedUserId, UserId};
use serde_json::Value;

use super::{Sessions, UserInfo};

/// The pending associations of every provider.
pub(super) type Pending = BTreeMap<String, Claimants>;

/// Who is waiting to be associated at one provider, and on what claims.
type Claimants = BTreeMap<OwnedUserId, Claims>;

/// The userinfo claims an authorization has to match, as name and value.
///
/// Every one of them has to match. An empty set would match the first identity
/// to authorize at all, so it is never stored.
pub type Claims = BTreeMap<String, String>;

/// Offers `user_id` to the next authorization at `idp_id` matching `claims`.
///
/// Returns the claims this replaced, where the same account was already
/// waiting at this provider.
#[implement(Sessions)]
pub fn set_user_association_pending(
    &self,
    idp_id: &str,
    user_id: &UserId,
    claims: Claims,
) -> Option<Claims> {
    self.association_pending
        .lock()
        .expect("locked")
        .entry(idp_id.into())
        .or_default()
        .insert(user_id.into(), claims)
}

/// The account waiting to be associated with this identity, if any.
///
/// Every claim the account was registered under has to match what the provider
/// says. A claim the provider did not send does not match.
#[implement(Sessions)]
pub fn find_user_association_pending(
    &self,
    idp_id: &str,
    user_info: &UserInfo,
) -> Option<OwnedUserId> {
    let claiming = serde_json::to_value(user_info).expect("user_info is always serializable");

    let claiming = claiming
        .as_object()
        .expect("user_info serializes to an object");

    debug_assert!(
        !claiming.is_empty(),
        "user_info always carries at least the `sub` claim"
    );

    debug!(?idp_id, ?claiming, "finding pending association");

    self.association_pending
        .lock()
        .expect("locked")
        .get(idp_id)
        .into_iter()
        .flat_map(Claimants::iter)
        .find_map(|(user_id, claimant)| {
            trace!(?user_id, ?claimant, "checking against pending association");

            debug_assert!(
                !claimant.is_empty(),
                "an empty claim set would match anything and is never stored"
            );

            claimant
                .iter()
                .all(|(claim, value)| {
                    claiming.get(claim).and_then(Value::as_str) == Some(value.as_str())
                })
                .then(|| user_id.clone())
        })
}

/// Withdraws every association waiting at a provider.
#[implement(Sessions)]
pub fn remove_provider_associations_pending(&self, idp_id: &str) {
    self.association_pending
        .lock()
        .expect("locked")
        .remove(idp_id);
}

/// Withdraws an account's association, at one provider or at all of them.
#[implement(Sessions)]
pub fn remove_user_association_pending(&self, user_id: &UserId, idp_id: Option<&str>) {
    self.association_pending
        .lock()
        .expect("locked")
        .iter_mut()
        .filter(|(provider, _)| idp_id.is_none_or(|idp_id| idp_id == provider.as_str()))
        .for_each(|(_, claiming)| {
            claiming.remove(user_id);
        });
}

/// Whether this account is waiting to be associated anywhere.
#[implement(Sessions)]
#[must_use]
pub fn is_user_association_pending(&self, user_id: &UserId) -> bool {
    self.association_pending
        .lock()
        .expect("locked")
        .values()
        .any(|claiming| claiming.contains_key(user_id))
}
