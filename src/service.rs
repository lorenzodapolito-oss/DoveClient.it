//! Operazioni di alto livello: sincronizzazione, ricerca della configurazione, importazione.

use crate::account::Account;
use crate::db::Db;
use crate::imap::Imap;
use crate::net::Security;
use crate::providers::{self, ServerCfg};
use crate::smtp;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Massimo di messaggi scaricati per cartella (i più recenti).
pub const MAX_PER_FOLDER: usize = 20_000;
const BATCH: usize = 40;
const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

/// Cartelle virtuali (es. Gmail «Tutti i messaggi») che duplicherebbero i messaggi.
const SKIP_FOLDERS: &[&str] = &[
    "all mail",
    "important",
    "starred",
    "tutti i messaggi",
    "in evidenza",
];

pub fn should_skip_folder(name: &str) -> bool {
    let l = name.to_lowercase();
    SKIP_FOLDERS.iter().any(|p| l.contains(p))
}

pub fn imap_security(ssl: bool) -> Security {
    if ssl {
        Security::Ssl
    } else {
        Security::StartTls
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub new_messages: usize,
    pub folders: usize,
    pub errors: Vec<String>,
}

/// Scarica i messaggi nuovi di un account. Non riscarica quelli già presenti.
pub fn sync_account(
    acc: &Account,
    db: &Db,
    progress: &mut dyn FnMut(String),
    cancel: &AtomicBool,
) -> SyncReport {
    let mut rep = SyncReport::default();
    let label = acc.email.clone();
    progress(format!("Connessione a {label}…"));
    let mut imap = match Imap::connect(
        &acc.imap_server,
        acc.imap_port,
        imap_security(acc.imap_ssl),
        SYNC_TIMEOUT,
    ) {
        Ok(i) => i,
        Err(e) => {
            rep.errors.push(format!("{label}: {e}"));
            return rep;
        }
    };
    if let Err(e) = imap.login(&acc.email, &acc.password) {
        rep.errors.push(format!("{label}: {e}"));
        return rep;
    }
    let folders = match imap.list_folders() {
        Ok(f) if !f.is_empty() => f,
        _ => vec![crate::imap::FolderName {
            raw: "INBOX".into(),
            display: "INBOX".into(),
        }],
    };
    for f in folders {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if should_skip_folder(&f.display) {
            continue;
        }
        match sync_folder(&mut imap, acc, db, &f.raw, &f.display, progress, cancel) {
            Ok(n) => {
                rep.folders += 1;
                rep.new_messages += n;
            }
            Err(e) => rep.errors.push(format!("{label}/{}: {e}", f.display)),
        }
    }
    imap.logout();
    rep
}

fn sync_folder(
    imap: &mut Imap,
    acc: &Account,
    db: &Db,
    raw: &str,
    display: &str,
    progress: &mut dyn FnMut(String),
    cancel: &AtomicBool,
) -> Result<usize, String> {
    let sel = imap.examine(raw)?;
    if db.check_uidvalidity(&acc.email, raw, &sel.uidvalidity)? {
        progress(format!(
            "{display}: la cartella è cambiata sul server, riscarico tutto"
        ));
    }
    let known = db.known_uids(&acc.email, raw)?;
    let mut new: Vec<u32> = imap
        .uid_search_all()?
        .into_iter()
        .filter(|u| !known.contains(&u.to_string()))
        .collect();
    // I più recenti per primi, con un tetto massimo.
    new.reverse();
    new.truncate(MAX_PER_FOLDER);
    let total = new.len();
    if total == 0 {
        return Ok(0);
    }
    let mut saved = 0;
    for chunk in new.chunks(BATCH) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        for (uid, data) in imap.fetch_many(chunk)? {
            db.save_email(
                &acc.email,
                raw,
                &uid.to_string(),
                &data,
                &sel.uidvalidity,
                acc.is_pec,
            )?;
            saved += 1;
        }
        progress(format!(
            "{} / {display}: {saved}/{total} messaggi nuovi",
            acc.email
        ));
    }
    Ok(saved)
}

/// Prova IMAP (accesso) con la configurazione data.
pub fn test_imap(
    cfg: &ServerCfg,
    email: &str,
    password: &str,
    timeout: Duration,
) -> Result<(), String> {
    let mut i = Imap::connect(
        &cfg.imap_server,
        cfg.imap_port,
        imap_security(cfg.imap_ssl),
        timeout,
    )?;
    i.login(email, password)?;
    i.logout();
    Ok(())
}

pub fn test_smtp(cfg: &ServerCfg, email: &str, password: &str) -> Result<(), String> {
    let mut a = Account::new(email, password, cfg);
    a.email = email.to_string();
    smtp::test(&a)
}

/// Cerca la configurazione che funziona davvero (accesso IMAP e SMTP riusciti).
/// Ogni tentativo viene scritto con `log`.
pub fn find_working_config(
    email: &str,
    password: &str,
    current: Option<&ServerCfg>,
    log: &mut dyn FnMut(String),
    cancel: &AtomicBool,
) -> Option<ServerCfg> {
    let list = providers::candidates(email, current, true);
    let total = list.len();
    log(format!("{total} configurazioni da provare"));
    let mut imap_cache: HashMap<(String, u16, bool), Result<(), String>> = HashMap::new();
    let mut smtp_cache: HashMap<(String, u16, bool), Result<(), String>> = HashMap::new();
    for (i, (cfg, src)) in list.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        log(format!("[{}/{total}] {src}: {}", i + 1, cfg.describe()));
        let ik = (cfg.imap_server.clone(), cfg.imap_port, cfg.imap_ssl);
        let ir = imap_cache
            .entry(ik)
            .or_insert_with(|| test_imap(&cfg, email, password, Duration::from_secs(8)))
            .clone();
        if let Err(e) = ir {
            log(format!("   IMAP non riuscito: {e}"));
            continue;
        }
        log("   IMAP ok".into());
        let sk = (cfg.smtp_server.clone(), cfg.smtp_port, cfg.smtp_ssl);
        let sr = smtp_cache
            .entry(sk)
            .or_insert_with(|| test_smtp(&cfg, email, password))
            .clone();
        if let Err(e) = sr {
            log(format!("   SMTP non riuscito: {e}"));
            continue;
        }
        log("   SMTP ok".into());
        return Some(cfg);
    }
    None
}

/// Importa i file .eml della cartella (account «INPUT») e li elimina. Restituisce quanti.
pub fn import_dir(dir: &Path, db: &Db) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| !x.eq_ignore_ascii_case("eml")) {
            continue;
        }
        let Ok(data) = std::fs::read(&p) else {
            continue;
        };
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let uid = format!("input_{name}_{}", crate::db::now_ts());
        if db
            .save_email("INPUT", "INBOX", &uid, &data, "", false)
            .is_ok()
        {
            let _ = std::fs::remove_file(&p);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::tests::mock_server;
    use crate::mime::tests::SAMPLE;

    #[test]
    fn skips_virtual_folders() {
        assert!(
            should_skip_folder("[Gmail]/All Mail")
                && should_skip_folder("[Gmail]/Tutti i messaggi")
        );
        assert!(!should_skip_folder("INBOX") && !should_skip_folder("Posta inviata"));
    }

    #[test]
    fn import_dir_moves_eml_into_db() {
        let d = std::env::temp_dir().join(format!("dove_i_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("uno.eml"), SAMPLE).unwrap();
        std::fs::write(d.join("due.EML"), b"Subject: due\r\n\r\nx").unwrap();
        std::fs::write(d.join("nota.txt"), b"x").unwrap();
        let db = Db::open_memory().unwrap();
        assert_eq!(import_dir(&d, &db), 2);
        assert!(!d.join("uno.eml").exists() && d.join("nota.txt").exists());
        assert_eq!(db.known_uids("INPUT", "INBOX").unwrap().len(), 2);
        assert_eq!(import_dir(&d.join("manca"), &db), 0);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn incremental_sync_against_mock() {
        let msgs = vec![
            (5u32, SAMPLE.as_bytes().to_vec()),
            (9u32, b"Subject: due\r\n\r\ncorpo".to_vec()),
        ];
        let db = Db::open_memory().unwrap();
        // Il server finto serve una sola connessione: la sincronizzazione parte con SSL=false → StartTls,
        // quindi la proviamo direttamente sulle funzioni interne in modalità Plain.
        let port = mock_server(msgs);
        let mut imap =
            Imap::connect("127.0.0.1", port, Security::Plain, Duration::from_secs(5)).unwrap();
        imap.login("u", "buona").unwrap();
        let acc = Account::new(
            "u@x.it",
            "buona",
            &providers::known_config("a@libero.it").unwrap().0,
        );
        let mut msgs_log = Vec::new();
        let n = sync_folder(
            &mut imap,
            &acc,
            &db,
            "INBOX",
            "INBOX",
            &mut |m| msgs_log.push(m),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(n, 2);
        assert!(msgs_log.last().unwrap().contains("2/2"));
        assert_eq!(db.known_uids("u@x.it", "INBOX").unwrap().len(), 2);
    }

    #[test]
    fn sync_reports_unreachable_server() {
        let db = Db::open_memory().unwrap();
        let mut acc = Account::new(
            "u@x.it",
            "p",
            &providers::known_config("a@libero.it").unwrap().0,
        );
        acc.imap_server = "127.0.0.1".into();
        acc.imap_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        acc.imap_ssl = false;
        let r = sync_account(&acc, &db, &mut |_| {}, &AtomicBool::new(false));
        assert_eq!(r.new_messages, 0);
        assert_eq!(r.errors.len(), 1);
    }
}
