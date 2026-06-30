//! Cryptographic primitives for PIN-authenticated pairing.
//!
//! Pairing uses SPAKE2 (a Password-Authenticated Key Exchange) with the
//! short PIN as the shared low-entropy password. Both sides derive an
//! identical session key *only* if the PINs match; an attacker without the
//! PIN cannot derive the key, so the channel is authenticated by the PIN
//! and is safe even against an active man-in-the-middle on the LAN.
//!
//! The raw SPAKE2 output is run through HKDF-SHA256 to derive a 256-bit
//! ChaCha20-Poly1305 key used to encrypt the provisioning payloads.

use anyhow::Context;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

/// Fixed identity string mixed into the SPAKE2 exchange. Both sides must agree.
const PAIRING_IDENTITY: &[u8] = b"supershare-pairing-v1";

/// HKDF info string for deriving the AEAD key from the SPAKE2 shared secret.
const HKDF_INFO: &[u8] = b"supershare-pairing-aead-key";

/// Nonce length for ChaCha20-Poly1305 (96-bit).
const NONCE_LEN: usize = 12;

/// One side of an in-progress SPAKE2 exchange.
///
/// Call [`start`](PairingExchange::start) with the PIN to get the first
/// protocol message to send to the peer, then [`finish`](PairingExchange::finish)
/// with the peer's message to obtain the shared [`SessionKey`].
pub struct PairingExchange {
    inner: Spake2<Ed25519Group>,
}

impl PairingExchange {
    /// Begin a symmetric SPAKE2 exchange using `pin` as the password.
    /// Returns the exchange handle and the outbound protocol message bytes.
    pub fn start(pin: &str) -> (Self, Vec<u8>) {
        let (inner, msg) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(pin.as_bytes()),
            &Identity::new(PAIRING_IDENTITY),
        );
        (Self { inner }, msg)
    }

    /// Complete the exchange using the peer's protocol message. Yields a
    /// session key that matches the peer's only if both used the same PIN.
    pub fn finish(self, peer_msg: &[u8]) -> anyhow::Result<SessionKey> {
        let shared = self
            .inner
            .finish(peer_msg)
            .map_err(|e| anyhow::anyhow!("SPAKE2 finish failed: {e:?}"))?;

        // Derive a fixed-size AEAD key from the (variable) shared secret.
        let hk = Hkdf::<Sha256>::new(None, &shared);
        let mut key = [0u8; 32];
        hk.expand(HKDF_INFO, &mut key)
            .map_err(|e| anyhow::anyhow!("HKDF expand failed: {e}"))?;
        Ok(SessionKey { key })
    }
}

/// A symmetric session key derived from a successful pairing exchange.
pub struct SessionKey {
    key: [u8; 32],
}

impl SessionKey {
    /// Encrypt `plaintext`, returning `(nonce, ciphertext)`. The nonce is
    /// randomly generated per call and must be sent alongside the ciphertext.
    pub fn seal(&self, plaintext: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("AEAD encrypt failed: {e}"))?;
        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    /// Decrypt a `(nonce, ciphertext)` pair produced by [`seal`](Self::seal).
    /// Fails if the key is wrong or the ciphertext/nonce was tampered with.
    pub fn open(&self, nonce: &[u8], ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
        if nonce.len() != NONCE_LEN {
            anyhow::bail!("invalid nonce length: {}", nonce.len());
        }
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let nonce = Nonce::from_slice(nonce);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("AEAD decrypt/authentication failed"))
            .context("provisioning payload could not be authenticated")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a full exchange and return both session keys.
    fn exchange(pin_a: &str, pin_b: &str) -> (anyhow::Result<SessionKey>, anyhow::Result<SessionKey>) {
        let (a, msg_a) = PairingExchange::start(pin_a);
        let (b, msg_b) = PairingExchange::start(pin_b);
        (a.finish(&msg_b), b.finish(&msg_a))
    }

    #[test]
    fn matching_pins_derive_equal_keys() {
        let (ka, kb) = exchange("123456", "123456");
        let ka = ka.expect("a finishes");
        let kb = kb.expect("b finishes");
        assert_eq!(ka.key, kb.key, "matching PINs must derive the same key");
    }

    #[test]
    fn mismatched_pins_derive_different_keys() {
        let (ka, kb) = exchange("123456", "654321");
        // SPAKE2 finish itself may succeed, but the derived keys must differ.
        let ka = ka.expect("a finishes");
        let kb = kb.expect("b finishes");
        assert_ne!(ka.key, kb.key, "mismatched PINs must not agree on a key");
    }

    #[test]
    fn aead_round_trip() {
        let (ka, _kb) = exchange("000111", "000111");
        let key = ka.unwrap();
        let (nonce, ct) = key.seal(b"hello provisioning").unwrap();
        let pt = key.open(&nonce, &ct).unwrap();
        assert_eq!(pt, b"hello provisioning");
    }

    #[test]
    fn aead_detects_tampering() {
        let (ka, _kb) = exchange("000111", "000111");
        let key = ka.unwrap();
        let (nonce, mut ct) = key.seal(b"sensitive cert bytes").unwrap();
        ct[0] ^= 0xFF; // flip a bit
        assert!(key.open(&nonce, &ct).is_err(), "tampered ciphertext must fail");
    }

    #[test]
    fn aead_keys_from_different_pins_cannot_decrypt() {
        let (ka, _) = exchange("111111", "111111");
        let (_, kb) = exchange("222222", "222222");
        let ka = ka.unwrap();
        let kb = kb.unwrap();
        let (nonce, ct) = ka.seal(b"secret").unwrap();
        assert!(kb.open(&nonce, &ct).is_err(), "wrong key must not decrypt");
    }
}
