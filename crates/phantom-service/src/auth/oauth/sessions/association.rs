use std::collections::BTreeMap;

use phantom_core::{debug, implement, trace};
use ruma::{OwnedUserId, UserId};
use serde_json::Value;

use super::{Sessions, UserInfo};

pub(super) type Pending = BTreeMap<String, Claimants>;

type Claimants = BTreeMap<OwnedUserId, Claims>;

pub type Claims = BTreeMap<String, String>;

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

#[implement(Sessions)]
pub fn remove_provider_associations_pending(&self, idp_id: &str) {
    self.association_pending
        .lock()
        .expect("locked")
        .remove(idp_id);
}

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

#[implement(Sessions)]
#[must_use]
pub fn is_user_association_pending(&self, user_id: &UserId) -> bool {
    self.association_pending
        .lock()
        .expect("locked")
        .values()
        .any(|claiming| claiming.contains_key(user_id))
}
