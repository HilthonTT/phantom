use ruma::{IdParseError, OwnedUserId, ServerName, UserId, api::appservice::Registration};

use super::NamespaceRegex;

#[derive(Clone, Debug)]
pub struct RegistrationInfo {
    pub registration: Registration,
    pub users: NamespaceRegex,
    pub aliases: NamespaceRegex,
    pub rooms: NamespaceRegex,
}

impl RegistrationInfo {
    #[inline]
    #[must_use]
    pub fn is_user_match(&self, user_id: &UserId) -> bool {
        self.is_sender(user_id) || self.users.is_match(user_id.as_str())
    }

    #[inline]
    #[must_use]
    pub fn is_exclusive_user_match(&self, user_id: &UserId) -> bool {
        self.is_sender(user_id) || self.users.is_exclusive_match(user_id.as_str())
    }

    #[inline]
    #[must_use]
    pub fn is_sender(&self, user_id: &UserId) -> bool {
        self.registration.sender_localpart == user_id.localpart()
    }

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
