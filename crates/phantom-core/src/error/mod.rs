mod err;
mod log;
mod panic;
mod response;
mod serde;

use std::{any::Any, borrow::Cow, convert::Infallible, sync::PoisonError};

pub use self::{err::visit, log::*};

#[derive(thiserror::Error)]
pub enum Error {
    #[error("PANIC!")]
    PanicAny(Box<dyn Any + Send>),
    #[error("PANIC! {0}")]
    Panic(&'static str, Box<dyn Any + Send + 'static>),

    // std
    #[error(transparent)]
    Fmt(#[from] std::fmt::Error),
    #[error(transparent)]
    FromUtf8(#[from] std::string::FromUtf8Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    ParseFloat(#[from] std::num::ParseFloatError),
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)]
    Std(#[from] Box<dyn std::error::Error + Send>),
    #[error(transparent)]
    ThreadAccessError(#[from] std::thread::AccessError),
    #[error(transparent)]
    TryFromInt(#[from] std::num::TryFromIntError),
    #[error(transparent)]
    TryFromSlice(#[from] std::array::TryFromSliceError),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),

    // third-party
    #[error(transparent)]
    CapacityError(#[from] arrayvec::CapacityError),
    #[error(transparent)]
    Extension(#[from] axum::extract::rejection::ExtensionRejection),
    #[error(transparent)]
    Figment(Box<figment::error::Error>),
    #[error(transparent)]
    Http(#[from] http::Error),
    #[error(transparent)]
    HttpHeader(#[from] http::header::InvalidHeaderValue),
    #[error("Join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    JsParseInt(#[from] js_int::ParseIntError),
    #[error(transparent)]
    JsTryFromInt(#[from] js_int::TryFromIntError),
    #[error(transparent)]
    Path(#[from] axum::extract::rejection::PathRejection),
    #[error("Mutex poisoned: {0}")]
    Poison(Cow<'static, str>),
    #[error("Request error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeDe(Cow<'static, str>),
    #[error("{0}")]
    SerdeSer(Cow<'static, str>),
    #[error(transparent)]
    TomlDe(#[from] toml::de::Error),
    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
    #[error(transparent)]
    TracingReload(#[from] tracing_subscriber::reload::Error),
    #[error(transparent)]
    TypedHeader(#[from] axum_extra::typed_header::TypedHeaderRejection),

    // ruma/conduwuit
    #[error("Arithmetic operation failed: {0}")]
    Arithmetic(Cow<'static, str>),
    #[error("{code}: {msg}", code = .0.errcode(), msg = .1)]
    BadRequest(ruma::api::error::ErrorKind, &'static str), //TODO: remove
    #[error("{0}")]
    BadServerResponse(Cow<'static, str>),
    #[error(transparent)]
    CanonicalJson(#[from] ruma::CanonicalJsonError),
    #[error("There was a problem with the '{0}' directive in your configuration: {1}")]
    Config(&'static str, Cow<'static, str>),
    #[error("{0}")]
    Conflict(Cow<'static, str>), // This is only needed for when a room alias already exists
    #[error(transparent)]
    ContentDisposition(#[from] ruma::http_headers::ContentDispositionParseError),
    #[error("{0}")]
    Database(Cow<'static, str>),
    #[error("Feature '{0}' is not available on this server.")]
    FeatureDisabled(Cow<'static, str>),
    #[error("Remote server {0} responded with: {1}")]
    Federation(ruma::OwnedServerName, ruma::api::error::Error),
    #[error("{0} in {1}")]
    InconsistentRoomState(&'static str, ruma::OwnedRoomId),
    #[error(transparent)]
    IntoHttp(#[from] ruma::api::error::IntoHttpError),
    #[error(transparent)]
    Mxc(#[from] ruma::MxcUriError),
    #[error(transparent)]
    Mxid(#[from] ruma::IdParseError),
    #[error("{code}: {msg}", code = .0.errcode(), msg = .1)]
    Request(
        ruma::api::error::ErrorKind,
        Cow<'static, str>,
        http::StatusCode,
    ),
    #[error(transparent)]
    Ruma(#[from] ruma::api::error::Error),
    #[error(transparent)]
    SignaturesJson(#[from] ruma::signatures::JsonError),
    #[error(transparent)]
    SignaturesVerification(#[from] ruma::signatures::VerificationError),
    // This crate carries its own state-resolution implementation in
    // `matrix::state_res`; ruma's is not built.
    #[error(transparent)]
    StateRes(#[from] crate::matrix::state_res::Error),
    #[error("uiaa")]
    Uiaa(Box<ruma::api::client::uiaa::UiaaInfo>),

    // unique / untyped
    #[error("{0}")]
    Err(Cow<'static, str>),
}

impl Error {
    #[inline]
    #[must_use]
    pub fn from_errno() -> Self {
        Self::Io(std::io::Error::last_os_error())
    }

    //#[deprecated]
    pub fn bad_database(message: &'static str) -> Self {
        crate::err!(Database(error!("{message}")))
    }

    /// Sanitizes public-facing errors that can leak sensitive information.
    pub fn sanitized_message(&self) -> String {
        match self {
            Self::Database(..) => String::from("Database error occurred."),
            Self::Io(..) => String::from("I/O error occurred."),
            _ => self.message(),
        }
    }

    /// Generate the error message string.
    pub fn message(&self) -> String {
        match self {
            Self::Federation(origin, error) => format!("Answer from {origin}: {error}"),
            Self::Ruma(error) => response::ruma_error_message(error),
            _ => format!("{self}"),
        }
    }

    /// Returns the Matrix error code / error kind
    #[inline]
    pub fn kind(&self) -> ruma::api::error::ErrorKind {
        use ruma::api::error::ErrorKind::{Unknown, Unrecognized};

        match self {
            Self::Federation(_, error) | Self::Ruma(error) => {
                response::ruma_error_kind(error).clone()
            }
            Self::BadRequest(kind, ..) | Self::Request(kind, ..) => kind.clone(),
            // ruma 0.16 dropped `ErrorKind::FeatureDisabled`; M_UNRECOGNIZED is
            // the spec's code for an endpoint this server does not offer.
            Self::FeatureDisabled(..) => Unrecognized,
            _ => Unknown,
        }
    }

    /// Returns the HTTP error code or closest approximation based on error
    /// variant.
    pub fn status_code(&self) -> http::StatusCode {
        use http::StatusCode;

        match self {
            Self::Federation(_, error) | Self::Ruma(error) => error.status_code,
            Self::Request(kind, _, code) => response::status_code(kind, *code),
            Self::BadRequest(kind, ..) => response::bad_request_code(kind),
            Self::FeatureDisabled(..) => response::bad_request_code(&self.kind()),
            Self::Reqwest(error) => error.status().unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Io(error) => response::io_error_code(error.kind()),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Returns true for "not found" errors. This means anything that qualifies
    /// as a "not found" from any variant's contained error type. This call is
    /// often used as a special case to eliminate a contained Option with a
    /// Result where Ok(None) is instead Err(e) if e.is_not_found().
    #[inline]
    pub fn is_not_found(&self) -> bool {
        self.status_code() == http::StatusCode::NOT_FOUND
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

// Boxed above to keep `Error` small, so `?` needs these by hand — `#[from]`
// would demand the caller box it first.
impl From<figment::error::Error> for Error {
    #[cold]
    #[inline(never)]
    fn from(error: figment::error::Error) -> Self {
        Self::Figment(Box::new(error))
    }
}

impl From<ruma::api::client::uiaa::UiaaInfo> for Error {
    #[cold]
    #[inline(never)]
    fn from(info: ruma::api::client::uiaa::UiaaInfo) -> Self {
        Self::Uiaa(Box::new(info))
    }
}

impl<T> From<PoisonError<T>> for Error {
    #[cold]
    #[inline(never)]
    fn from(e: PoisonError<T>) -> Self {
        Self::Poison(e.to_string().into())
    }
}

#[allow(clippy::fallible_impl_from)]
impl From<Infallible> for Error {
    #[cold]
    #[inline(never)]
    fn from(_e: Infallible) -> Self {
        panic!("infallible error should never exist");
    }
}

#[cold]
#[inline(never)]
pub fn infallible(_e: &Infallible) {
    panic!("infallible error should never exist");
}

/// Convenience functor for fundamental Error::sanitized_message(); see member.
#[inline]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn sanitized_message(e: Error) -> String {
    e.sanitized_message()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Error` is returned by nearly every fallible function in the crate, so
    /// its size is the size of almost every `Result` phantom passes around.
    /// This pins it so a future variant that balloons it is noticed here.
    #[test]
    fn error_size_is_bounded() {
        let size = size_of::<Error>();
        assert!(
            size <= 112,
            "Error grew to {size} bytes; box the new variant"
        );
    }

    #[test]
    fn err_macro_builds_plain_and_formatted_messages() {
        let plain = crate::err!("something went wrong");
        assert_eq!(plain.message(), "something went wrong");

        let detail = "disk full";
        let formatted = crate::err!("something went wrong: {detail}");
        assert_eq!(formatted.message(), "something went wrong: disk full");
    }

    #[test]
    fn err_macro_scopes_variants() {
        let error = crate::err!(Database("table is missing"));
        assert!(matches!(error, Error::Database(..)), "{error:?}");
        assert_eq!(error.message(), "table is missing");
    }

    #[test]
    fn err_macro_logs_and_keeps_the_message() {
        // The `error!` form dispatches a tracing event *and* carries the same
        // string into the Error, which is the whole point of `err_log!`.
        let error = crate::err!(Database(error!("bad row {}", 7)));
        assert_eq!(error.message(), "bad row 7");
    }

    #[test]
    fn request_errors_carry_matrix_code_and_status() {
        use ruma::api::error::ErrorKind;

        let error = crate::err!(Request(Forbidden("you shall not pass")));
        assert_eq!(error.kind(), ErrorKind::Forbidden);
        assert_eq!(error.status_code(), http::StatusCode::FORBIDDEN);
        assert!(error.to_string().starts_with("M_FORBIDDEN:"), "{error}");
    }

    #[test]
    fn status_codes_map_per_matrix_spec() {
        use ruma::api::error::ErrorKind;

        let unauthorized = crate::err!(Request(MissingToken("no token")));
        assert_eq!(unauthorized.status_code(), http::StatusCode::UNAUTHORIZED);

        let not_found = Error::Request(
            ErrorKind::NotFound,
            "nope".into(),
            http::StatusCode::BAD_REQUEST,
        );
        assert_eq!(not_found.status_code(), http::StatusCode::NOT_FOUND);
        assert!(not_found.is_not_found());
    }

    #[test]
    fn sanitized_message_hides_internals() {
        let error = crate::err!(Database("connection string user:hunter2@host"));

        assert_eq!(error.sanitized_message(), "Database error occurred.");
        assert!(!error.sanitized_message().contains("hunter2"));
    }

    #[test]
    fn err_bang_returns_the_result_variant() {
        fn fallible() -> crate::Result<()> {
            crate::Err!(Database("nope"))
        }

        assert!(matches!(fallible(), Err(Error::Database(..))));
    }
}
