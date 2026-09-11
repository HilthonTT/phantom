use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as b64};
use phantom_core::{Err, Result};
use ring::signature::{EcdsaKeyPair, KeyPair};
use serde_json::{Value as JsonValue, json};

const UNCOMPRESSED_POINT_LEN: usize = 65;
const COORDINATE_LEN: usize = 32;

impl super::Server {
    #[inline]
    #[must_use]
    pub fn jwks(&self) -> JsonValue {
        json!({ "keys": [self.jwk.clone()] })
    }
}

pub(super) fn init_jwk(key_pair: &EcdsaKeyPair, key_id: &str) -> Result<JsonValue> {
    let point = key_pair.public_key().as_ref();

    if point.len() != UNCOMPRESSED_POINT_LEN || point[0] != 0x04 {
        return Err!(error!(
            "The OIDC signing key's public half is not an uncompressed P-256 point"
        ));
    }

    let (x, y) = point[1..].split_at(COORDINATE_LEN);

    Ok(json!({
        "kty": "EC",
        "crv": "P-256",
        "use": "sig",
        "alg": "ES256",
        "kid": key_id,
        "x": b64.encode(x),
        "y": b64.encode(y),
    }))
}
