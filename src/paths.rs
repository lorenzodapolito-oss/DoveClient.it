//! Cartelle dei dati (accanto all'eseguibile) e impostazioni salvate.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Dirs {
    pub data: PathBuf,
    pub accounts: PathBuf,
    pub attachments: PathBuf,
    pub input: PathBuf,
    pub export: PathBuf,
    pub templates: PathBuf,
}

impl Dirs {
    pub fn new(base: &Path) -> Result<Dirs, String> {
        let d = Dirs {
            data: base.join("data"),
            accounts: base.join("accounts"),
            attachments: base.join("attachments"),
            input: base.join("input"),
            export: base.join("export"),
            templates: base.join("templates"),
        };
        for p in [
            &d.data,
            &d.accounts,
            &d.attachments,
            &d.input,
            &d.export,
            &d.templates,
        ] {
            std::fs::create_dir_all(p)
                .map_err(|e| format!("impossibile creare {}: {e}", p.display()))?;
        }
        Ok(d)
    }

    /// Cartella dell'eseguibile (o la cartella corrente se non è nota).
    pub fn default_base() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn db_file(&self) -> PathBuf {
        self.data.join("doveclient.db")
    }
    pub fn key_file(&self) -> PathBuf {
        self.data.join(".secret.key")
    }
    pub fn config_file(&self) -> PathBuf {
        self.data.join("config.json")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub dark_mode: bool,
    pub list_limit: usize,
    /// Minuti tra un aggiornamento automatico e l'altro (0 = mai).
    pub auto_refresh_minutes: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            dark_mode: false,
            list_limit: 200,
            auto_refresh_minutes: 5,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        let mut c: Config = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        c.list_limit = c.list_limit.clamp(50, 50_000);
        c.auto_refresh_minutes = c.auto_refresh_minutes.min(240);
        c
    }

    pub fn save(&self, path: &Path) {
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_created_and_config_roundtrip() {
        let b = std::env::temp_dir().join(format!("dove_p_{}", std::process::id()));
        let d = Dirs::new(&b).unwrap();
        assert!(d.accounts.is_dir() && d.templates.is_dir() && d.input.is_dir());
        let mut c = Config::load(&d.config_file());
        assert!(!c.dark_mode && c.list_limit == 200);
        c.dark_mode = true;
        c.list_limit = 1;
        c.save(&d.config_file());
        let c2 = Config::load(&d.config_file());
        assert!(c2.dark_mode);
        assert_eq!(
            c2.list_limit, 50,
            "valori fuori scala vengono riportati nei limiti"
        );
        std::fs::write(d.config_file(), "{rotto").unwrap();
        assert!(!Config::load(&d.config_file()).dark_mode);
        std::fs::remove_dir_all(&b).unwrap();
    }
}
