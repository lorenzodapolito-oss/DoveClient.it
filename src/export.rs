//! Esportazione dei messaggi in file .eml o in un archivio .mbox.

use crate::db::{Db, EmailRow};
use crate::mime::sanitize_filename;
use std::io::Write;
use std::path::{Path, PathBuf};

fn eml_name(row: &EmailRow) -> String {
    let subj: String = row.subject.chars().take(40).collect();
    let s = sanitize_filename(if subj.trim().is_empty() {
        "senza_oggetto"
    } else {
        &subj
    });
    format!("{}_{}.eml", row.id, s)
}

/// Scrive un messaggio nel formato mboxrd (le righe «From » nel testo vengono protette con «>»).
pub fn write_mbox_message(out: &mut impl Write, raw: &[u8], when: &str) -> std::io::Result<()> {
    out.write_all(format!("From MAILER-DAEMON {when}\n").as_bytes())?;
    let text = raw
        .strip_suffix(b"\r\n")
        .or_else(|| raw.strip_suffix(b"\n"))
        .unwrap_or(raw);
    for line in text.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let stripped = line.iter().position(|&b| b != b'>').unwrap_or(line.len());
        if line
            .get(stripped..)
            .is_some_and(|r| r.starts_with(b"From "))
        {
            out.write_all(b">")?;
        }
        out.write_all(line)?;
        out.write_all(b"\n")?;
    }
    out.write_all(b"\n")
}

/// Esporta in `dir`; restituisce il numero di messaggi scritti.
pub fn export(
    db: &Db,
    rows: &[EmailRow],
    dir: &Path,
    as_mbox: bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<usize, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cartella di destinazione: {e}"))?;
    let mut written = 0;
    let total = rows.len();
    let mut mbox = if as_mbox {
        let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let path: PathBuf = dir.join(format!("export_{stamp}.mbox"));
        Some(std::io::BufWriter::new(
            std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?,
        ))
    } else {
        None
    };
    let when = chrono::Local::now()
        .format("%a %b %e %H:%M:%S %Y")
        .to_string();
    for (i, row) in rows.iter().enumerate() {
        if let Some(raw) = db.raw(row.id)? {
            match &mut mbox {
                Some(w) => {
                    write_mbox_message(w, &raw, &when).map_err(|e| format!("scrittura: {e}"))?
                }
                None => std::fs::write(dir.join(eml_name(row)), &raw)
                    .map_err(|e| format!("scrittura: {e}"))?,
            }
            written += 1;
        }
        progress(i + 1, total);
    }
    if let Some(mut w) = mbox {
        w.flush().map_err(|e| format!("scrittura: {e}"))?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Query;
    use crate::mime::tests::SAMPLE;

    #[test]
    fn mboxrd_escaping() {
        let mut v = Vec::new();
        write_mbox_message(
            &mut v,
            b"Subject: x\r\n\r\nFrom qui\r\n>From gia'\r\nciao\r\n",
            "Tue Nov  3 10:00:00 2026",
        )
        .unwrap();
        let s = String::from_utf8(v).unwrap();
        assert_eq!(s, "From MAILER-DAEMON Tue Nov  3 10:00:00 2026\nSubject: x\n\n>From qui\n>>From gia'\nciao\n\n");
    }

    #[test]
    fn exports_eml_and_mbox() {
        let db = Db::open_memory().unwrap();
        db.save_email("a", "INBOX", "1", SAMPLE.as_bytes(), "1", false)
            .unwrap();
        db.save_email(
            "a",
            "INBOX",
            "2",
            b"Subject: ../../x\r\n\r\ncorpo",
            "1",
            false,
        )
        .unwrap();
        let rows = db
            .query(&Query {
                limit: 10,
                ..Query::default()
            })
            .unwrap();
        let d = std::env::temp_dir().join(format!("dove_e_{}", std::process::id()));
        let mut steps = 0;
        assert_eq!(
            export(&db, &rows, &d.join("eml"), false, &mut |_, _| steps += 1).unwrap(),
            2
        );
        assert_eq!(steps, 2);
        let names: Vec<_> = std::fs::read_dir(d.join("eml"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names
            .iter()
            .all(|n| n.ends_with(".eml") && !n.contains('/')));
        assert_eq!(
            export(&db, &rows, &d.join("mbox"), true, &mut |_, _| {}).unwrap(),
            2
        );
        let mb = std::fs::read_dir(d.join("mbox"))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        assert_eq!(
            std::fs::read_to_string(mb)
                .unwrap()
                .matches("From MAILER-DAEMON")
                .count(),
            2
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
