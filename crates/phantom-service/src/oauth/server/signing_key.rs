//! The key this server signs its own ID tokens with.
//!
//! Generated on first use and kept in `oidc_signingkey` from then on. It has
//! to survive restarts: a client that fetched the JWKS holds the public half,
//! and a key regenerated under it invalidates every token already issued.
//!
//! ECDSA on P-256, which is `ES256` — the algorithm every OpenID Connect
//! client is required to implement, so it is the one choice that never needs
//! negotiating.

use phantom_core::{Result, err, info, rand};
use phantom_database::{Cbor, Deserialized};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair},
};
use serde::{Deserialize, Serialize};

use super::Data;

/// The signing key as it is stored: the PKCS#8 private key and the id clients
/// select it by in the JWKS.
#[derive(Deserialize, Serialize)]
pub(super) struct SigningKey {
    pub(super) key_id: String,
    pub(super) key_der: Vec<u8>,
}

/// The single key in `oidc_signingkey`. The column holds one entry; the name
/// is what it is stored under.
const SIGNING_KEY_DB_KEY: &str = "oidc_signing_key";

/// Characters in a key id. Not a secret — it is published in the JWKS — only
/// distinct enough that a rotated key never collides with the one it replaced.
const KEY_ID_LENGTH: usize = 16;

/// Loads the signing key, generating and storing one if there is none.
///
/// Runs during service construction, before there is a runtime to defer to,
/// which is why the read blocks.
pub(super) fn init_signing_key(db: &Data) -> Result<SigningKey> {
    if let Ok(Cbor(key)) = db
        .oidc_signingkey
        .get_blocking(SIGNING_KEY_DB_KEY)
        .and_then(Deserialized::deserialized::<Cbor<SigningKey>>)
    {
        info!(key_id = ?key.key_id, "Loaded the existing OIDC signing key");

        return Ok(key);
    }

    let key = generate_signing_key()?;

    db.oidc_signingkey.raw_put(SIGNING_KEY_DB_KEY, Cbor(&key))?;

    info!(key_id = ?key.key_id, "Generated a new OIDC signing key");

    Ok(key)
}

/// Parses a stored key back into something that can sign.
pub(super) fn load_key_pair(key_der: &[u8], rng: &SystemRandom) -> Result<EcdsaKeyPair> {
    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, key_der, rng)
        .map_err(|e| err!(error!("Failed to load the OIDC signing key: {e}")))
}

pub(super) fn generate_signing_key() -> Result<SigningKey> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
        .map_err(|e| err!(error!("Failed to generate an ECDSA key: {e}")))?;

    Ok(SigningKey {
        key_id: rand::string(KEY_ID_LENGTH),
        key_der: pkcs8.as_ref().to_vec(),
    })
}
