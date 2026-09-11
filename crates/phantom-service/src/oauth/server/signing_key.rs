use phantom_core::{Result, err, info, rand};
use phantom_database::{Cbor, Deserialized};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair},
};
use serde::{Deserialize, Serialize};

use super::Data;

#[derive(Deserialize, Serialize)]
pub(super) struct SigningKey {
    pub(super) key_id: String,
    pub(super) key_der: Vec<u8>,
}

const SIGNING_KEY_DB_KEY: &str = "oidc_signing_key";

const KEY_ID_LENGTH: usize = 16;

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
