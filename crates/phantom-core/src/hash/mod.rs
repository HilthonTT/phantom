//! Password hashing and content digests.
//!
//! Passwords go through [`argon`], which is a deliberate cost; content hashes
//! go through [`sha256`], which is not. Do not reach for one where the other
//! belongs.

mod argon;
pub mod sha256;

use crate::Result;

pub fn verify_password(password: &str, password_hash: &str) -> Result {
    argon::verify_password(password, password_hash)
}

pub fn password(password: &str) -> Result<String> {
    argon::password(password)
}
