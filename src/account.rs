//! Account di posta: struttura e archivio (un file JSON per account, password cifrata).

use crate::crypto::Crypto;
use crate::providers::ServerCfg;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    #[serde(default)]
    pub password: String,
    pub imap_server: String,
    #[serde(default = "d993")]
    pub imap_port: u16,
    #[serde(default = "yes", alias = "use_imap_ssl")]
    pub imap_ssl: bool,
    pub smtp_server: String,
    #[serde(default = "d465")]
    pub smtp_port: u16,
    #[serde(default = "yes", alias = "use_ssl")]
    pub smtp_ssl: bool,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub is_pec: bool,
}

fn d993() -> u16 {
    993
}
fn d465() -> u16 {
    465
}
fn yes() -> bool {
    true
}

impl Account {
    pub fn new(email: &str, password: &str, cfg: &ServerCfg) -> Account {
        let email = email.trim().to_lowercase();
        Account {
            is_pec: email.contains("pec"),
            email,
            password: password.to_string(),
            imap_server: cfg.imap_server.clone(),
            imap_port: cfg.imap_port,
            imap_ssl: cfg.imap_ssl,
            smtp_server: cfg.smtp_server.clone(),
            smtp_port: cfg.smtp_port,
            smtp_ssl: cfg.smtp_ssl,
            display_name: String::new(),
        }
    }

    pub fn file_name(&self) -> String {
        let safe: String = self
            .email
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!("{safe}.json")
    }
}

pub struct AccountStore {
    dir: PathBuf,
    crypto: Crypto,
}

impl AccountStore {
    pub fn new(dir: &Path, crypto: Crypto) -> AccountStore {
        AccountStore {
            dir: dir.to_path_buf(),
            crypto,
        }
    }

    /// Carica tutti gli account, in ordine alfabetico. Restituisce anche gli avvisi (file illeggibili, password da reinserire).
    pub fn load_all(&self) -> (Vec<Account>, Vec<String>) {
        let mut out = Vec::new();
        let mut warns = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return (out, warns);
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Ok(text) = std::fs::read_to_string(&p) else {
                warns.push(format!("{name}: file non leggibile"));
                continue;
            };
            let mut acc: Account = match serde_json::from_str(&text) {
                Ok(a) => a,
                Err(e) => {
                    warns.push(format!("{name}: {e}"));
                    continue;
                }
            };
            acc.email = acc.email.trim().to_lowercase();
            match self.crypto.decrypt(&acc.password) {
                Ok(p) => acc.password = p,
                Err(e) => {
                    warns.push(format!("{}: {e}", acc.email));
                    acc.password.clear();
                }
            }
            out.push(acc);
        }
        out.sort_by(|a, b| a.email.cmp(&b.email));
        (out, warns)
    }

    pub fn save(&self, acc: &Account) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("cartella account: {e}"))?;
        let mut stored = acc.clone();
        stored.password = self.crypto.encrypt(&acc.password)?;
        let json = serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?;
        let path = self.dir.join(acc.file_name());
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("scrittura: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("scrittura: {e}"))
    }

    pub fn delete(&self, acc: &Account) -> Result<(), String> {
        std::fs::remove_file(self.dir.join(acc.file_name()))
            .map_err(|e| format!("eliminazione: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ServerCfg {
        crate::providers::known_config("a@libero.it").unwrap().0
    }

    #[test]
    fn save_load_delete_with_encrypted_password() {
        let d = std::env::temp_dir().join(format!("dove_a_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let store = AccountStore::new(&d, Crypto::from_key([3; 32]));
        let a = Account::new("  Mario.Rossi@Libero.IT ", "pw-segreta", &cfg());
        assert_eq!(a.email, "mario.rossi@libero.it");
        store.save(&a).unwrap();
        let raw = std::fs::read_to_string(d.join(a.file_name())).unwrap();
        assert!(!raw.contains("pw-segreta"));
        let (all, warns) = store.load_all();
        assert!(warns.is_empty());
        assert_eq!(all, vec![a.clone()]);
        // Stessa casella scritta con maiuscole diverse = stesso file.
        store
            .save(&Account::new("MARIO.ROSSI@libero.it", "x", &cfg()))
            .unwrap();
        assert_eq!(store.load_all().0.len(), 1);
        store.delete(&a).unwrap();
        assert!(store.load_all().0.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn old_python_files_and_bad_files() {
        let d = std::env::temp_dir().join(format!("dove_o_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("a.json"),
            r#"{"email":"A@x.it","password":"gAAAAAB123","imap_server":"imap.x.it","smtp_server":"smtp.x.it","use_ssl":false,"use_imap_ssl":true}"#,
        )
        .unwrap();
        std::fs::write(d.join("b.json"), "non json").unwrap();
        std::fs::write(d.join("c.txt"), "ignorato").unwrap();
        let (all, warns) = AccountStore::new(&d, Crypto::from_key([1; 32])).load_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].email, "a@x.it");
        assert!(!all[0].smtp_ssl && all[0].imap_ssl && all[0].password.is_empty());
        assert_eq!(warns.len(), 2);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn pec_detected() {
        let a = Account::new("x@legalmail.it", "p", &cfg());
        assert!(!a.is_pec);
        let a = Account::new("x@pec.it", "p", &cfg());
        assert!(a.is_pec);
    }
}
