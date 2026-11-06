//! Client IMAP minimale: login, elenco cartelle, ricerca UID e scarico dei messaggi.

use crate::net::{self, Security, Stream};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

const MAX_LITERAL: usize = 150 * 1024 * 1024;
const MAX_LINE: usize = 1024 * 1024;

/// Una riga non etichettata del server, con i letterali `{n}` tolti dal testo.
#[derive(Debug, Default, Clone)]
pub struct Untagged {
    pub text: String,
    pub literals: Vec<Vec<u8>>,
}

pub struct Imap {
    r: BufReader<Stream>,
    host: String,
    tag: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderName {
    /// Nome come lo vuole il server (modified UTF-7).
    pub raw: String,
    /// Nome leggibile.
    pub display: String,
}

pub struct Selected {
    pub uidvalidity: String,
    pub exists: u32,
}

fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            o.push('\\');
        }
        o.push(c);
    }
    o.push('"');
    o
}

impl Imap {
    pub fn connect(
        host: &str,
        port: u16,
        sec: Security,
        timeout: Duration,
    ) -> Result<Imap, String> {
        let tcp = net::tcp_connect(host, port, timeout)?;
        let stream = if sec == Security::Ssl {
            net::tls_wrap(tcp, host)?
        } else {
            Stream::Plain(tcp)
        };
        let mut imap = Imap {
            r: BufReader::new(stream),
            host: host.to_string(),
            tag: 0,
        };
        let greeting = imap
            .read_line()
            .map_err(|e| format!("saluto del server: {e}"))?;
        if !greeting.starts_with(b"* OK") && !greeting.starts_with(b"* PREAUTH") {
            return Err(format!(
                "risposta inattesa dal server: {}",
                String::from_utf8_lossy(&greeting).trim()
            ));
        }
        if sec == Security::StartTls {
            imap.command("STARTTLS")?;
            let Imap { r, host, tag } = imap;
            let tcp = match r.into_inner() {
                Stream::Plain(t) => t,
                Stream::Tls(_) => return Err("STARTTLS su connessione già cifrata".into()),
            };
            imap = Imap {
                r: BufReader::new(net::tls_wrap(tcp, &host)?),
                host,
                tag,
            };
        }
        Ok(imap)
    }

    fn read_line(&mut self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        let n = (&mut self.r)
            .take(MAX_LINE as u64)
            .read_until(b'\n', &mut buf)
            .map_err(|e| net::describe_io(&e, &self.host, 0))?;
        if n == 0 {
            return Err("il server ha chiuso la connessione".into());
        }
        Ok(buf)
    }

    fn send(&mut self, line: &str) -> Result<(), String> {
        let w = self.r.get_mut();
        w.write_all(line.as_bytes())
            .and_then(|()| w.write_all(b"\r\n"))
            .and_then(|()| w.flush())
            .map_err(|e| net::describe_io(&e, &self.host, 0))
    }

    /// Esegue un comando e restituisce le righe non etichettate; errore se la risposta non è OK.
    pub fn command(&mut self, cmd: &str) -> Result<Vec<Untagged>, String> {
        self.tag += 1;
        let tag = format!("A{:04}", self.tag);
        self.send(&format!("{tag} {cmd}"))?;
        self.read_response(&tag)
    }

    fn read_response(&mut self, tag: &str) -> Result<Vec<Untagged>, String> {
        let mut out = Vec::new();
        loop {
            let line = self.read_line()?;
            if line.starts_with(tag.as_bytes()) && line.get(tag.len()) == Some(&b' ') {
                let rest = String::from_utf8_lossy(line.get(tag.len() + 1..).unwrap_or(&[]))
                    .trim()
                    .to_string();
                return if rest.to_uppercase().starts_with("OK") {
                    Ok(out)
                } else {
                    Err(rest)
                };
            }
            if line.starts_with(b"+") {
                continue;
            }
            // Riga non etichettata: legge gli eventuali letterali.
            let mut u = Untagged::default();
            let mut cur = line;
            loop {
                let (text_part, lit) = split_literal(&cur);
                u.text.push_str(&String::from_utf8_lossy(text_part));
                match lit {
                    Some(n) => {
                        if n > MAX_LITERAL {
                            return Err("messaggio troppo grande".into());
                        }
                        let mut data = vec![0u8; n];
                        self.r
                            .read_exact(&mut data)
                            .map_err(|e| net::describe_io(&e, &self.host, 0))?;
                        u.literals.push(data);
                        u.text.push_str("{}");
                        cur = self.read_line()?;
                    }
                    None => break,
                }
            }
            out.push(u);
        }
    }

    pub fn login(&mut self, user: &str, pass: &str) -> Result<(), String> {
        let plain_ok = |s: &str| s.chars().all(|c| c.is_ascii() && !c.is_ascii_control());
        if plain_ok(user) && plain_ok(pass) {
            return self
                .command(&format!("LOGIN {} {}", quote(user), quote(pass)))
                .map(|_| ())
                .map_err(|e| format!("accesso rifiutato: {e}"));
        }
        // Caratteri non ASCII: SASL PLAIN in base64.
        self.tag += 1;
        let tag = format!("A{:04}", self.tag);
        self.send(&format!("{tag} AUTHENTICATE PLAIN"))?;
        let cont = self.read_line()?;
        if !cont.starts_with(b"+") {
            return Err("il server non accetta password con caratteri speciali".into());
        }
        self.send(&B64.encode(format!("\0{user}\0{pass}")))?;
        self.read_response(&tag)
            .map(|_| ())
            .map_err(|e| format!("accesso rifiutato: {e}"))
    }

    pub fn list_folders(&mut self) -> Result<Vec<FolderName>, String> {
        let resp = self.command("LIST \"\" \"*\"")?;
        let mut out = Vec::new();
        for u in resp {
            if let Some((flags, name)) = parse_list(&u) {
                if flags.to_lowercase().contains("\\noselect")
                    || flags.to_lowercase().contains("\\nonexistent")
                {
                    continue;
                }
                out.push(FolderName {
                    display: utf7_decode(&name),
                    raw: name,
                });
            }
        }
        Ok(out)
    }

    /// Apre la cartella in sola lettura.
    pub fn examine(&mut self, folder: &str) -> Result<Selected, String> {
        let resp = self.command(&format!("EXAMINE {}", quote(folder)))?;
        let mut sel = Selected {
            uidvalidity: String::new(),
            exists: 0,
        };
        for u in resp {
            if let Some(p) = u.text.find("[UIDVALIDITY ") {
                let v = u.text.get(p + 13..).unwrap_or("");
                sel.uidvalidity = v.chars().take_while(char::is_ascii_digit).collect();
            }
            let t = u.text.trim();
            if let Some(num) = t.strip_prefix("* ").and_then(|r| r.strip_suffix(" EXISTS")) {
                sel.exists = num.trim().parse().unwrap_or(0);
            }
        }
        Ok(sel)
    }

    pub fn uid_search_all(&mut self) -> Result<Vec<u32>, String> {
        let resp = self.command("UID SEARCH ALL")?;
        let mut uids = Vec::new();
        for u in resp {
            if let Some(rest) = u.text.trim().strip_prefix("* SEARCH") {
                uids.extend(
                    rest.split_whitespace()
                        .filter_map(|n| n.parse::<u32>().ok()),
                );
            }
        }
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
    }

    /// Scarica più messaggi in una sola richiesta: coppie (uid, testo grezzo).
    pub fn fetch_many(&mut self, uids: &[u32]) -> Result<Vec<(u32, Vec<u8>)>, String> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let resp = self.command(&format!("UID FETCH {set} (UID BODY.PEEK[])"))?;
        let mut out = Vec::new();
        for u in resp {
            let Some(uid) = parse_uid(&u.text) else {
                continue;
            };
            if let Some(lit) = u.literals.into_iter().next() {
                out.push((uid, lit));
            }
        }
        Ok(out)
    }

    pub fn logout(&mut self) {
        let _ = self.command("LOGOUT");
    }
}

/// Se la riga finisce con `{n}\r\n` restituisce il testo prima e `n`.
fn split_literal(line: &[u8]) -> (&[u8], Option<usize>) {
    let t = line
        .strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))
        .unwrap_or(line);
    if t.last() == Some(&b'}') {
        if let Some(open) = t.iter().rposition(|&c| c == b'{') {
            let num = t.get(open + 1..t.len() - 1).unwrap_or(&[]);
            let num = num.strip_suffix(b"+").unwrap_or(num);
            if !num.is_empty() && num.iter().all(u8::is_ascii_digit) {
                if let Ok(n) = std::str::from_utf8(num).unwrap_or("").parse::<usize>() {
                    return (t.get(..open).unwrap_or(&[]), Some(n));
                }
            }
        }
    }
    (t, None)
}

fn parse_uid(text: &str) -> Option<u32> {
    let p = text.find("UID ")?;
    let v: String = text
        .get(p + 4..)?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    v.parse().ok()
}

/// `* LIST (flags) "/" nome` → (flags, nome)
fn parse_list(u: &Untagged) -> Option<(String, String)> {
    let t = u.text.trim_end();
    let rest = t
        .strip_prefix("* LIST ")
        .or_else(|| t.strip_prefix("* LSUB "))?;
    let close = rest.find(')')?;
    let flags = rest.get(1..close)?.to_string();
    let mut s = rest.get(close + 1..)?.trim_start();
    // delimitatore: "x" oppure NIL
    if let Some(r) = s.strip_prefix('"') {
        let end = r.find('"')?;
        s = r.get(end + 1..)?.trim_start();
    } else {
        s = s.strip_prefix("NIL")?.trim_start();
    }
    let name = if s == "{}" {
        String::from_utf8_lossy(u.literals.first()?).into_owned()
    } else if let Some(q) = s.strip_prefix('"') {
        let mut n = String::new();
        let mut esc = false;
        for c in q.chars() {
            if esc {
                n.push(c);
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                break;
            } else {
                n.push(c);
            }
        }
        n
    } else {
        s.to_string()
    };
    if name.is_empty() {
        None
    } else {
        Some((flags, name))
    }
}

/// Decodifica i nomi di cartella IMAP (UTF-7 modificato): `Posta &AOA-` → `Posta à`.
pub fn utf7_decode(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(rest.get(..p).unwrap_or(""));
        let tail = rest.get(p + 1..).unwrap_or("");
        let Some(end) = tail.find('-') else {
            out.push('&');
            out.push_str(tail);
            return out;
        };
        let enc = tail.get(..end).unwrap_or("");
        if enc.is_empty() {
            out.push('&');
        } else {
            let std_b64 = enc.replace(',', "/");
            let mut padded = std_b64;
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            match B64.decode(padded.as_bytes()) {
                Ok(bytes) => {
                    let units: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                Err(_) => {
                    out.push('&');
                    out.push_str(enc);
                    out.push('-');
                }
            }
        }
        rest = tail.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Server IMAP finto, una sola connessione, con risposte scritte a mano.
    pub fn mock_server(messages: Vec<(u32, Vec<u8>)>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut r = BufReader::new(s.try_clone().unwrap());
            s.write_all(b"* OK mock pronto\r\n").unwrap();
            let mut line = String::new();
            loop {
                line.clear();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let mut parts = line.trim_end().splitn(2, ' ');
                let tag = parts.next().unwrap_or("").to_string();
                let cmd = parts.next().unwrap_or("").to_string();
                let up = cmd.to_uppercase();
                if up.starts_with("LOGIN") {
                    if cmd.contains("\"buona\"") {
                        s.write_all(format!("{tag} OK accesso\r\n").as_bytes())
                            .unwrap();
                    } else {
                        s.write_all(
                            format!("{tag} NO [AUTHENTICATIONFAILED] credenziali errate\r\n")
                                .as_bytes(),
                        )
                        .unwrap();
                    }
                } else if up.starts_with("LIST") {
                    s.write_all(b"* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n")
                        .unwrap();
                    s.write_all(b"* LIST (\\Noselect \\HasChildren) \"/\" \"[Gmail]\"\r\n")
                        .unwrap();
                    s.write_all(b"* LIST (\\HasNoChildren) \"/\" {13}\r\nPosta &AOA-vv\r\n")
                        .unwrap();
                    s.write_all(b"* LIST () NIL Sent\r\n").unwrap();
                    s.write_all(format!("{tag} OK fatto\r\n").as_bytes())
                        .unwrap();
                } else if up.starts_with("EXAMINE") {
                    s.write_all(b"* 2 EXISTS\r\n* OK [UIDVALIDITY 3857529045] valido\r\n")
                        .unwrap();
                    s.write_all(format!("{tag} OK [READ-ONLY] fatto\r\n").as_bytes())
                        .unwrap();
                } else if up.starts_with("UID SEARCH") {
                    let list = messages
                        .iter()
                        .map(|(u, _)| u.to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                    s.write_all(format!("* SEARCH {list}\r\n{tag} OK fatto\r\n").as_bytes())
                        .unwrap();
                } else if up.starts_with("UID FETCH") {
                    for (i, (uid, raw)) in messages.iter().enumerate() {
                        if i % 2 == 0 {
                            s.write_all(
                                format!(
                                    "* {} FETCH (UID {uid} BODY[] {{{}}}\r\n",
                                    i + 1,
                                    raw.len()
                                )
                                .as_bytes(),
                            )
                            .unwrap();
                            s.write_all(raw).unwrap();
                            s.write_all(b")\r\n").unwrap();
                        } else {
                            // UID dopo il letterale
                            s.write_all(
                                format!("* {} FETCH (BODY[] {{{}}}\r\n", i + 1, raw.len())
                                    .as_bytes(),
                            )
                            .unwrap();
                            s.write_all(raw).unwrap();
                            s.write_all(format!(" UID {uid})\r\n").as_bytes()).unwrap();
                        }
                    }
                    s.write_all(format!("{tag} OK fatto\r\n").as_bytes())
                        .unwrap();
                } else if up.starts_with("LOGOUT") {
                    s.write_all(format!("* BYE\r\n{tag} OK ciao\r\n").as_bytes())
                        .unwrap();
                    return;
                } else {
                    s.write_all(format!("{tag} BAD comando sconosciuto\r\n").as_bytes())
                        .unwrap();
                }
            }
        });
        port
    }

    #[test]
    fn full_session_against_mock() {
        let msgs = vec![
            (5u32, b"Subject: uno\r\n\r\ncorpo\r\n".to_vec()),
            (9u32, b"Subject: due\r\n\r\n)\r\n{3}\r\n".to_vec()),
        ];
        let port = mock_server(msgs.clone());
        let mut im =
            Imap::connect("127.0.0.1", port, Security::Plain, Duration::from_secs(5)).unwrap();
        im.login("utente", "buona").unwrap();
        let f = im.list_folders().unwrap();
        let names: Vec<_> = f.iter().map(|x| x.display.as_str()).collect();
        assert_eq!(names, vec!["INBOX", "Posta àvv", "Sent"]);
        let sel = im.examine("INBOX").unwrap();
        assert_eq!(sel.uidvalidity, "3857529045");
        assert_eq!(sel.exists, 2);
        assert_eq!(im.uid_search_all().unwrap(), vec![5, 9]);
        let got = im.fetch_many(&[5, 9]).unwrap();
        assert_eq!(got, msgs);
        assert!(im.fetch_many(&[]).unwrap().is_empty());
        im.logout();
    }

    #[test]
    fn wrong_password_and_dead_server() {
        let port = mock_server(Vec::new());
        let mut im =
            Imap::connect("127.0.0.1", port, Security::Plain, Duration::from_secs(5)).unwrap();
        let e = im.login("utente", "sbagliata").unwrap_err();
        assert!(e.contains("accesso rifiutato") && e.contains("credenziali"));
        // porta chiusa
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(
            Imap::connect("127.0.0.1", closed, Security::Plain, Duration::from_secs(2)).is_err()
        );
        assert!(Imap::connect(
            "host-che-non-esiste.invalid",
            993,
            Security::Ssl,
            Duration::from_secs(2)
        )
        .is_err());
    }

    #[test]
    fn helpers() {
        assert_eq!(
            split_literal(b"* 1 FETCH (BODY[] {42}\r\n"),
            (&b"* 1 FETCH (BODY[] "[..], Some(42))
        );
        assert_eq!(split_literal(b"* OK ciao\r\n").1, None);
        assert_eq!(split_literal(b"{x}\r\n").1, None);
        assert_eq!(parse_uid("* 1 FETCH (UID 77 BODY[] {}"), Some(77));
        assert_eq!(parse_uid("niente"), None);
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(utf7_decode("Posta &AOA-"), "Posta à");
        assert_eq!(utf7_decode("A&-B"), "A&B");
        assert_eq!(utf7_decode("&2D3eAA-"), "\u{1F600}");
        assert_eq!(utf7_decode("rotto&xx"), "rotto&xx");
        assert_eq!(utf7_decode("&@@@-"), "&@@@-");
    }
}
