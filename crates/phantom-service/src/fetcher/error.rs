use std::fmt;

use phantom_core::err;
use ruma::OwnedServerName;
use smallvec::SmallVec;

pub(super) type Attempted = SmallVec<[OwnedServerName; 3]>;

#[derive(Clone, Debug)]
pub(super) enum Failure {
    NotFound { attempted: Attempted },

    NoCandidates,

    Cancelled,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCandidates => write!(f, "no candidate servers available"),
            Self::Cancelled => write!(f, "fetch cancelled"),
            Self::NotFound { attempted } => {
                write!(f, "event not found on any of {} servers", attempted.len())
            }
        }
    }
}

impl From<Failure> for phantom_core::Error {
    fn from(failure: Failure) -> Self {
        err!(Request(NotFound("{failure}")))
    }
}
