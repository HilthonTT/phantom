use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as b64};
use phantom_core::{Result, err, hash::sha256, implement};
use ring::{rand::SecureRandom, signature::EcdsaKeyPair};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct IdTokenClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub exp: u64,
    pub iat: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_hash: Option<String>,
}

#[derive(Serialize)]
struct Header<'a> {
    alg: &'static str,
    typ: &'static str,
    kid: &'a str,
}

#[implement(super::Server)]
pub fn sign_id_token(&self, claims: &IdTokenClaims) -> Result<String> {
    sign_compact(&self.key_pair, &self.rng, &self.key.key_id, claims)
}

fn sign_compact<T>(
    key_pair: &EcdsaKeyPair,
    rng: &dyn SecureRandom,
    key_id: &str,
    claims: &T,
) -> Result<String>
where
    T: Serialize,
{
    let header = Header {
        alg: "ES256",
        typ: "JWT",
        kid: key_id,
    };

    let mut token = b64.encode(serde_json::to_vec(&header)?);
    token.push('.');
    token.push_str(&b64.encode(serde_json::to_vec(claims)?));

    let signature = key_pair
        .sign(rng, token.as_bytes())
        .map_err(|e| err!(error!("Failed to sign an ID token: {e}")))?;

    token.push('.');
    token.push_str(&b64.encode(signature.as_ref()));

    Ok(token)
}

#[implement(super::Server)]
#[must_use]
#[inline]
pub fn at_hash(access_token: &str) -> String {
    let hash = sha256::hash(access_token.as_bytes());

    b64.encode(&hash[..16])
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use ring::{
        rand::SystemRandom,
        signature::{ECDSA_P256_SHA256_FIXED, KeyPair, UnparsedPublicKey},
    };
    use serde_json::Value as JsonValue;

    use super::{
        super::{
            jwk::init_jwk,
            signing_key::{generate_signing_key, load_key_pair},
        },
        IdTokenClaims, b64, sign_compact,
    };

    fn test_key_pair(rng: &SystemRandom) -> ring::signature::EcdsaKeyPair {
        let key = generate_signing_key().expect("generates a key");

        load_key_pair(&key.key_der, rng).expect("loads the key it just generated")
    }

    #[test]
    fn a_signed_id_token_verifies_against_the_published_jwk() {
        let rng = SystemRandom::new();
        let key_pair = test_key_pair(&rng);
        let jwk = init_jwk(&key_pair, "kid-1").expect("builds a JWK");

        let claims = IdTokenClaims {
            iss: "https://matrix.example.com/".to_owned(),
            sub: "@someone:example.com".to_owned(),
            aud: "client-id".to_owned(),
            exp: 1_757_003_600,
            iat: 1_757_000_000,
            nonce: Some("nonce".to_owned()),
            at_hash: None,
        };

        let token = sign_compact(&key_pair, &rng, "kid-1", &claims).expect("signs");

        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "a compact JWS has three segments");

        let header: JsonValue =
            serde_json::from_slice(&b64.decode(parts[0]).expect("base64url")).expect("JSON");
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "kid-1");

        let payload: JsonValue =
            serde_json::from_slice(&b64.decode(parts[1]).expect("base64url")).expect("JSON");
        assert_eq!(payload["sub"], "@someone:example.com");
        assert_eq!(payload["nonce"], "nonce");

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature = b64.decode(parts[2]).expect("base64url");
        assert_eq!(signature.len(), 64, "ES256 signs to a fixed r || s");

        let x = b64
            .decode(jwk["x"].as_str().expect("x"))
            .expect("base64url");
        let y = b64
            .decode(jwk["y"].as_str().expect("y"))
            .expect("base64url");
        let point: Vec<u8> = [&[0x04][..], &x, &y].concat();

        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &point)
            .verify(signing_input.as_bytes(), &signature)
            .expect("the published JWK verifies the token this server signed");

        assert_eq!(
            point,
            key_pair.public_key().as_ref(),
            "the JWK is the key pair's own public half"
        );
    }

    #[test]
    fn a_token_does_not_verify_under_a_different_key() {
        let rng = SystemRandom::new();
        let signer = test_key_pair(&rng);
        let other = test_key_pair(&rng);

        let claims = IdTokenClaims {
            iss: "https://matrix.example.com/".to_owned(),
            sub: "@someone:example.com".to_owned(),
            aud: "client-id".to_owned(),
            exp: 1_757_003_600,
            iat: 1_757_000_000,
            nonce: None,
            at_hash: None,
        };

        let token = sign_compact(&signer, &rng, "kid-1", &claims).expect("signs");
        let (signing_input, signature) = token.rsplit_once('.').expect("three segments");
        let signature = b64.decode(signature).expect("base64url");

        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, other.public_key().as_ref())
            .verify(signing_input.as_bytes(), &signature)
            .expect_err("a different key must not verify it");
    }
}
