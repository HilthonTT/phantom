use ruma::{IdParseError, OwnedUserId, ServerName, UserId, api::appservice::Registration};

use super::NamespaceRegex;

/// A registration together with its namespaces compiled.
///
/// The registration is kept whole beside the compiled form: it is what is
/// written back to the database and handed to a caller that wants the fields
/// the patterns were built from, while every match goes through the three
/// [`NamespaceRegex`] below.
#[derive(Clone, Debug)]
pub struct RegistrationInfo {
    pub registration: Registration,
    pub users: NamespaceRegex,
    pub aliases: NamespaceRegex,
    pub rooms: NamespaceRegex,
}

impl RegistrationInfo {
    /// Whether the appservice may act as `user_id`.
    ///
    /// Its own sender is always its own, whether or not the user namespaces
    /// happen to cover it — the spec gives an appservice that user implicitly.
    #[inline]
    #[must_use]
    pub fn is_user_match(&self, user_id: &UserId) -> bool {
        self.is_sender(user_id) || self.users.is_match(user_id.as_str())
    }

    /// Whether the appservice claims `user_id` to the exclusion of everyone
    /// else, which is what stops the user being registered by hand.
    #[inline]
    #[must_use]
    pub fn is_exclusive_user_match(&self, user_id: &UserId) -> bool {
        self.is_sender(user_id) || self.users.is_exclusive_match(user_id.as_str())
    }

    /// Whether `user_id` is the user the appservice sends as.
    ///
    /// The localpart alone decides it. An appservice acts only for users on
    /// the server it is registered with, so a localpart that matches on any
    /// other server is not a user this can be asked about.
    #[inline]
    #[must_use]
    pub fn is_sender(&self, user_id: &UserId) -> bool {
        self.registration.sender_localpart == user_id.localpart()
    }

    /// The user the appservice sends as, on `server_name`.
    ///
    /// Fallible because `sender_localpart` is whatever the registration file
    /// said; a registration this server accepted has already been checked, so
    /// the error is only reachable for one that arrived some other way.
    #[inline]
    pub fn sender_user(&self, server_name: &ServerName) -> Result<OwnedUserId, IdParseError> {
        UserId::parse_with_server_name(self.registration.sender_localpart.as_str(), server_name)
    }
}

impl TryFrom<Registration> for RegistrationInfo {
    type Error = regex::Error;

    fn try_from(registration: Registration) -> Result<Self, Self::Error> {
        Ok(Self {
            users: registration.namespaces.users.as_slice().try_into()?,
            aliases: registration.namespaces.aliases.as_slice().try_into()?,
            rooms: registration.namespaces.rooms.as_slice().try_into()?,
            registration,
        })
    }
}
