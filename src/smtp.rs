//! Invio della posta (SMTP) con lettre.

use crate::account::Account;
use crate::providers::app_password_hint;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};
use std::path::PathBuf;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<PathBuf>,
}

/// Divide un elenco di destinatari separati da virgola o punto e virgola.
pub fn split_recipients(s: &str) -> Vec<String> {
    s.split([',', ';'])
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

/// Estrae l'indirizzo da «Nome <indirizzo>».
pub fn bare_address(s: &str) -> String {
    match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => s.get(a + 1..b).unwrap_or(s).trim().to_string(),
        _ => s.trim().to_string(),
    }
}

fn friendly(e: &lettre::transport::smtp::Error, acc: &Account) -> String {
    let t = e.to_string();
    if t.contains("535") || t.to_lowercase().contains("authentication") {
        format!(
            "autenticazione rifiutata. {}",
            app_password_hint(&acc.email)
        )
        .trim()
        .to_string()
    } else {
        t
    }
}

pub fn transport(acc: &Account) -> Result<SmtpTransport, String> {
    let creds = Credentials::new(acc.email.clone(), acc.password.clone());
    let tls = TlsParameters::new(acc.smtp_server.clone()).map_err(|e| format!("TLS: {e}"))?;
    let b = SmtpTransport::builder_dangerous(acc.smtp_server.clone())
        .port(acc.smtp_port)
        .credentials(creds)
        .timeout(Some(TIMEOUT))
        .tls(if acc.smtp_ssl {
            Tls::Wrapper(tls)
        } else {
            Tls::Required(tls)
        });
    Ok(b.build())
}

/// Verifica collegamento e credenziali SMTP.
pub fn test(acc: &Account) -> Result<(), String> {
    let t = transport(acc)?;
    match t.test_connection() {
        Ok(true) => Ok(()),
        Ok(false) => Err("il server non ha risposto correttamente".into()),
        Err(e) => Err(friendly(&e, acc)),
    }
}

fn mailbox(s: &str) -> Result<Mailbox, String> {
    s.parse::<Mailbox>()
        .map_err(|_| format!("indirizzo non valido: {s}"))
}

pub fn build_message(acc: &Account, o: &Outgoing) -> Result<Message, String> {
    if o.to.is_empty() {
        return Err("inserisci almeno un destinatario".into());
    }
    let from = if acc.display_name.trim().is_empty() {
        mailbox(&acc.email)?
    } else {
        let addr = acc
            .email
            .parse()
            .map_err(|_| format!("indirizzo non valido: {}", acc.email))?;
        Mailbox::new(Some(acc.display_name.trim().to_string()), addr)
    };
    let mut b = Message::builder().from(from).subject(o.subject.clone());
    for t in &o.to {
        b = b.to(mailbox(t)?);
    }
    for c in &o.cc {
        b = b.cc(mailbox(c)?);
    }
    let text = SinglePart::plain(o.body.clone());
    if o.attachments.is_empty() {
        return b.singlepart(text).map_err(|e| e.to_string());
    }
    let mut mp = MultiPart::mixed().singlepart(text);
    for p in &o.attachments {
        let data = std::fs::read(p).map_err(|e| format!("allegato {}: {e}", p.display()))?;
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "allegato".into());
        let ct = ContentType::parse("application/octet-stream").map_err(|e| e.to_string())?;
        mp = mp.singlepart(Attachment::new(name).body(data, ct));
    }
    b.multipart(mp).map_err(|e| e.to_string())
}

/// Invia il messaggio e restituisce il testo grezzo inviato (per conservarne una copia).
pub fn send(acc: &Account, o: &Outgoing) -> Result<Vec<u8>, String> {
    let msg = build_message(acc, o)?;
    let raw = msg.formatted();
    transport(acc)?.send(&msg).map_err(|e| friendly(&e, acc))?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn acc(port: u16) -> Account {
        let cfg = crate::providers::ServerCfg {
            imap_server: "x".into(),
            imap_port: 993,
            imap_ssl: true,
            smtp_server: "127.0.0.1".into(),
            smtp_port: port,
            smtp_ssl: false,
        };
        let mut a = Account::new("io@example.com", "pw", &cfg);
        a.display_name = "Io Stesso".into();
        a
    }

    #[test]
    fn recipients_and_addresses() {
        assert_eq!(
            split_recipients("a@x.it; b@x.it , ,c@x.it"),
            vec!["a@x.it", "b@x.it", "c@x.it"]
        );
        assert_eq!(bare_address("Mario Rossi <m@x.it>"), "m@x.it");
        assert_eq!(bare_address(" m@x.it "), "m@x.it");
        assert_eq!(bare_address("rotto >< x"), "rotto >< x");
    }

    #[test]
    fn message_has_headers_body_and_attachment() {
        let f = std::env::temp_dir().join(format!("dove_att_{}.txt", std::process::id()));
        std::fs::write(&f, b"contenuto").unwrap();
        let o = Outgoing {
            to: vec!["Anna <anna@x.it>".into(), "b@x.it".into()],
            cc: vec!["c@x.it".into()],
            subject: "Prova è".into(),
            body: "Ciao".into(),
            attachments: vec![f.clone()],
        };
        let m = build_message(&acc(25), &o).unwrap();
        let parsed = crate::mime::parse(&m.formatted()).unwrap();
        assert_eq!(parsed.subject, "Prova è");
        assert!(parsed.from.contains("Io Stesso") && parsed.to.contains("anna@x.it"));
        assert!(parsed.body.contains("Ciao"));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].data, b"contenuto");
        std::fs::remove_file(&f).unwrap();
    }

    #[test]
    fn invalid_input_is_rejected() {
        let a = acc(25);
        assert!(build_message(&a, &Outgoing::default()).is_err());
        let o = Outgoing {
            to: vec!["non valido".into()],
            ..Outgoing::default()
        };
        assert!(build_message(&a, &o).is_err());
        let o = Outgoing {
            to: vec!["a@x.it".into()],
            attachments: vec!["/non/esiste/x".into()],
            ..Outgoing::default()
        };
        assert!(build_message(&a, &o).unwrap_err().contains("allegato"));
    }

    /// SMTP finto: accetta tutto in chiaro (AUTH PLAIN) e conserva il messaggio.
    fn mock_smtp() -> (u16, std::sync::mpsc::Receiver<String>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok((mut s, _)) = l.accept() else { return };
            let mut r = BufReader::new(s.try_clone().unwrap());
            s.write_all(b"220 mock\r\n").unwrap();
            let mut data = String::new();
            let mut in_data = false;
            let mut line = String::new();
            loop {
                line.clear();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        let _ = tx.send(std::mem::take(&mut data));
                        s.write_all(b"250 ok\r\n").unwrap();
                    } else {
                        data.push_str(&line);
                    }
                    continue;
                }
                let up = line.to_uppercase();
                if up.starts_with("EHLO") {
                    s.write_all(b"250-mock\r\n250 AUTH PLAIN\r\n").unwrap();
                } else if up.starts_with("AUTH") {
                    s.write_all(b"235 ok\r\n").unwrap();
                } else if up.starts_with("DATA") {
                    in_data = true;
                    s.write_all(b"354 vai\r\n").unwrap();
                } else if up.starts_with("QUIT") {
                    s.write_all(b"221 ciao\r\n").unwrap();
                    return;
                } else {
                    s.write_all(b"250 ok\r\n").unwrap();
                }
            }
        });
        (port, rx)
    }

    #[test]
    fn plain_transport_can_deliver() {
        // Senza TLS non possiamo usare `transport()` (richiede TLS); proviamo il trasporto in chiaro direttamente.
        let (port, rx) = mock_smtp();
        let a = acc(port);
        let msg = build_message(
            &a,
            &Outgoing {
                to: vec!["a@x.it".into()],
                subject: "S".into(),
                body: "B".into(),
                ..Outgoing::default()
            },
        )
        .unwrap();
        let t = SmtpTransport::builder_dangerous("127.0.0.1")
            .port(port)
            .tls(Tls::None)
            .credentials(Credentials::new("u".into(), "p".into()))
            .build();
        t.send(&msg).unwrap();
        assert!(rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .contains("Subject: S"));
    }
}
