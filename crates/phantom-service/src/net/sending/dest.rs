use phantom_core::runtime::metrics::SendClass;
use ruma::{OwnedServerName, OwnedUserId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
    Appservice(String),
    Push(OwnedUserId, String),
    Federation(OwnedServerName),
}

impl Destination {
    /// The destination's class, as the metrics count it.
    #[must_use]
    pub fn class(&self) -> SendClass {
        match self {
            Self::Appservice(_) => SendClass::Appservice,
            Self::Push(..) => SendClass::Push,
            Self::Federation(_) => SendClass::Federation,
        }
    }

    #[must_use]
    pub(super) fn get_prefix(&self) -> Vec<u8> {
        match self {
            Self::Federation(server) => {
                let len = server.as_bytes().len().saturating_add(1);
                let mut p = Vec::with_capacity(len);
                p.extend_from_slice(server.as_bytes());
                p.push(0xFF);
                p
            }
            Self::Appservice(server) => {
                let sigil = b"+";
                let len = sigil.len().saturating_add(server.len()).saturating_add(1);
                let mut p = Vec::with_capacity(len);
                p.extend_from_slice(sigil);
                p.extend_from_slice(server.as_bytes());
                p.push(0xFF);
                p
            }
            Self::Push(user, pushkey) => {
                let sigil = b"$";
                let len = sigil
                    .len()
                    .saturating_add(user.as_bytes().len())
                    .saturating_add(1)
                    .saturating_add(pushkey.len())
                    .saturating_add(1);
                let mut p = Vec::with_capacity(len);
                p.extend_from_slice(sigil);
                p.extend_from_slice(user.as_bytes());
                p.push(0xFF);
                p.extend_from_slice(pushkey.as_bytes());
                p.push(0xFF);
                p
            }
        }
    }
}
