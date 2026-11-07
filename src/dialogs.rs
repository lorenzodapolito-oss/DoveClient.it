//! Finestre secondarie: account, scrittura email, statistiche, esportazione, conferme e registro.

use crate::account::Account;
use crate::app::{App, Confirm, Msg};
use crate::db::{EmailRow, FolderStat, Query};
use crate::providers::{self, ServerCfg};
use crate::smtp::{self, Outgoing};
use crate::{export, mime, service, templates};
use eframe::egui::{self, RichText};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// `dir/nome.ext`, oppure `dir/nome (1).ext` se esiste già.
pub fn unique_file(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "allegato".into());
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..10_000 {
        let t = dir.join(format!("{stem} ({n}){ext}"));
        if !t.exists() {
            return t;
        }
    }
    first
}

// ---------------------------------------------------------------- account

pub struct AccountEdit {
    pub orig: Option<Account>,
    pub email: String,
    pub password: String,
    pub display_name: String,
    pub imap_server: String,
    pub imap_port: String,
    pub imap_ssl: bool,
    pub smtp_server: String,
    pub smtp_port: String,
    pub smtp_ssl: bool,
    pub pec: bool,
    pub log: Vec<String>,
    pub running: bool,
    pub cancel: Arc<AtomicBool>,
    pub error: String,
    filled_for: String,
}

impl AccountEdit {
    pub fn new() -> AccountEdit {
        AccountEdit {
            orig: None,
            email: String::new(),
            password: String::new(),
            display_name: String::new(),
            imap_server: String::new(),
            imap_port: "993".into(),
            imap_ssl: true,
            smtp_server: String::new(),
            smtp_port: "465".into(),
            smtp_ssl: true,
            pec: false,
            log: Vec::new(),
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            error: String::new(),
            filled_for: String::new(),
        }
    }

    pub fn from_account(a: &Account) -> AccountEdit {
        AccountEdit {
            orig: Some(a.clone()),
            email: a.email.clone(),
            password: a.password.clone(),
            display_name: a.display_name.clone(),
            imap_server: a.imap_server.clone(),
            imap_port: a.imap_port.to_string(),
            imap_ssl: a.imap_ssl,
            smtp_server: a.smtp_server.clone(),
            smtp_port: a.smtp_port.to_string(),
            smtp_ssl: a.smtp_ssl,
            pec: a.is_pec,
            filled_for: a.email.clone(),
            ..AccountEdit::new()
        }
    }

    pub fn apply_cfg(&mut self, c: &ServerCfg) {
        self.imap_server = c.imap_server.clone();
        self.imap_port = c.imap_port.to_string();
        self.imap_ssl = c.imap_ssl;
        self.smtp_server = c.smtp_server.clone();
        self.smtp_port = c.smtp_port.to_string();
        self.smtp_ssl = c.smtp_ssl;
    }

    fn port(s: &str, what: &str) -> Result<u16, String> {
        match s.trim().parse::<u16>() {
            Ok(p) if p > 0 => Ok(p),
            _ => Err(format!("porta {what} non valida (1-65535)")),
        }
    }

    /// Impostazioni dei server come scritte nel modulo (se valide).
    pub fn cfg(&self) -> Result<ServerCfg, String> {
        let (is, ss) = (self.imap_server.trim(), self.smtp_server.trim());
        if !providers::is_valid_hostname(is) {
            return Err("server IMAP non valido".into());
        }
        if !providers::is_valid_hostname(ss) {
            return Err("server SMTP non valido".into());
        }
        Ok(ServerCfg {
            imap_server: is.to_string(),
            imap_port: Self::port(&self.imap_port, "IMAP")?,
            imap_ssl: self.imap_ssl,
            smtp_server: ss.to_string(),
            smtp_port: Self::port(&self.smtp_port, "SMTP")?,
            smtp_ssl: self.smtp_ssl,
        })
    }

    pub fn check_credentials(&self) -> Result<(), String> {
        if !providers::is_valid_email(self.email.trim()) {
            return Err("indirizzo email non valido".into());
        }
        if self.password.is_empty() {
            return Err("inserisci la password".into());
        }
        Ok(())
    }

    pub fn to_account(&self) -> Result<Account, String> {
        self.check_credentials()?;
        let mut a = Account::new(&self.email, &self.password, &self.cfg()?);
        a.display_name = self.display_name.trim().to_string();
        a.is_pec = self.pec || a.is_pec;
        Ok(a)
    }

    /// Compila i server dal provider noto, se i campi sono ancora vuoti.
    pub fn prefill(&mut self) {
        let e = self.email.trim().to_lowercase();
        if e == self.filled_for || !providers::is_valid_email(&e) {
            return;
        }
        self.filled_for = e.clone();
        if let Some((c, _)) = providers::known_config(&e) {
            self.apply_cfg(&c);
        }
        if e.contains("pec") {
            self.pec = true;
        }
    }
}

impl Default for AccountEdit {
    fn default() -> Self {
        AccountEdit::new()
    }
}

fn security_combo(ui: &mut egui::Ui, id: &str, ssl: &mut bool) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(if *ssl { "SSL" } else { "STARTTLS" })
        .show_ui(ui, |ui| {
            ui.selectable_value(ssl, true, "SSL");
            ui.selectable_value(ssl, false, "STARTTLS");
        });
}

// ---------------------------------------------------------------- scrittura

pub struct Compose {
    pub account: usize,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<PathBuf>,
    pub template: Option<String>,
    pub vars: Vec<(String, String)>,
    pub sending: bool,
    pub error: String,
    pub sender_email: String,
    pub read_receipt: bool,
    pub delivery_receipt: bool,
}

impl Compose {
    pub fn new(account: usize) -> Compose {
        Compose {
            account,
            to: String::new(),
            cc: String::new(),
            subject: String::new(),
            body: String::new(),
            attachments: Vec::new(),
            template: None,
            vars: Vec::new(),
            sending: false,
            error: String::new(),
            sender_email: String::new(),
            read_receipt: false,
            delivery_receipt: false,
        }
    }

    pub fn reply(account: usize, orig: &EmailRow, text: &str, forward: bool) -> Compose {
        let mut c = Compose::new(account);
        let quoted: String = text
            .lines()
            .take(20)
            .map(|l| format!("> {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (prefix, label) = if forward {
            ("Fwd:", "Messaggio inoltrato")
        } else {
            ("Re:", "Messaggio originale")
        };
        if !forward {
            c.to = smtp::bare_address(&orig.from);
        }
        let subj = orig.subject.trim();
        c.subject = if subj.to_lowercase().starts_with(&prefix.to_lowercase()) {
            subj.to_string()
        } else {
            format!("{prefix} {subj}")
        };
        c.body = format!(
            "\n\n\n--- {label} ---\nDa: {}\nData: {}\nOggetto: {}\n\n{}",
            orig.from,
            crate::app::fmt_date(orig.timestamp, &orig.date),
            orig.subject,
            if forward { text.to_string() } else { quoted }
        );
        c
    }

    /// Controlla i destinatari e prepara il messaggio da inviare.
    pub fn outgoing(&self) -> Result<Outgoing, String> {
        let to = smtp::split_recipients(&self.to);
        if to.is_empty() {
            return Err("inserisci almeno un destinatario".into());
        }
        let cc = smtp::split_recipients(&self.cc);
        let bad: Vec<&String> = to
            .iter()
            .chain(cc.iter())
            .filter(|r| !providers::is_valid_email(&smtp::bare_address(r)))
            .collect();
        if !bad.is_empty() {
            let names: Vec<&str> = bad.iter().take(3).map(|s| s.as_str()).collect();
            return Err(format!("indirizzo non valido: {}", names.join(", ")));
        }
        Ok(Outgoing {
            to,
            cc,
            subject: self.subject.trim().to_string(),
            body: self.body.trim_end().to_string(),
            attachments: self.attachments.clone(),
            read_receipt: self.read_receipt,
            delivery_receipt: self.delivery_receipt,
        })
    }
}

// ---------------------------------------------------------------- statistiche ed esportazione

#[derive(Default)]
pub struct StatsWin {
    data: Option<StatsData>,
}

struct StatsData {
    active: usize,
    trash: usize,
    unread: usize,
    starred: usize,
    folders: Vec<FolderStat>,
}

pub struct ExportWin {
    pub rows: Vec<EmailRow>,
    pub dest: String,
    pub mbox: bool,
    pub running: bool,
    pub progress: (usize, usize),
    pub result: Option<String>,
}

impl ExportWin {
    pub fn new(rows: Vec<EmailRow>, dest: PathBuf) -> ExportWin {
        ExportWin {
            rows,
            dest: dest.display().to_string(),
            mbox: false,
            running: false,
            progress: (0, 0),
            result: None,
        }
    }
}

// ---------------------------------------------------------------- finestre

impl App {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        self.accounts_window(ctx);
        self.edit_window(ctx);
        self.compose_window(ctx);
        self.stats_window(ctx);
        self.export_window(ctx);
        self.confirm_window(ctx);
        self.log_window(ctx);
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        if !self.show_log {
            return;
        }
        let mut open = true;
        let mut clear = false;
        egui::Window::new("Registro")
            .open(&mut open)
            .default_size([560.0, 300.0])
            .show(ctx, |ui| {
                if ui.button("Pulisci").clicked() {
                    clear = true;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.log.is_empty() {
                            ui.label("Nessun messaggio.");
                        }
                        for l in &self.log {
                            ui.label(RichText::new(l).monospace());
                        }
                    });
            });
        if clear {
            self.log.clear();
        }
        self.show_log = open;
    }

    fn accounts_window(&mut self, ctx: &egui::Context) {
        if !self.show_accounts {
            return;
        }
        let mut open = true;
        let mut action: Option<(u8, usize)> = None;
        egui::Window::new("Account")
            .open(&mut open)
            .default_size([460.0, 320.0])
            .show(ctx, |ui| {
                if self.accounts.is_empty() {
                    ui.label("Nessun account configurato.");
                }
                for (i, a) in self.accounts.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let tag = if a.is_pec { "  (PEC)" } else { "" };
                        ui.label(format!("{}{tag}", a.email));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Elimina").clicked() {
                                action = Some((2, i));
                            }
                            if ui.button("Modifica").clicked() {
                                action = Some((1, i));
                            }
                        });
                    });
                    if a.password.is_empty() {
                        ui.label(RichText::new("   password da reinserire").color(self.p().error));
                    }
                }
                ui.separator();
                if ui.button("Aggiungi account").clicked() {
                    action = Some((0, 0));
                }
            });
        self.show_accounts = open;
        match action {
            Some((0, _)) => self.edit = Some(AccountEdit::new()),
            Some((1, i)) => self.edit = self.accounts.get(i).map(AccountEdit::from_account),
            Some((2, i)) => self.confirm = Some(Confirm::RemoveAccount(i)),
            _ => {}
        }
    }

    fn edit_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.edit.take() else {
            return;
        };
        let mut open = true;
        let mut do_detect = false;
        let mut do_save = false;
        let mut stop = false;
        let p = self.p();
        egui::Window::new(if e.orig.is_some() {
            "Modifica account"
        } else {
            "Nuovo account"
        })
        .open(&mut open)
        .default_size([560.0, 520.0])
        .show(ctx, |ui| {
            egui::Grid::new("acc_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Email:");
                    let r = ui.add(egui::TextEdit::singleline(&mut e.email).desired_width(280.0));
                    if r.lost_focus() {
                        e.prefill();
                    }
                    ui.end_row();
                    ui.label("Password:");
                    ui.add(
                        egui::TextEdit::singleline(&mut e.password)
                            .password(true)
                            .desired_width(280.0),
                    );
                    ui.end_row();
                    ui.label("Nome visualizzato:");
                    ui.add(egui::TextEdit::singleline(&mut e.display_name).desired_width(280.0));
                    ui.end_row();
                    ui.label("Server IMAP:");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut e.imap_server).desired_width(200.0));
                        ui.add(egui::TextEdit::singleline(&mut e.imap_port).desired_width(50.0));
                        security_combo(ui, "imap_sec", &mut e.imap_ssl);
                    });
                    ui.end_row();
                    ui.label("Server SMTP:");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut e.smtp_server).desired_width(200.0));
                        ui.add(egui::TextEdit::singleline(&mut e.smtp_port).desired_width(50.0));
                        security_combo(ui, "smtp_sec", &mut e.smtp_ssl);
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut e.pec, "Casella PEC");
                    ui.end_row();
                });
            if let Some((_, note)) =
                providers::known_config(&e.email).filter(|(_, n)| !n.is_empty())
            {
                ui.label(RichText::new(note).color(p.text_secondary));
            }
            let hint = providers::app_password_hint(&e.email);
            if !hint.is_empty() {
                ui.label(RichText::new(hint).color(p.text_secondary));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if e.running {
                    ui.spinner();
                    if ui.button("Interrompi").clicked() {
                        stop = true;
                    }
                } else {
                    if ui.button("Rileva, prova e salva").clicked() {
                        do_detect = true;
                    }
                    if ui.button("Salva senza provare").clicked() {
                        do_save = true;
                    }
                }
            });
            if !e.error.is_empty() {
                ui.label(RichText::new(&e.error).color(p.error));
            }
            if !e.log.is_empty() {
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for l in &e.log {
                            ui.label(RichText::new(l).monospace().size(12.0));
                        }
                    });
            }
        });
        if stop {
            e.cancel.store(true, Ordering::Relaxed);
            e.running = false;
            e.log.push("Interrotto.".into());
        }
        if do_detect {
            e.error.clear();
            match e.check_credentials() {
                Err(m) => e.error = m,
                Ok(()) => {
                    e.log.clear();
                    e.running = true;
                    e.cancel = Arc::new(AtomicBool::new(false));
                    let (email, pw, cur) = (
                        e.email.trim().to_lowercase(),
                        e.password.clone(),
                        e.cfg().ok(),
                    );
                    let (tx, cancel, c2) = (self.tx.clone(), e.cancel.clone(), ctx.clone());
                    std::thread::spawn(move || {
                        let r = service::find_working_config(
                            &email,
                            &pw,
                            cur.as_ref(),
                            &mut |l| {
                                let _ = tx.send(Msg::DetectLog(l));
                                c2.request_repaint();
                            },
                            &cancel,
                        );
                        let _ = tx.send(Msg::DetectDone(r));
                        c2.request_repaint();
                    });
                }
            }
        }
        if do_save {
            match e.to_account() {
                Ok(a) => {
                    self.save_account(a, e.orig.clone(), ctx);
                    open = false;
                }
                Err(m) => e.error = m,
            }
        }
        if open {
            self.edit = Some(e);
        } else if e.running {
            e.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Salva l'account (sostituendo quello vecchio se l'indirizzo è cambiato) e aggiorna la posta.
    pub fn save_account(&mut self, acc: Account, orig: Option<Account>, ctx: &egui::Context) {
        if let Err(e) = self.store.save(&acc) {
            self.status = format!("Errore: {e}");
            return;
        }
        if let Some(o) = orig.filter(|o| o.email != acc.email) {
            let _ = self.store.delete(&o);
            self.accounts.retain(|a| a.email != o.email);
        }
        self.accounts.retain(|a| a.email != acc.email);
        self.status = format!("Account {} salvato", acc.email);
        self.accounts.push(acc);
        self.accounts.sort_by(|a, b| a.email.cmp(&b.email));
        self.account_sel = None;
        self.refresh(ctx);
    }

    pub fn finish_detect(&mut self, cfg: Option<ServerCfg>, ctx: &egui::Context) {
        let Some(mut e) = self.edit.take() else {
            return;
        };
        if !e.running {
            self.edit = Some(e);
            return;
        }
        e.running = false;
        match cfg {
            Some(c) => {
                e.apply_cfg(&c);
                e.log
                    .push(format!("Configurazione funzionante: {}", c.describe()));
                match e.to_account() {
                    Ok(a) => {
                        self.save_account(a, e.orig.clone(), ctx);
                        return;
                    }
                    Err(m) => e.error = m,
                }
            }
            None => {
                if !e.cancel.load(Ordering::Relaxed) {
                    e.log.push("Nessuna configurazione ha funzionato.".into());
                    e.log
                        .push("Controlla la password e le impostazioni del provider.".into());
                    let h = providers::app_password_hint(&e.email);
                    if !h.is_empty() {
                        e.log.push(h);
                    }
                }
            }
        }
        self.edit = Some(e);
    }

    fn compose_window(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.compose.take() else {
            return;
        };
        let mut open = true;
        let mut send = false;
        let mut pick_attach = false;
        let mut apply_template = false;
        let tdir = self.dirs.templates.clone();
        let names = templates::list(&tdir);
        let p = self.p();
        let accs: Vec<String> = self.accounts.iter().map(|a| a.email.clone()).collect();
        egui::Window::new("Nuova email")
            .open(&mut open)
            .default_size([700.0, 560.0])
            .show(ctx, |ui| {
                egui::Grid::new("compose_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Da:");
                        egui::ComboBox::from_id_salt("from_acc")
                            .selected_text(accs.get(c.account).cloned().unwrap_or_default())
                            .show_ui(ui, |ui| {
                                for (i, a) in accs.iter().enumerate() {
                                    ui.selectable_value(&mut c.account, i, a);
                                }
                            });
                        ui.end_row();
                        ui.label("A:");
                        ui.add(
                            egui::TextEdit::singleline(&mut c.to)
                                .desired_width(460.0)
                                .hint_text("più destinatari separati da , o ;"),
                        );
                        ui.end_row();
                        ui.label("Cc:");
                        ui.add(egui::TextEdit::singleline(&mut c.cc).desired_width(460.0));
                        ui.end_row();
                        ui.label("Oggetto:");
                        ui.add(egui::TextEdit::singleline(&mut c.subject).desired_width(460.0));
                        ui.end_row();
                        ui.label("Modello:");
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("tpl")
                                .selected_text(
                                    c.template.clone().unwrap_or_else(|| "(nessuno)".into()),
                                )
                                .show_ui(ui, |ui| {
                                    for n in &names {
                                        if ui
                                            .selectable_label(
                                                c.template.as_deref() == Some(n.as_str()),
                                                n,
                                            )
                                            .clicked()
                                        {
                                            c.template = Some(n.clone());
                                            let text =
                                                templates::load(&tdir, n).unwrap_or_default();
                                            c.vars = templates::placeholders(&text)
                                                .into_iter()
                                                .map(|v| (v, String::new()))
                                                .collect();
                                        }
                                    }
                                });
                            if c.template.is_some() && ui.button("Applica").clicked() {
                                apply_template = true;
                            }
                        });
                        ui.end_row();
                    });
                if c.template.is_some() && !c.vars.is_empty() {
                    ui.label(RichText::new("Valori del modello:").color(p.text_secondary));
                    ui.horizontal_wrapped(|ui| {
                        for (k, v) in &mut c.vars {
                            ui.label(format!("{k}:"));
                            ui.add(egui::TextEdit::singleline(v).desired_width(110.0));
                        }
                    });
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut c.body)
                                .desired_width(f32::INFINITY)
                                .desired_rows(12),
                        );
                    });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut c.read_receipt, "Conferma di lettura");
                    ui.checkbox(&mut c.delivery_receipt, "Conferma di ricezione");
                });
                ui.horizontal(|ui| {
                    if ui.button("Allega file…").clicked() {
                        pick_attach = true;
                    }
                    if !c.attachments.is_empty() {
                        let mut remove = None;
                        for (i, a) in c.attachments.iter().enumerate() {
                            let n = a
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if ui.small_button(format!("{n} ×")).clicked() {
                                remove = Some(i);
                            }
                        }
                        if let Some(i) = remove {
                            c.attachments.remove(i);
                        }
                    }
                });
                if !c.error.is_empty() {
                    ui.label(RichText::new(&c.error).color(p.error));
                }
                ui.horizontal(|ui| {
                    if c.sending {
                        ui.spinner();
                        ui.label("Invio in corso…");
                    } else if ui.button("Invia").clicked() {
                        send = true;
                    }
                });
            });
        if apply_template {
            if let Some(t) = c.template.as_ref().and_then(|n| templates::load(&tdir, n)) {
                let vals: HashMap<String, String> = c
                    .vars
                    .iter()
                    .filter(|(_, v)| !v.is_empty())
                    .cloned()
                    .collect();
                c.body = templates::fill(&t, &vals);
            }
        }
        if pick_attach {
            if let Some(files) = rfd::FileDialog::new().pick_files() {
                c.attachments.extend(files);
            }
        }
        if send {
            c.error.clear();
            match (c.outgoing(), self.accounts.get(c.account).cloned()) {
                (Err(m), _) => c.error = m,
                (_, None) => c.error = "scegli un account".into(),
                (Ok(o), Some(acc)) => {
                    if acc.password.is_empty() {
                        c.error = "la password di questo account va reinserita".into();
                    } else {
                        c.sending = true;
                        c.sender_email = acc.email.clone();
                        let (tx, c2) = (self.tx.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            let r = smtp::send(&acc, &o);
                            let _ = tx.send(Msg::Sent(r));
                            c2.request_repaint();
                        });
                    }
                }
            }
        }
        if open {
            self.compose = Some(c);
        }
    }

    pub fn finish_send(&mut self, r: Result<Vec<u8>, String>) {
        let Some(mut c) = self.compose.take() else {
            return;
        };
        match r {
            Ok(raw) => {
                if let Some(m) = mime::parse(&raw) {
                    let uid = format!(
                        "sent_{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_nanos())
                    );
                    let pec = self
                        .accounts
                        .iter()
                        .find(|a| a.email == c.sender_email)
                        .is_some_and(|a| a.is_pec);
                    let _ =
                        self.db
                            .save_parsed(&c.sender_email, "Sent", &uid, &raw, "", pec, &m, true);
                }
                self.status = "Email inviata".into();
                self.reload();
            }
            Err(e) => {
                c.sending = false;
                c.error = format!("Invio fallito: {e}");
                self.compose = Some(c);
            }
        }
    }

    fn stats_window(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.stats.take() else {
            return;
        };
        let mut open = true;
        let mut refresh = s.data.is_none();
        let p = self.p();
        egui::Window::new("Statistiche").open(&mut open).default_size([520.0, 420.0]).show(ctx, |ui| {
            if ui.button("Aggiorna").clicked() {
                refresh = true;
            }
            if refresh {
                let n = |q: Query| self.db.count(&Query { limit: 1, ..q }).unwrap_or(0);
                s.data = Some(StatsData {
                    active: n(Query { deleted: Some(false), ..Query::default() }),
                    trash: n(Query { deleted: Some(true), ..Query::default() }),
                    unread: n(Query { deleted: Some(false), unread: Some(true), ..Query::default() }),
                    starred: n(Query { deleted: Some(false), starred: Some(true), ..Query::default() }),
                    folders: self.db.folder_stats(None).unwrap_or_default(),
                });
            }
            if let Some(d) = &s.data {
                ui.label(
                    RichText::new(format!(
                        "{} email attive   |   {} nel cestino   |   {} non lette   |   {} preferite",
                        d.active, d.trash, d.unread, d.starred
                    ))
                    .strong(),
                );
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("stats_grid").striped(true).num_columns(4).spacing([16.0, 6.0]).show(ui, |ui| {
                        for h in ["Account", "Cartella", "Messaggi", "Non letti"] {
                            ui.label(RichText::new(h).strong().color(p.text_secondary));
                        }
                        ui.end_row();
                        for f in &d.folders {
                            ui.label(&f.account);
                            ui.label(crate::imap::utf7_decode(&f.folder));
                            ui.label(f.total.to_string());
                            ui.label(f.unread.to_string());
                            ui.end_row();
                        }
                    });
                });
            }
        });
        if open {
            self.stats = Some(s);
        }
    }

    fn export_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.export.take() else {
            return;
        };
        let mut open = true;
        let mut start = false;
        egui::Window::new("Esporta email")
            .open(&mut open)
            .default_size([520.0, 240.0])
            .show(ctx, |ui| {
                ui.label(format!("{} messaggi da esportare", w.rows.len()));
                ui.add_enabled_ui(!w.running, |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut w.mbox, false, "File .eml separati");
                        ui.radio_value(&mut w.mbox, true, "Un unico file .mbox");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Cartella:");
                        ui.add(egui::TextEdit::singleline(&mut w.dest).desired_width(320.0));
                        if ui.button("Sfoglia…").clicked() {
                            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                                w.dest = d.display().to_string();
                            }
                        }
                    });
                });
                if w.running {
                    let f = if w.progress.1 == 0 {
                        0.0
                    } else {
                        w.progress.0 as f32 / w.progress.1 as f32
                    };
                    ui.add(
                        egui::ProgressBar::new(f)
                            .text(format!("{}/{}", w.progress.0, w.progress.1)),
                    );
                } else if ui
                    .add_enabled(
                        !w.rows.is_empty() && !w.dest.trim().is_empty(),
                        egui::Button::new("Esporta"),
                    )
                    .clicked()
                {
                    start = true;
                }
                if let Some(r) = &w.result {
                    ui.label(r);
                }
            });
        if start {
            w.running = true;
            w.result = None;
            let (db, tx, c2) = (self.db.clone(), self.tx.clone(), ctx.clone());
            let (rows, dir, mbox) = (w.rows.clone(), PathBuf::from(w.dest.trim()), w.mbox);
            std::thread::spawn(move || {
                let t2 = tx.clone();
                let c3 = c2.clone();
                let r = export::export(&db, &rows, &dir, mbox, &mut |d, t| {
                    let _ = t2.send(Msg::ExportProgress(d, t));
                    c3.request_repaint();
                });
                let _ = tx.send(Msg::ExportDone(r));
                c2.request_repaint();
            });
        }
        if open {
            self.export = Some(w);
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(c) = self.confirm.take() else { return };
        let mut decision: Option<u8> = None; // 1 = sì, 2 = sì (variante), 0 = no
        egui::Window::new("Conferma")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                match &c {
                    Confirm::Delete(ids, true) => {
                        ui.label(format!(
                            "Eliminare definitivamente {} messaggi? Non si può annullare.",
                            ids.len()
                        ));
                    }
                    Confirm::Delete(ids, false) => {
                        ui.label(format!("Spostare {} messaggi nel cestino?", ids.len()));
                    }
                    Confirm::EmptyTrash => {
                        ui.label(
                            "Svuotare il cestino? I messaggi verranno eliminati definitivamente.",
                        );
                    }
                    Confirm::RemoveAccount(i) => {
                        let name = self
                            .accounts
                            .get(*i)
                            .map(|a| a.email.as_str())
                            .unwrap_or("");
                        ui.label(format!("Eliminare l'account {name}?"));
                    }
                }
                ui.horizontal(|ui| {
                    if let Confirm::RemoveAccount(_) = c {
                        if ui.button("Elimina account e messaggi").clicked() {
                            decision = Some(2);
                        }
                        if ui.button("Solo l'account").clicked() {
                            decision = Some(1);
                        }
                    } else if ui.button("Sì").clicked() {
                        decision = Some(1);
                    }
                    if ui.button("No").clicked() {
                        decision = Some(0);
                    }
                });
            });
        match (decision, c) {
            (None, c) => self.confirm = Some(c),
            (Some(0), _) => {}
            (Some(_), Confirm::Delete(ids, perm)) => self.do_delete(&ids, perm),
            (Some(_), Confirm::EmptyTrash) => {
                self.status = match self.db.empty_trash() {
                    Ok(n) => format!("{n} messaggi eliminati"),
                    Err(e) => e,
                };
                self.reload();
            }
            (Some(d), Confirm::RemoveAccount(i)) => {
                if let Some(a) = self.accounts.get(i).cloned() {
                    let _ = self.store.delete(&a);
                    if d == 2 {
                        let _ = self.db.delete_account_emails(&a.email);
                    }
                    self.accounts.remove(i);
                    self.account_sel = None;
                    self.status = format!("Account {} eliminato", a.email);
                    self.reload();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> EmailRow {
        EmailRow {
            from: "Anna Rossi <anna@x.it>".into(),
            subject: "Prova".into(),
            to: "io@x.it".into(),
            timestamp: 1_793_696_400,
            ..EmailRow::default()
        }
    }

    #[test]
    fn reply_and_forward_fields() {
        let r = Compose::reply(0, &row(), "riga1\nriga2", false);
        assert_eq!(r.to, "anna@x.it");
        assert_eq!(r.subject, "Re: Prova");
        assert!(r.body.contains("> riga1") && r.body.contains("Messaggio originale"));
        let f = Compose::reply(0, &row(), "testo", true);
        assert!(f.to.is_empty() && f.subject == "Fwd: Prova" && f.body.contains("testo"));
        let mut r2 = row();
        r2.subject = "RE: già".into();
        assert_eq!(Compose::reply(0, &r2, "", false).subject, "RE: già");
    }

    #[test]
    fn compose_validation() {
        let mut c = Compose::new(0);
        assert!(c.outgoing().is_err());
        c.to = "a@x.it; Anna <b@x.it>".into();
        let o = c.outgoing().unwrap();
        assert_eq!(o.to.len(), 2);
        c.to = "a@x.it, rotto".into();
        assert!(c.outgoing().unwrap_err().contains("rotto"));
        c.to = "a@x.it".into();
        c.cc = "x@".into();
        assert!(c.outgoing().is_err());
    }

    #[test]
    fn account_form_validation_and_prefill() {
        let mut e = AccountEdit::new();
        assert!(e.to_account().is_err());
        e.email = "Mario@Libero.it".into();
        e.password = "pw".into();
        e.prefill();
        assert_eq!(e.imap_server, "imapmail.libero.it");
        let a = e.to_account().unwrap();
        assert_eq!(a.email, "mario@libero.it");
        e.imap_port = "0".into();
        assert!(e.to_account().unwrap_err().contains("porta"));
        e.imap_port = "993".into();
        e.smtp_server = "no spazi".into();
        assert!(e.to_account().is_err());
        let mut pec = AccountEdit::new();
        pec.email = "x@pec.it".into();
        pec.prefill();
        assert!(pec.pec);
    }

    #[test]
    fn unique_file_names() {
        let d = std::env::temp_dir().join(format!("dove_u_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(unique_file(&d, "a.pdf"), d.join("a.pdf"));
        std::fs::write(d.join("a.pdf"), b"1").unwrap();
        assert_eq!(unique_file(&d, "a.pdf"), d.join("a (1).pdf"));
        std::fs::write(d.join("a (1).pdf"), b"1").unwrap();
        assert_eq!(unique_file(&d, "a.pdf"), d.join("a (2).pdf"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
