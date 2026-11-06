//! Lettura dei messaggi (RFC 5322 / MIME): intestazioni, testo, HTML e allegati.

use mail_parser::{Address, MessageParser, MimeHeaders};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedMail {
    pub subject: String,
    pub from: String,
    pub to: String,
    pub message_id: String,
    pub date_header: String,
    /// Secondi dal 1970 (UTC); `None` se la data manca o non è valida.
    pub timestamp: Option<i64>,
    pub body: String,
    pub html: String,
    pub attachments: Vec<Attachment>,
}

fn fmt_addr(a: Option<&Address<'_>>) -> String {
    let Some(a) = a else { return String::new() };
    a.iter()
        .map(|x| match (x.name(), x.address()) {
            (Some(n), Some(e)) if !n.is_empty() => format!("{n} <{e}>"),
            (_, Some(e)) => e.to_string(),
            (Some(n), None) => n.to_string(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Toglie i caratteri non ammessi nei nomi di file e il percorso.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let s: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " ._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() {
        "allegato".into()
    } else {
        s
    }
}

pub fn parse(raw: &[u8]) -> Option<ParsedMail> {
    let msg = MessageParser::default().parse(raw)?;
    let mut out = ParsedMail {
        subject: msg.subject().unwrap_or("").to_string(),
        from: fmt_addr(msg.from()),
        to: fmt_addr(msg.to()),
        message_id: msg.message_id().unwrap_or("").to_string(),
        date_header: msg.date().map(|d| d.to_rfc822()).unwrap_or_default(),
        timestamp: msg.date().map(|d| d.to_timestamp()),
        body: msg.body_text(0).map(|b| b.into_owned()).unwrap_or_default(),
        html: msg.body_html(0).map(|b| b.into_owned()).unwrap_or_default(),
        attachments: Vec::new(),
    };
    for (i, p) in msg.attachments().enumerate() {
        let name = p
            .attachment_name()
            .map(str::to_string)
            .unwrap_or_else(|| format!("allegato{}", i + 1));
        let ct = p
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".into());
        out.attachments.push(Attachment {
            filename: sanitize_filename(&name),
            content_type: ct,
            data: p.contents().to_vec(),
        });
    }
    Some(out)
}

/// Testo leggibile da un HTML: toglie script/stile e tag, decodifica le entità più comuni.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let b = html.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            let rest = lower.get(i..).unwrap_or("");
            let mut skipped = false;
            for tag in ["script", "style"] {
                if rest.starts_with(&format!("<{tag}")) {
                    let end = lower
                        .get(i..)
                        .and_then(|r| r.find(&format!("</{tag}")))
                        .map(|e| i + e);
                    let close = end
                        .and_then(|e| html.get(e..).and_then(|r| r.find('>')).map(|c| e + c + 1));
                    i = close.unwrap_or(b.len());
                    skipped = true;
                    break;
                }
            }
            if skipped {
                continue;
            }
            let end = html
                .get(i..)
                .and_then(|r| r.find('>'))
                .map_or(b.len(), |e| i + e + 1);
            let tag = rest.get(..end - i).unwrap_or("");
            if tag.starts_with("<br")
                || tag.starts_with("</p")
                || tag.starts_with("</div")
                || tag.starts_with("</tr")
                || tag.starts_with("</li")
                || tag.starts_with("</h")
            {
                out.push('\n');
            }
            i = end;
        } else {
            let next = html
                .get(i..)
                .and_then(|r| r.find('<'))
                .map_or(b.len(), |e| i + e);
            out.push_str(html.get(i..next).unwrap_or(""));
            i = next.max(i + 1);
        }
    }
    let out = decode_entities(&out);
    let mut cleaned = String::new();
    let mut blank = 0;
    for line in out.lines() {
        let l = line.trim();
        if l.is_empty() {
            blank += 1;
            if blank <= 1 && !cleaned.is_empty() {
                cleaned.push('\n');
            }
        } else {
            blank = 0;
            cleaned.push_str(l);
            cleaned.push('\n');
        }
    }
    cleaned.trim().to_string()
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(rest.get(..p).unwrap_or(""));
        let tail = rest.get(p..).unwrap_or("");
        let semi = tail.find(';').filter(|&e| e <= 10);
        let decoded = semi.and_then(|e| {
            let ent = tail.get(1..e)?;
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => ent
                    .strip_prefix('#')
                    .and_then(|n| {
                        if let Some(h) = n.strip_prefix(['x', 'X']) {
                            u32::from_str_radix(h, 16).ok()
                        } else {
                            n.parse().ok()
                        }
                    })
                    .and_then(char::from_u32),
            }?;
            Some((ch, e))
        });
        match decoded {
            Some((ch, e)) => {
                out.push(ch);
                rest = tail.get(e + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const SAMPLE: &str = "From: =?UTF-8?Q?Jos=C3=A9_Rossi?= <jose@example.com>\r\n\
To: a@example.com, B Bianchi <b@example.com>\r\n\
Subject: =?UTF-8?B?Q2lhbyBtb25kbyDwn5iA?=\r\n\
Message-ID: <abc@example.com>\r\n\
Date: Tue, 03 Nov 2026 10:00:00 +0100\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"XX\"\r\n\r\n\
--XX\r\n\
Content-Type: multipart/alternative; boundary=\"YY\"\r\n\r\n\
--YY\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\n\
Testo semplice\r\n\
--YY\r\n\
Content-Type: text/html; charset=utf-8\r\n\r\n\
<p>Testo <b>HTML</b></p>\r\n\
--YY--\r\n\
--XX\r\n\
Content-Type: application/pdf; name=\"../doc.pdf\"\r\n\
Content-Disposition: attachment; filename=\"../doc.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\n\
SGVsbG8=\r\n\
--XX--\r\n";

    #[test]
    fn parses_headers_body_and_attachment() {
        let m = parse(SAMPLE.as_bytes()).unwrap();
        assert!(m.subject.starts_with("Ciao mondo"));
        assert_eq!(m.from, "José Rossi <jose@example.com>");
        assert!(m.to.contains("b@example.com"));
        assert_eq!(m.message_id, "abc@example.com");
        assert_eq!(m.timestamp, Some(1_793_696_400));
        assert!(m.body.contains("Testo semplice"));
        assert!(m.html.contains("<b>HTML</b>"));
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].filename, "doc.pdf");
        assert_eq!(m.attachments[0].data, b"Hello");
    }

    #[test]
    fn garbage_does_not_panic() {
        for raw in [&b""[..], b"\xff\xfe\x00", b"Subject", b"\r\n\r\n"] {
            let _ = parse(raw);
        }
    }

    #[test]
    fn filenames_are_sanitized() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a:b*c.txt"), "a_b_c.txt");
        assert_eq!(sanitize_filename("..."), "allegato");
        assert_eq!(sanitize_filename("relazione è.pdf"), "relazione è.pdf");
    }

    #[test]
    fn html_becomes_text() {
        let t = html_to_text("<style>x{}</style><p>Ciao &amp; <b>mondo</b>&nbsp;&#233;</p><br>Fine<script>alert(1)</script>");
        assert_eq!(t, "Ciao & mondo é\n\nFine");
        assert_eq!(html_to_text(""), "");
        assert_eq!(html_to_text("<unclosed"), "");
        assert_eq!(html_to_text("a &unknown; b &"), "a &unknown; b &");
    }
}
