//! Connessioni TCP/TLS condivise da IMAP e dai test di connessione.

use rustls::pki_types::ServerName;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Security {
    /// TLS fin dall'inizio (porte 993 / 465).
    Ssl,
    /// Connessione in chiaro poi STARTTLS (porte 143 / 587).
    StartTls,
    /// Nessuna cifratura: solo per i test.
    #[allow(dead_code)]
    Plain,
}

pub enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

/// Messaggio in italiano per un errore di rete.
pub fn describe_io(e: &std::io::Error, host: &str, port: u16) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        TimedOut | WouldBlock => "tempo scaduto".into(),
        ConnectionRefused => format!("connessione rifiutata sulla porta {port}"),
        ConnectionReset | ConnectionAborted | UnexpectedEof => {
            "il server ha chiuso la connessione".into()
        }
        _ => {
            let t = e.to_string();
            if t.to_lowercase().contains("certificate") {
                return format!("certificato del server non valido ({t})");
            }
            if t.to_lowercase().contains("lookup")
                || t.contains("resolve")
                || t.contains("No such host")
                || t.contains("failed to lookup")
            {
                format!("server {host} non trovato (controlla il nome)")
            } else {
                t
            }
        }
    }
}

pub fn tcp_connect(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|_| format!("server {host} non trovato (controlla il nome)"))?
        .collect();
    let mut last = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(timeout));
                let _ = s.set_write_timeout(Some(timeout));
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.map_or_else(
        || format!("server {host} non trovato"),
        |e| describe_io(&e, host, port),
    ))
}

pub fn tls_wrap(tcp: TcpStream, host: &str) -> Result<Stream, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("TLS: {e}"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| format!("nome server non valido: {host}"))?;
    let conn =
        rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| format!("TLS: {e}"))?;
    Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}
