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
