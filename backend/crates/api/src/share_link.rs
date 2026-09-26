// Encodes/decodes the opaque token in a container's public View
// share-link (`GET /invites/{token}`, and `?share_token=` on the media
// read routes). The token is AES-256-GCM-encrypted JSON embedding
// `(container_id, share_link_id)` — see backend/AGENTS.md's "Design
// decisions already made" for the original sketch of this. It's
// deliberately stateless (no separate token-storage table): "rotate"
// works by changing `container.share_link_id` in Postgres, which makes
// every previously issued token's embedded id stop matching, even
// though it still decrypts successfully.
//
// The AES key is derived from `AUTH_SECRET` with a fixed context
// string (domain separation), not reused directly — see `new()`. This
// avoids needing a second secret in `.env` at the cost of the two
// concerns (session auth, share-link tokens) not being cryptographically
// independent if `AUTH_SECRET` ever leaked; acceptable for now, revisit
// if that stops being true.

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const NONCE_LEN: usize = 12;

#[derive(Debug, Serialize, Deserialize)]
struct SharePayload {
    container_id: Uuid,
    share_link_id: Uuid,
}

#[derive(Debug)]
pub struct InvalidShareToken;

pub struct ShareLinkCodec {
    cipher: Aes256Gcm,
}

impl ShareLinkCodec {
    pub fn new(auth_secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"remera-share-link-aes-key-v1:");
        hasher.update(auth_secret.as_bytes());
        let key_bytes: [u8; 32] = hasher.finalize().into();
        let key = Key::<Aes256Gcm>::from(key_bytes);
        Self {
            cipher: Aes256Gcm::new(&key),
        }
    }

    pub fn encode(&self, container_id: Uuid, share_link_id: Uuid) -> String {
        let payload = SharePayload {
            container_id,
            share_link_id,
        };
        // Only ever fails on a type that can't serialize (ours always can).
        let plaintext = serde_json::to_vec(&payload).expect("SharePayload always serializes");

        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        // Only fails on buffer-length overflow, not a real concern for
        // payloads this small.
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_ref())
            .expect("encrypting a small fixed-shape payload should not fail");

        let mut combined = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        combined.extend_from_slice(&nonce);
        combined.extend_from_slice(&ciphertext);
        URL_SAFE_NO_PAD.encode(combined)
    }

    pub fn decode(&self, token: &str) -> Result<(Uuid, Uuid), InvalidShareToken> {
        let combined = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| InvalidShareToken)?;

        if combined.len() <= NONCE_LEN {
            return Err(InvalidShareToken);
        }
        let (nonce_bytes, ciphertext) = combined.split_at(NONCE_LEN);
        let nonce_arr: [u8; NONCE_LEN] = nonce_bytes.try_into().map_err(|_| InvalidShareToken)?;
        let nonce = Nonce::from(nonce_arr);

        let plaintext = self
            .cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| InvalidShareToken)?;

        let payload: SharePayload =
            serde_json::from_slice(&plaintext).map_err(|_| InvalidShareToken)?;

        Ok((payload.container_id, payload.share_link_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_then_decode_round_trips() {
        let codec = ShareLinkCodec::new("test-secret");
        let container_id = Uuid::now_v7();
        let share_link_id = Uuid::now_v7();

        let token = codec.encode(container_id, share_link_id);
        let (decoded_container_id, decoded_share_link_id) =
            codec.decode(&token).expect("token should decode");

        assert_eq!(decoded_container_id, container_id);
        assert_eq!(decoded_share_link_id, share_link_id);
    }

    #[test]
    fn two_tokens_for_the_same_payload_are_different_ciphertext() {
        // AES-GCM uses a fresh random nonce every call — this is what
        // makes GET .../share-link idempotent at the (container_id,
        // share_link_id) level without ever returning the exact same
        // token string twice (see backend/AGENTS.md's "Built — View
        // share-link" notes).
        let codec = ShareLinkCodec::new("test-secret");
        let container_id = Uuid::now_v7();
        let share_link_id = Uuid::now_v7();

        let token_a = codec.encode(container_id, share_link_id);
        let token_b = codec.encode(container_id, share_link_id);

        assert_ne!(token_a, token_b);
        assert_eq!(
            codec.decode(&token_a).unwrap(),
            codec.decode(&token_b).unwrap()
        );
    }

    #[test]
    fn decoding_with_a_different_secret_fails() {
        let codec_a = ShareLinkCodec::new("secret-a");
        let codec_b = ShareLinkCodec::new("secret-b");

        let token = codec_a.encode(Uuid::now_v7(), Uuid::now_v7());

        assert!(codec_b.decode(&token).is_err());
    }

    #[test]
    fn decoding_a_tampered_token_fails() {
        let codec = ShareLinkCodec::new("test-secret");
        let token = codec.encode(Uuid::now_v7(), Uuid::now_v7());

        let mut bytes = URL_SAFE_NO_PAD.decode(&token).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF; // flip the last byte of the ciphertext/auth tag
        let tampered = URL_SAFE_NO_PAD.encode(bytes);

        assert!(codec.decode(&tampered).is_err());
    }

    #[test]
    fn decoding_garbage_input_fails_without_panicking() {
        let codec = ShareLinkCodec::new("test-secret");

        assert!(codec.decode("").is_err());
        assert!(codec.decode("not-valid-base64!!!").is_err());
        assert!(codec.decode("YQ").is_err()); // valid base64, too short to contain a nonce
    }

    #[test]
    fn rotating_the_share_link_id_invalidates_the_old_token() {
        // Mirrors what routes/containers.rs's rotate handler relies on:
        // decrypting still succeeds, but the embedded share_link_id no
        // longer matches what's stored, so the caller rejects it.
        let codec = ShareLinkCodec::new("test-secret");
        let container_id = Uuid::now_v7();
        let old_share_link_id = Uuid::now_v7();
        let new_share_link_id = Uuid::now_v7();

        let old_token = codec.encode(container_id, old_share_link_id);

        let (decoded_container_id, decoded_share_link_id) = codec.decode(&old_token).unwrap();
        assert_eq!(decoded_container_id, container_id);
        assert_ne!(decoded_share_link_id, new_share_link_id);
    }
}
