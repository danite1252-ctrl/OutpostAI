//! Sealing secrets kept in the database (tools' keys and sign-ins): ChaCha20-Poly1305 with a
//! key of this Outpost's own, kept in `<data>/secrets.key` (not in the database, so a copy of
//! the database alone gives nothing away). Lose the key file and those secrets are gone:
//! people sign in again. Backups should keep it with the database.

use std::path::Path;

use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

pub struct Vault {
    key: LessSafeKey,
    rng: SystemRandom,
}

const PREFIX: &str = "v1:";

impl Vault {
    fn from_bytes(bytes: &[u8]) -> Self {
        let key = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, bytes).expect("a 32-byte key"));
        Self { key, rng: SystemRandom::new() }
    }

    /// This Outpost's vault: its key file, made on first use. If it can't be read or made,
    /// a key for this run only (secrets sealed now won't open after a restart), with a warning.
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("secrets.key");
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() == 32 {
                return Self::from_bytes(&bytes);
            }
            tracing::warn!(path = %path.display(), "secrets.key isn't a key; making a new one (saved sign-ins will need doing again)");
        }
        let mut bytes = [0u8; 32];
        SystemRandom::new().fill(&mut bytes).expect("random bytes");
        let saved = std::fs::create_dir_all(data_dir).and_then(|_| std::fs::write(&path, bytes));
        match saved {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
            }
            Err(e) => tracing::warn!(error = %e, "couldn't save secrets.key; tool keys won't survive a restart"),
        }
        Self::from_bytes(&bytes)
    }

    /// Seal text: `v1:` + base64 of nonce and ciphertext.
    pub fn seal(&self, plain: &str) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce).expect("random bytes");
        let mut data = plain.as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data)
            .expect("sealing");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&data);
        format!("{PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(out))
    }

    /// Open sealed text (`None`: not sealed with this key, or damaged).
    pub fn unseal(&self, sealed: &str) -> Option<String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(sealed.strip_prefix(PREFIX)?).ok()?;
        if raw.len() < NONCE_LEN {
            return None;
        }
        let (nonce, rest) = raw.split_at(NONCE_LEN);
        let mut data = rest.to_vec();
        let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
        let plain = self.key.open_in_place(nonce, Aad::empty(), &mut data).ok()?;
        String::from_utf8(plain.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_secrets_open_only_with_the_same_key() {
        let dir = std::env::temp_dir().join(format!("outpost-vault-{}", std::process::id()));
        let v = Vault::open(&dir);
        let s = v.seal("Bearer abc123");
        assert!(s.starts_with("v1:") && !s.contains("abc123"));
        assert_eq!(v.unseal(&s).as_deref(), Some("Bearer abc123"));
        // Each seal differs (fresh nonce).
        assert_ne!(v.seal("x"), v.seal("x"));
        // The same key file opens it again; another key doesn't; tampering fails.
        assert_eq!(Vault::open(&dir).unseal(&s).as_deref(), Some("Bearer abc123"));
        let other = std::env::temp_dir().join(format!("outpost-vault-other-{}", std::process::id()));
        assert!(Vault::open(&other).unseal(&s).is_none());
        // A character in the middle (the end may be padding bits).
        let mid = s.len() / 2;
        let c = s.as_bytes()[mid];
        let bad = format!("{}{}{}", &s[..mid], if c == b'A' { 'B' } else { 'A' }, &s[mid + 1..]);
        assert!(v.unseal(&bad).is_none());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }
}
