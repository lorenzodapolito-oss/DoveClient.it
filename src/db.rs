//! Archivio SQLite delle email: messaggi, allegati, UIDVALIDITY e ricerca full-text.

use crate::mime::{self, ParsedMail};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

/// Riga dell'elenco messaggi.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmailRow {
    pub id: i64,
    pub account: String,
    pub folder: String,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub date: String,
    pub timestamp: i64,
    pub has_attachments: bool,
    pub is_read: bool,
    pub is_deleted: bool,
    pub is_starred: bool,
    pub is_pec: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmailFull {
    pub row: EmailRow,
    pub body: String,
    pub html: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentInfo {
    pub id: i64,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub account: Option<String>,
    pub folder: Option<String>,
    /// La cartella deve contenere una di queste parole (senza maiuscole).
    pub folder_keywords: Vec<String>,
    pub starred: Option<bool>,
    pub deleted: Option<bool>,
    pub pec: Option<bool>,
    pub unread: Option<bool>,
    pub search: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderStat {
    pub account: String,
    pub folder: String,
    pub total: i64,
    pub unread: i64,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("database: {e}")
}

/// Trasforma il testo digitato in una query FTS5 sicura: ogni parola tra virgolette, con prefisso.
pub fn fts_query(text: &str) -> Option<String> {
    let parts: Vec<String> = text
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

const COLS: &str = "e.id, e.account, e.folder, e.from_addr, e.to_addr, e.subject, e.date, e.date_timestamp, e.has_attachments, e.is_read, e.is_deleted, e.is_starred, e.is_pec";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<EmailRow> {
    Ok(EmailRow {
        id: r.get(0)?,
        account: r.get(1)?,
        folder: r.get(2)?,
        from: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        to: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        subject: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        date: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        timestamp: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
        has_attachments: r.get::<_, i64>(8)? != 0,
        is_read: r.get::<_, i64>(9)? != 0,
        is_deleted: r.get::<_, i64>(10)? != 0,
        is_starred: r.get::<_, i64>(11)? != 0,
        is_pec: r.get::<_, i64>(12)? != 0,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Db, String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(err)?;
        }
        Db::init(Connection::open(path).map_err(err)?)
    }

    #[cfg(test)]
    pub fn open_memory() -> Result<Db, String> {
        Db::init(Connection::open_in_memory().map_err(err)?)
    }

    fn init(conn: Connection) -> Result<Db, String> {
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS emails (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account TEXT NOT NULL, folder TEXT NOT NULL, uid TEXT NOT NULL,
                uid_validity TEXT, message_id TEXT,
                from_addr TEXT, to_addr TEXT, subject TEXT, date TEXT,
                date_timestamp INTEGER, body TEXT, html_body TEXT,
                has_attachments INTEGER NOT NULL DEFAULT 0,
                is_read INTEGER NOT NULL DEFAULT 0, is_deleted INTEGER NOT NULL DEFAULT 0,
                is_starred INTEGER NOT NULL DEFAULT 0, is_pec INTEGER NOT NULL DEFAULT 0,
                raw BLOB,
                UNIQUE(account, folder, uid)
            );
            CREATE TABLE IF NOT EXISTS attachments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                email_id INTEGER NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
                filename TEXT, content_type TEXT, data BLOB
            );
            CREATE TABLE IF NOT EXISTS folder_uidvalidity (
                account TEXT NOT NULL, folder TEXT NOT NULL, uid_validity TEXT NOT NULL,
                PRIMARY KEY(account, folder)
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS emails_fts USING fts5(subject, from_addr, to_addr, body);
            CREATE TRIGGER IF NOT EXISTS emails_fts_ai AFTER INSERT ON emails BEGIN
                INSERT INTO emails_fts(rowid, subject, from_addr, to_addr, body)
                VALUES (new.id, new.subject, new.from_addr, new.to_addr, new.body);
            END;
            CREATE TRIGGER IF NOT EXISTS emails_fts_ad AFTER DELETE ON emails BEGIN
                DELETE FROM emails_fts WHERE rowid = old.id;
            END;
            CREATE TRIGGER IF NOT EXISTS emails_fts_au AFTER UPDATE OF subject, from_addr, to_addr, body ON emails BEGIN
                DELETE FROM emails_fts WHERE rowid = old.id;
                INSERT INTO emails_fts(rowid, subject, from_addr, to_addr, body)
                VALUES (new.id, new.subject, new.from_addr, new.to_addr, new.body);
            END;
            CREATE INDEX IF NOT EXISTS idx_emails_date ON emails(date_timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_emails_account ON emails(account, folder);
            CREATE INDEX IF NOT EXISTS idx_emails_flags ON emails(is_deleted, is_starred, is_pec);
            CREATE INDEX IF NOT EXISTS idx_att_email ON attachments(email_id);",
        )
        .map_err(err)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut g = self
            .conn
            .lock()
            .map_err(|_| "database bloccato da un errore precedente".to_string())?;
        f(&mut g).map_err(err)
    }

    // ---- sincronizzazione ----

    pub fn known_uids(&self, account: &str, folder: &str) -> Result<HashSet<String>, String> {
        self.with(|c| {
            let mut st = c.prepare("SELECT uid FROM emails WHERE account=?1 AND folder=?2")?;
            let rows = st.query_map(params![account, folder], |r| r.get::<_, String>(0))?;
            rows.collect()
        })
    }

    /// Confronta l'UIDVALIDITY del server con quello salvato; se è cambiato svuota la cartella.
    /// Restituisce `true` se la cartella è stata svuotata.
    pub fn check_uidvalidity(
        &self,
        account: &str,
        folder: &str,
        new_uv: &str,
    ) -> Result<bool, String> {
        if new_uv.is_empty() {
            return Ok(false);
        }
        self.with(|c| {
            let old: Option<String> = c
                .query_row(
                    "SELECT uid_validity FROM folder_uidvalidity WHERE account=?1 AND folder=?2",
                    params![account, folder],
                    |r| r.get(0),
                )
                .optional()?;
            let changed = old.as_deref().is_some_and(|o| o != new_uv);
            let tx = c.transaction()?;
            if changed {
                tx.execute(
                    "DELETE FROM emails WHERE account=?1 AND folder=?2",
                    params![account, folder],
                )?;
            }
            tx.execute(
                "INSERT INTO folder_uidvalidity(account, folder, uid_validity) VALUES (?1,?2,?3)
                 ON CONFLICT(account, folder) DO UPDATE SET uid_validity=excluded.uid_validity",
                params![account, folder, new_uv],
            )?;
            tx.commit()?;
            Ok(changed)
        })
    }

    /// Salva (o aggiorna) un messaggio mantenendo letto/preferito/cestino. Restituisce l'id.
    pub fn save_email(
        &self,
        account: &str,
        folder: &str,
        uid: &str,
        raw: &[u8],
        uid_validity: &str,
        is_pec: bool,
    ) -> Result<i64, String> {
        let parsed = mime::parse(raw).unwrap_or_else(|| ParsedMail {
            subject: "(messaggio non leggibile)".into(),
            ..ParsedMail::default()
        });
        self.save_parsed(
            account,
            folder,
            uid,
            raw,
            uid_validity,
            is_pec,
            &parsed,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_parsed(
        &self,
        account: &str,
        folder: &str,
        uid: &str,
        raw: &[u8],
        uid_validity: &str,
        is_pec: bool,
        m: &ParsedMail,
        read: bool,
    ) -> Result<i64, String> {
        let ts = m.timestamp.unwrap_or_else(now_ts);
        let subject = if m.subject.trim().is_empty() {
            "(senza oggetto)"
        } else {
            m.subject.as_str()
        };
        let from = if m.from.is_empty() {
            "Sconosciuto"
        } else {
            m.from.as_str()
        };
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO emails(account, folder, uid, uid_validity, message_id, from_addr, to_addr, subject,
                    date, date_timestamp, body, html_body, has_attachments, is_read, is_pec, raw)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                 ON CONFLICT(account, folder, uid) DO UPDATE SET
                    uid_validity=excluded.uid_validity, message_id=excluded.message_id,
                    from_addr=excluded.from_addr, to_addr=excluded.to_addr, subject=excluded.subject,
                    date=excluded.date, date_timestamp=excluded.date_timestamp, body=excluded.body,
                    html_body=excluded.html_body, has_attachments=excluded.has_attachments,
                    is_pec=excluded.is_pec, raw=excluded.raw",
                params![
                    account,
                    folder,
                    uid,
                    uid_validity,
                    m.message_id,
                    from,
                    m.to,
                    subject,
                    m.date_header,
                    ts,
                    m.body,
                    m.html,
                    i64::from(!m.attachments.is_empty()),
                    i64::from(read),
                    i64::from(is_pec),
                    raw
                ],
            )?;
            let id: i64 = tx.query_row(
                "SELECT id FROM emails WHERE account=?1 AND folder=?2 AND uid=?3",
                params![account, folder, uid],
                |r| r.get(0),
            )?;
            tx.execute("DELETE FROM attachments WHERE email_id=?1", params![id])?;
            for a in &m.attachments {
                tx.execute(
                    "INSERT INTO attachments(email_id, filename, content_type, data) VALUES (?1,?2,?3,?4)",
                    params![id, a.filename, a.content_type, a.data],
                )?;
            }
            tx.commit()?;
            Ok(id)
        })
    }

    // ---- lettura ----

    /// Parte FROM/WHERE comune a `query` e `count`.
    fn from_where(q: &Query) -> (String, Vec<String>) {
        let mut sql = String::from(" FROM emails e");
        let mut args: Vec<String> = Vec::new();
        let mut conds: Vec<String> = Vec::new();
        if let Some(fts) = q.search.as_deref().and_then(fts_query) {
            sql.push_str(" JOIN emails_fts f ON f.rowid = e.id");
            conds.push("emails_fts MATCH ?".into());
            args.push(fts);
        }
        if let Some(d) = q.deleted {
            conds.push(format!("e.is_deleted={}", i32::from(d)));
        }
        if let Some(s) = q.starred {
            conds.push(format!("e.is_starred={}", i32::from(s)));
        }
        if let Some(p) = q.pec {
            conds.push(format!("e.is_pec={}", i32::from(p)));
        }
        if let Some(u) = q.unread {
            conds.push(format!("e.is_read={}", i32::from(!u)));
        }
        if let Some(a) = &q.account {
            conds.push("e.account=?".into());
            args.push(a.clone());
        }
        if let Some(f) = &q.folder {
            conds.push("e.folder=?".into());
            args.push(f.clone());
        }
        if !q.folder_keywords.is_empty() {
            let likes = vec!["LOWER(e.folder) LIKE ?"; q.folder_keywords.len()].join(" OR ");
            conds.push(format!("({likes})"));
            args.extend(
                q.folder_keywords
                    .iter()
                    .map(|k| format!("%{}%", k.to_lowercase())),
            );
        }
        if !conds.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conds.join(" AND "));
        }
        (sql, args)
    }

    pub fn query(&self, q: &Query) -> Result<Vec<EmailRow>, String> {
        let (from, args) = Db::from_where(q);
        let sql = format!(
            "SELECT {COLS}{from} ORDER BY e.date_timestamp DESC, e.id DESC LIMIT {}",
            q.limit.clamp(1, 100_000)
        );
        self.with(|c| {
            let mut st = c.prepare(&sql)?;
            let rows = st.query_map(params_from_iter(args.iter()), row_of)?;
            rows.collect()
        })
    }

    pub fn get_email(&self, id: i64) -> Result<Option<EmailFull>, String> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT {COLS}, e.body, e.html_body FROM emails e WHERE e.id=?1"),
                params![id],
                |r| {
                    Ok(EmailFull {
                        row: row_of(r)?,
                        body: r.get::<_, Option<String>>(13)?.unwrap_or_default(),
                        html: r.get::<_, Option<String>>(14)?.unwrap_or_default(),
                    })
                },
            )
            .optional()
        })
    }

    pub fn raw(&self, id: i64) -> Result<Option<Vec<u8>>, String> {
        self.with(|c| {
            c.query_row("SELECT raw FROM emails WHERE id=?1", params![id], |r| {
                r.get::<_, Option<Vec<u8>>>(0)
            })
            .optional()
            .map(Option::flatten)
        })
    }

    pub fn attachments(&self, email_id: i64) -> Result<Vec<AttachmentInfo>, String> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT id, filename, content_type, length(data) FROM attachments WHERE email_id=?1 ORDER BY id",
            )?;
            let rows = st.query_map(params![email_id], |r| {
                Ok(AttachmentInfo {
                    id: r.get(0)?,
                    filename: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    content_type: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    size: usize::try_from(r.get::<_, Option<i64>>(3)?.unwrap_or(0)).unwrap_or(0),
                })
            })?;
            rows.collect()
        })
    }

    pub fn attachment_data(&self, att_id: i64) -> Result<Option<(String, Vec<u8>)>, String> {
        self.with(|c| {
            c.query_row(
                "SELECT filename, data FROM attachments WHERE id=?1",
                params![att_id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default(),
                    ))
                },
            )
            .optional()
        })
    }

    // ---- modifica ----

    fn set_flag(&self, id: i64, col: &str, v: bool) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                &format!("UPDATE emails SET {col}=?1 WHERE id=?2"),
                params![i64::from(v), id],
            )
            .map(|_| ())
        })
    }

    pub fn mark_read(&self, id: i64, v: bool) -> Result<(), String> {
        self.set_flag(id, "is_read", v)
    }
    pub fn set_starred(&self, id: i64, v: bool) -> Result<(), String> {
        self.set_flag(id, "is_starred", v)
    }
    pub fn set_deleted(&self, id: i64, v: bool) -> Result<(), String> {
        self.set_flag(id, "is_deleted", v)
    }
    pub fn delete_permanently(&self, id: i64) -> Result<(), String> {
        self.with(|c| {
            c.execute("DELETE FROM emails WHERE id=?1", params![id])
                .map(|_| ())
        })
    }
    /// Elimina definitivamente tutto il contenuto del cestino. Restituisce quanti messaggi.
    pub fn empty_trash(&self) -> Result<usize, String> {
        self.with(|c| c.execute("DELETE FROM emails WHERE is_deleted=1", []))
    }
    /// Elimina i messaggi di un account (quando l'account viene rimosso).
    pub fn delete_account_emails(&self, account: &str) -> Result<usize, String> {
        self.with(|c| {
            c.execute(
                "DELETE FROM folder_uidvalidity WHERE account=?1",
                params![account],
            )?;
            c.execute("DELETE FROM emails WHERE account=?1", params![account])
        })
    }

    // ---- statistiche ----

    pub fn folder_stats(&self, account: Option<&str>) -> Result<Vec<FolderStat>, String> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT account, folder, COUNT(*), SUM(CASE WHEN is_read=0 THEN 1 ELSE 0 END)
                 FROM emails WHERE is_deleted=0 AND (?1 IS NULL OR account=?1)
                 GROUP BY account, folder ORDER BY account, folder",
            )?;
            let rows = st.query_map(params![account], |r| {
                Ok(FolderStat {
                    account: r.get(0)?,
                    folder: r.get(1)?,
                    total: r.get(2)?,
                    unread: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                })
            })?;
            rows.collect()
        })
    }

    /// Conta i messaggi che corrispondono a una query (senza limite).
    pub fn count(&self, q: &Query) -> Result<usize, String> {
        let (from, args) = Db::from_where(q);
        let n: i64 = self.with(|c| {
            c.query_row(
                &format!("SELECT COUNT(*){from}"),
                params_from_iter(args.iter()),
                |r| r.get(0),
            )
        })?;
        Ok(usize::try_from(n).unwrap_or(0))
    }
}

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mime::tests::SAMPLE;

    fn db() -> Db {
        Db::open_memory().unwrap()
    }

    #[test]
    fn save_query_and_attachments() {
        let d = db();
        let id = d
            .save_email("a@x.it", "INBOX", "1", SAMPLE.as_bytes(), "7", false)
            .unwrap();
        let rows = d
            .query(&Query {
                limit: 10,
                deleted: Some(false),
                ..Query::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].has_attachments && !rows[0].is_read && rows[0].subject.starts_with("Ciao"));
        let full = d.get_email(id).unwrap().unwrap();
        assert!(full.body.contains("Testo semplice"));
        let atts = d.attachments(id).unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0].size, 5);
        assert_eq!(d.attachment_data(atts[0].id).unwrap().unwrap().1, b"Hello");
        assert_eq!(d.raw(id).unwrap().unwrap(), SAMPLE.as_bytes());
        assert!(d.get_email(999).unwrap().is_none());
    }

    #[test]
    fn resave_keeps_flags_and_does_not_duplicate() {
        let d = db();
        let id = d
            .save_email("a", "INBOX", "1", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        d.set_starred(id, true).unwrap();
        d.mark_read(id, true).unwrap();
        let id2 = d
            .save_email("a", "INBOX", "1", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        assert_eq!(id, id2);
        let r = &d
            .query(&Query {
                limit: 5,
                ..Query::default()
            })
            .unwrap()[0];
        assert!(r.is_starred && r.is_read);
        assert_eq!(d.known_uids("a", "INBOX").unwrap().len(), 1);
        assert_eq!(d.attachments(id).unwrap().len(), 1);
    }

    #[test]
    fn search_is_safe_and_finds_prefixes() {
        let d = db();
        d.save_email("a", "INBOX", "1", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        let s = |t: &str| {
            d.query(&Query {
                limit: 5,
                search: Some(t.into()),
                ..Query::default()
            })
            .unwrap()
            .len()
        };
        assert_eq!(s("sempl"), 1);
        assert_eq!(s("ciao semplice"), 1);
        assert_eq!(s("inesistente"), 0);
        for hostile in [
            "\"",
            "AND OR NOT",
            "a*(",
            "NEAR(",
            "col:x",
            "'; DROP TABLE emails;--",
        ] {
            let _ = s(hostile); // non deve dare errore né panico
        }
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("a\"b c").unwrap(), "\"ab\"* \"c\"*");
    }

    #[test]
    fn filters_and_trash() {
        let d = db();
        let a = d
            .save_email("a", "INBOX", "1", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        let b = d
            .save_email("a", "Sent Items", "2", SAMPLE.as_bytes(), "1", true)
            .unwrap();
        let n = |q: Query| d.query(&Query { limit: 10, ..q }).unwrap().len();
        assert_eq!(
            n(Query {
                folder: Some("INBOX".into()),
                deleted: Some(false),
                ..Query::default()
            }),
            1
        );
        assert_eq!(
            n(Query {
                folder_keywords: vec!["sent".into(), "inviat".into()],
                ..Query::default()
            }),
            1
        );
        assert_eq!(
            n(Query {
                pec: Some(true),
                ..Query::default()
            }),
            1
        );
        d.set_deleted(a, true).unwrap();
        assert_eq!(
            n(Query {
                deleted: Some(true),
                ..Query::default()
            }),
            1
        );
        assert_eq!(
            n(Query {
                deleted: Some(false),
                ..Query::default()
            }),
            1
        );
        d.set_deleted(a, false).unwrap();
        d.set_deleted(a, true).unwrap();
        assert_eq!(d.empty_trash().unwrap(), 1);
        d.delete_permanently(b).unwrap();
        assert_eq!(n(Query::default()), 0);
        assert!(d.attachments(a).unwrap().is_empty());
    }

    #[test]
    fn uidvalidity_change_clears_folder() {
        let d = db();
        assert!(!d.check_uidvalidity("a", "INBOX", "5").unwrap());
        d.save_email("a", "INBOX", "1", SAMPLE.as_bytes(), "5", false)
            .unwrap();
        d.save_email("a", "Altra", "1", SAMPLE.as_bytes(), "9", false)
            .unwrap();
        assert!(!d.check_uidvalidity("a", "INBOX", "5").unwrap());
        assert!(!d.check_uidvalidity("a", "INBOX", "").unwrap());
        assert!(d.check_uidvalidity("a", "INBOX", "6").unwrap());
        assert!(d.known_uids("a", "INBOX").unwrap().is_empty());
        assert_eq!(d.known_uids("a", "Altra").unwrap().len(), 1);
    }

    #[test]
    fn stats_and_account_removal() {
        let d = db();
        let id = d
            .save_email("a", "INBOX", "1", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        d.save_email("a", "INBOX", "2", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        d.save_email("b", "INBOX", "1", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        d.mark_read(id, true).unwrap();
        let st = d.folder_stats(Some("a")).unwrap();
        assert_eq!(
            st,
            vec![FolderStat {
                account: "a".into(),
                folder: "INBOX".into(),
                total: 2,
                unread: 1
            }]
        );
        assert_eq!(d.folder_stats(None).unwrap().len(), 2);
        assert_eq!(d.delete_account_emails("a").unwrap(), 2);
        assert_eq!(d.count(&Query::default()).unwrap(), 1);
    }

    #[test]
    fn unreadable_message_is_still_stored() {
        let d = db();
        d.save_email("a", "INBOX", "1", b"\xff\xfe", "1", false)
            .unwrap();
        assert_eq!(
            d.query(&Query {
                limit: 5,
                ..Query::default()
            })
            .unwrap()
            .len(),
            1
        );
    }
}
