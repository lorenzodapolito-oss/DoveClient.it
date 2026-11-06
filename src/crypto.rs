//! Cifratura delle password salvate (ChaCha20-Poly1305 con chiave casuale in un file locale).

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use std::path::Path;

const PREFIX: &str = "enc1:";

pub struct Crypto {
    key: [u8; 32],
}

impl Crypto {
    /// Legge la chiave dal file, oppure ne crea una nuova.
    pub fn load_or_create(path: &Path) -> Result<Crypto, String> {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(bytes) = B64.decode(text.trim()) {
                if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
                    return Ok(Crypto { key });
                }
            }
        }
        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).map_err(|e| format!("generatore casuale: {e}"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cartella dati: {e}"))?;
        }
        std::fs::write(path, B64.encode(key)).map_err(|e| format!("scrittura chiave: {e}"))?;
        Ok(Crypto { key })
    }

    #[cfg(test)]
    pub fn from_key(key: [u8; 32]) -> Crypto {
        Crypto { key }
    }

    pub fn encrypt(&self, plain: &str) -> Result<String, String> {
        if plain.is_empty() {
            return Ok(String::new());
        }
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let mut nonce = [0u8; 12];
        getrandom::getrandom(&mut nonce).map_err(|e| format!("generatore casuale: {e}"))?;
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
            .map_err(|_| "cifratura non riuscita".to_string())?;
        let mut all = nonce.to_vec();
        all.extend(ct);
        Ok(format!("{PREFIX}{}", B64.encode(all)))
    }

    /// Decifra; un valore senza prefisso è considerato testo in chiaro (file vecchi).
    /// Un valore cifrato con un'altra chiave (o di un altro programma) dà errore.
    pub fn decrypt(&self, value: &str) -> Result<String, String> {
        let Some(b64) = value.strip_prefix(PREFIX) else {
            if value.starts_with("gAAAA") {
                return Err("password cifrata dal vecchio programma: va reinserita".into());
            }
            return Ok(value.to_string());
        };
        let all = B64
            .decode(b64)
            .map_err(|_| "password danneggiata".to_string())?;
        if all.len() < 13 {
            return Err("password danneggiata".into());
        }
        let (nonce, ct) = all.split_at(12);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| "password non decifrabile con questa chiave".to_string())?;
        String::from_utf8(plain).map_err(|_| "password danneggiata".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper() {
        let c = Crypto::from_key([7; 32]);
        let e = c.encrypt("segreto è ✓").unwrap();
        assert!(e.starts_with("enc1:") && !e.contains("segreto"));
        assert_eq!(c.decrypt(&e).unwrap(), "segreto è ✓");
        assert_ne!(
            e,
            c.encrypt("segreto è ✓").unwrap(),
            "nonce diverso a ogni volta"
        );
        let mut bad = e.clone();
        bad.push('A');
        assert!(c.decrypt(&bad).is_err() || c.decrypt(&bad).unwrap() != "x");
        assert!(Crypto::from_key([8; 32]).decrypt(&e).is_err());
        assert!(c.decrypt("enc1:!!!").is_err());
        assert!(c.decrypt("enc1:AAAA").is_err());
    }

    #[test]
    fn plaintext_legacy_and_fernet() {
        let c = Crypto::from_key([1; 32]);
        assert_eq!(c.decrypt("vecchia").unwrap(), "vecchia");
        assert_eq!(c.encrypt("").unwrap(), "");
        assert!(c.decrypt("gAAAAABk123").is_err());
    }

    #[test]
    fn key_file_is_created_once() {
        let d = std::env::temp_dir().join(format!("dove_k_{}", std::process::id()));
        let p = d.join("sub/.key");
        let a = Crypto::load_or_create(&p).unwrap();
        let e = a.encrypt("x").unwrap();
        let b = Crypto::load_or_create(&p).unwrap();
        assert_eq!(b.decrypt(&e).unwrap(), "x");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
