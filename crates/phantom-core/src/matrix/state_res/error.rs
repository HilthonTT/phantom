use serde_json::Error as JsonError;
use thiserror::Error;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    SerdeJson(#[from] JsonError),

    #[error("Unsupported room version: {0}")]
    Unsupported(String),

    #[error("Not found error: {0}")]
    NotFound(String),

    #[error("Invalid PDU: {0}")]
    InvalidPdu(String),
}
