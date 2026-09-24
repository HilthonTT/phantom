use std::sync::Arc;

use phantom_core::{Result, debug_info, err, error, rand};
use phantom_database::Database;
use ruma::{api::federation::discovery::VerifyKey, serde::Base64, signatures::Ed25519KeyPair};

use super::VerifyKeys;

pub(super) fn init(db: &Arc<Database>) -> Result<(Box<Ed25519KeyPair>, VerifyKeys)> {
    let keypair = load(db).inspect_err(|_e| {
        error!("Keypair invalid. Deleting...");
        remove(db);
    })?;

    let verify_key = VerifyKey::new(Base64::new(keypair.public_key().to_vec()));

    let id = format!("ed25519:{}", keypair.version());
    let verify_keys: VerifyKeys = [(id.try_into()?, verify_key)].into();

    Ok((keypair, verify_keys))
}

fn load(db: &Arc<Database>) -> Result<Box<Ed25519KeyPair>> {
    let (version, der) = match db["global"].get_blocking(b"keypair") {
        Ok(handle) => {
            let (version, der): (&str, &[u8]) = handle.de()?;
            (version.to_owned(), der.to_vec())
        }
        Err(e) if e.is_not_found() => create(db)?,
        Err(e) => return Err(e),
    };

    let keypair = Ed25519KeyPair::from_der(&der, version)
        .map_err(|e| err!("Failed to load ed25519 keypair: {e:?}"))?;

    Ok(Box::new(keypair))
}

fn create(db: &Arc<Database>) -> Result<(String, Vec<u8>)> {
    let keypair = Ed25519KeyPair::generate();

    let id = rand::string(8);
    debug_info!("Generated new ED25519 keypair: {id:?}");

    let value: (String, Vec<u8>) = (id, keypair.to_vec());
    db["global"].raw_put(b"keypair", &value)?;

    Ok(value)
}

#[inline]
fn remove(db: &Arc<Database>) {
    let global = &db["global"];
    global.remove(b"keypair").ok();
}
