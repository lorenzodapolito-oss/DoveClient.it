//! Provider noti, alias di dominio e generazione delle configurazioni candidate.

use serde::{Deserialize, Serialize};

/// Impostazioni dei server di posta.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCfg {
    pub imap_server: String,
    pub imap_port: u16,
    /// true = SSL diretto, false = STARTTLS.
    pub imap_ssl: bool,
    pub smtp_server: String,
    pub smtp_port: u16,
    pub smtp_ssl: bool,
}

impl ServerCfg {
    pub fn describe(&self) -> String {
        format!(
            "IMAP {}:{} ({}) · SMTP {}:{} ({})",
            self.imap_server,
            self.imap_port,
            if self.imap_ssl { "SSL" } else { "STARTTLS" },
            self.smtp_server,
            self.smtp_port,
            if self.smtp_ssl { "SSL" } else { "STARTTLS" }
        )
    }
}

struct Provider {
    domain: &'static str,
    imap: &'static str,
    imap_port: u16,
    imap_ssl: bool,
    smtp: &'static str,
    smtp_port: u16,
    smtp_ssl: bool,
    note: &'static str,
}

#[allow(clippy::too_many_arguments)]
const fn p(
    domain: &'static str,
    imap: &'static str,
    imap_port: u16,
    imap_ssl: bool,
    smtp: &'static str,
    smtp_port: u16,
    smtp_ssl: bool,
    note: &'static str,
) -> Provider {
    Provider {
        domain,
        imap,
        imap_port,
        imap_ssl,
        smtp,
        smtp_port,
        smtp_ssl,
        note,
    }
}

const PROVIDERS: &[Provider] = &[
    p(
        "gmail.com",
        "imap.gmail.com",
        993,
        true,
        "smtp.gmail.com",
        587,
        false,
        "Richiede una Password per le app se è attiva la verifica in 2 passaggi",
    ),
    p(
        "outlook.com",
        "outlook.office365.com",
        993,
        true,
        "smtp-mail.outlook.com",
        587,
        false,
        "Può richiedere una Password per le app",
    ),
    p(
        "hotmail.com",
        "outlook.office365.com",
        993,
        true,
        "smtp-mail.outlook.com",
        587,
        false,
        "",
    ),
    p(
        "yahoo.com",
        "imap.mail.yahoo.com",
        993,
        true,
        "smtp.mail.yahoo.com",
        587,
        false,
        "",
    ),
    p(
        "yahoo.it",
        "imap.mail.yahoo.com",
        993,
        true,
        "smtp.mail.yahoo.com",
        587,
        false,
        "",
    ),
    p(
        "icloud.com",
        "imap.mail.me.com",
        993,
        true,
        "smtp.mail.me.com",
        587,
        false,
        "",
    ),
    p(
        "libero.it",
        "imapmail.libero.it",
        993,
        true,
        "smtp.libero.it",
        465,
        true,
        "",
    ),
    p(
        "alice.it",
        "in.alice.it",
        143,
        false,
        "out.alice.it",
        587,
        false,
        "IMAP su 143 con STARTTLS",
    ),
    p(
        "virgilio.it",
        "in.virgilio.it",
        993,
        true,
        "out.virgilio.it",
        465,
        true,
        "",
    ),
    p(
        "tiscali.it",
        "imap.tiscali.it",
        993,
        true,
        "smtp.tiscali.it",
        465,
        true,
        "",
    ),
    p(
        "poste.it",
        "imap.poste.it",
        993,
        true,
        "smtp.poste.it",
        465,
        true,
        "",
    ),
    p(
        "email.it",
        "imap.email.it",
        993,
        true,
        "smtp.email.it",
        465,
        true,
        "",
    ),
    p(
        "aruba.it",
        "imaps.aruba.it",
        993,
        true,
        "smtps.aruba.it",
        465,
        true,
        "",
    ),
    p(
        "tin.it",
        "box.tin.it",
        993,
        true,
        "smtp.tin.it",
        465,
        true,
        "",
    ),
    p(
        "pec.it",
        "imaps.pec.aruba.it",
        993,
        true,
        "smtps.pec.aruba.it",
        465,
        true,
        "PEC Aruba",
    ),
    p(
        "arubapec.it",
        "imaps.pec.aruba.it",
        993,
        true,
        "smtps.pec.aruba.it",
        465,
        true,
        "PEC Aruba",
    ),
    p(
        "legalmail.it",
        "mail.legalmail.it",
        993,
        true,
        "mail.legalmail.it",
        465,
        true,
        "PEC Legalmail",
    ),
];

const ALIASES: &[(&str, &str)] = &[
    ("iol.it", "libero.it"),
    ("inwind.it", "libero.it"),
    ("blu.it", "libero.it"),
    ("giallo.it", "libero.it"),
    ("hotmail.it", "outlook.com"),
    ("msn.com", "outlook.com"),
    ("live.com", "outlook.com"),
    ("live.it", "outlook.com"),
    ("outlook.it", "outlook.com"),
    ("me.com", "icloud.com"),
    ("mac.com", "icloud.com"),
    ("googlemail.com", "gmail.com"),
    ("tim.it", "alice.it"),
];

const APP_PASSWORD_DOMAINS: &[&str] = &[
    "gmail.com",
    "yahoo.com",
    "yahoo.it",
    "icloud.com",
    "outlook.com",
    "hotmail.com",
];

/// Tentativi di porte usati quando nessun'altra fonte conosce il dominio.
const PROBES: &[(u16, bool, u16, bool)] = &[
    (993, true, 465, true),
    (993, true, 587, false),
    (143, false, 587, false),
];

pub fn domain_of(email: &str) -> String {
    email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .unwrap_or_default()
}

pub fn resolve_alias(domain: &str) -> String {
    let d = domain.to_lowercase();
    ALIASES
        .iter()
        .find(|(a, _)| *a == d)
        .map_or(d, |(_, t)| (*t).to_string())
}

pub fn is_valid_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
        && domain.rsplit('.').next().is_some_and(|t| t.len() >= 2)
}

pub fn is_valid_hostname(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

fn from_provider(p: &Provider) -> ServerCfg {
    ServerCfg {
        imap_server: p.imap.into(),
        imap_port: p.imap_port,
        imap_ssl: p.imap_ssl,
        smtp_server: p.smtp.into(),
        smtp_port: p.smtp_port,
        smtp_ssl: p.smtp_ssl,
    }
}

/// Configurazione nota per l'indirizzo, se il provider è in elenco.
pub fn known_config(email: &str) -> Option<(ServerCfg, &'static str)> {
    let d = resolve_alias(&domain_of(email));
    PROVIDERS
        .iter()
        .find(|p| p.domain == d)
        .map(|p| (from_provider(p), p.note))
}

/// Suggerimento sulle password per le app, vuoto se non serve.
pub fn app_password_hint(email: &str) -> String {
    let d = resolve_alias(&domain_of(email));
    if !APP_PASSWORD_DOMAINS.contains(&d.as_str()) {
        return String::new();
    }
    let how = match d.as_str() {
        "gmail.com" => "Account Google, Sicurezza, Verifica in 2 passaggi, Password per le app.",
        "yahoo.com" | "yahoo.it" => "Info account, Sicurezza, Password per le app.",
        "icloud.com" => "appleid.apple.com, Accesso e sicurezza, Password per le app.",
        _ => "Crea una password per le app dalle impostazioni di sicurezza dell'account.",
    };
    format!("Per @{d} serve una «Password per le app». {how}")
}

/// Interpreta la configurazione automatica di Mozilla (XML, formato 1.1).
pub fn parse_ispdb(xml: &str, domain: &str) -> Vec<ServerCfg> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let fill = |s: &str| s.trim().replace("%EMAILDOMAIN%", domain);
    let mut imaps = Vec::new();
    let mut smtps = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        let kind = n.attribute("type").unwrap_or("");
        let tag = n.tag_name().name();
        if !(tag == "incomingServer" || tag == "outgoingServer") {
            continue;
        }
        let get = |name: &str| {
            n.children()
                .find(|c| c.has_tag_name(name))
                .and_then(|c| c.text())
                .map(fill)
        };
        let (Some(host), socket) = (get("hostname"), get("socketType").unwrap_or_default()) else {
            continue;
        };
        if host.contains('%') || !is_valid_hostname(&host) {
            continue;
        }
        let ssl = socket.eq_ignore_ascii_case("SSL");
        let default_port = match (kind, ssl) {
            ("imap", true) => 993,
            ("imap", false) => 143,
            (_, true) => 465,
            (_, false) => 587,
        };
        let port = get("port")
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(default_port);
        match kind {
            "imap" => imaps.push((host, port, ssl)),
            "smtp" => smtps.push((host, port, ssl)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (ih, ip, is) in &imaps {
        for (sh, sp, ss) in &smtps {
            out.push(ServerCfg {
                imap_server: ih.clone(),
                imap_port: *ip,
                imap_ssl: *is,
                smtp_server: sh.clone(),
                smtp_port: *sp,
                smtp_ssl: *ss,
            });
        }
    }
    out
}

fn fetch_ispdb(domain: &str) -> Vec<ServerCfg> {
    if !is_valid_hostname(domain) {
        return Vec::new();
    }
    let url = format!("https://autoconfig.thunderbird.net/v1.1/{domain}");
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build();
    let body = agent
        .get(&url)
        .call()
        .ok()
        .and_then(|r| r.into_string().ok());
    body.map(|xml| parse_ispdb(&xml, domain))
        .unwrap_or_default()
}

/// Elenco ordinato (senza doppioni) delle configurazioni da provare:
/// quella attuale, il provider noto, Mozilla ISPDB e infine i tentativi per porta.
pub fn candidates(
    email: &str,
    current: Option<&ServerCfg>,
    online: bool,
) -> Vec<(ServerCfg, String)> {
    let domain = resolve_alias(&domain_of(email));
    if domain.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(ServerCfg, String)> = Vec::new();
    let mut add = |cfg: ServerCfg, src: String| {
        if !out.iter().any(|(c, _)| *c == cfg) {
            out.push((cfg, src));
        }
    };
    if let Some(c) = current {
        if !c.imap_server.is_empty() && !c.smtp_server.is_empty() {
            add(c.clone(), "impostazioni attuali".into());
        }
    }
    if let Some((c, _)) = known_config(email) {
        add(c, format!("provider noto ({domain})"));
    }
    if online {
        for c in fetch_ispdb(&domain) {
            add(c, "configurazione automatica Mozilla".into());
        }
    }
    for imap_host in [format!("imap.{domain}"), format!("mail.{domain}")] {
        for smtp_host in [format!("smtp.{domain}"), format!("mail.{domain}")] {
            for (ip, is, sp, ss) in PROBES {
                add(
                    ServerCfg {
                        imap_server: imap_host.clone(),
                        imap_port: *ip,
                        imap_ssl: *is,
                        smtp_server: smtp_host.clone(),
                        smtp_port: *sp,
                        smtp_ssl: *ss,
                    },
                    "tentativo sulle porte standard".into(),
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_and_known_providers() {
        assert_eq!(resolve_alias("Hotmail.IT"), "outlook.com");
        let (c, _) = known_config("Mario@iol.it").unwrap();
        assert_eq!(c.imap_server, "imapmail.libero.it");
        assert!(known_config("x@sconosciuto.example").is_none());
        let (a, note) = known_config("a@alice.it").unwrap();
        assert!(!a.imap_ssl && note.contains("STARTTLS"));
    }

    #[test]
    fn validation() {
        assert!(is_valid_email("mario.rossi+x@libero.it"));
        for bad in [
            "", "a", "a@b", "@x.it", "a@.it", "a@x.", "a b@x.it", "a@x.i",
        ] {
            assert!(!is_valid_email(bad), "{bad}");
        }
        assert!(
            is_valid_hostname("imap.x-y.it") && !is_valid_hostname("a b") && !is_valid_hostname("")
        );
    }

    #[test]
    fn app_password_hints() {
        assert!(app_password_hint("a@googlemail.com").contains("Password per le app"));
        assert!(app_password_hint("a@libero.it").is_empty());
    }

    #[test]
    fn ispdb_xml() {
        let xml = r#"<clientConfig><emailProvider id="x">
        <incomingServer type="imap"><hostname>imap.%EMAILDOMAIN%</hostname><port>993</port><socketType>SSL</socketType></incomingServer>
        <incomingServer type="pop3"><hostname>pop.x.it</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
        <outgoingServer type="smtp"><hostname>smtp.x.it</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>
        <outgoingServer type="smtp"><hostname>%EMAILLOCALPART%.x.it</hostname></outgoingServer>
        </emailProvider></clientConfig>"#;
        let v = parse_ispdb(xml, "x.it");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].imap_server, "imap.x.it");
        assert!(v[0].imap_ssl && !v[0].smtp_ssl && v[0].smtp_port == 587);
        assert!(parse_ispdb("non xml", "x.it").is_empty());
    }

    #[test]
    fn candidates_are_ordered_and_unique() {
        let cur = ServerCfg {
            imap_server: "imap.miodominio.it".into(),
            imap_port: 993,
            imap_ssl: true,
            smtp_server: "smtp.miodominio.it".into(),
            smtp_port: 465,
            smtp_ssl: true,
        };
        let v = candidates("a@miodominio.it", Some(&cur), false);
        assert_eq!(v[0].0, cur);
        assert_eq!(v[0].1, "impostazioni attuali");
        for (i, (a, _)) in v.iter().enumerate() {
            assert!(!v[i + 1..].iter().any(|(b, _)| a == b));
        }
        assert!(v.len() > 6);
        let g = candidates("a@gmail.com", None, false);
        assert_eq!(g[0].0.imap_server, "imap.gmail.com");
        assert!(candidates("senza-chiocciola", None, false).is_empty());
    }
}
