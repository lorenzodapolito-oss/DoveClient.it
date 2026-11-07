//! Finestra principale di DoveClient: barra strumenti, cartelle, elenco e anteprima.

use crate::account::{Account, AccountStore};
use crate::db::{Db, EmailFull, EmailRow, Query};
use crate::dialogs::{AccountEdit, Compose, ExportWin, StatsWin};
use crate::paths::{Config, Dirs};
use crate::providers::ServerCfg;
use crate::theme::{self, Palette};
use crate::{mime, service};
use eframe::egui::{self, Align2, Color32, FontId, Modifiers, RichText, Sense};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const APP_TITLE: &str = "DoveClient";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const FOLDERS: [&str; 5] = ["Posta in arrivo", "Inviati", "Preferiti", "PEC", "Cestino"];
const TRASH: usize = 4;

/// Messaggi dai thread di lavoro verso la finestra.
pub enum Msg {
    Progress(String),
    SyncDone { new: usize, errors: Vec<String> },
    Imported(usize),
    DetectLog(String),
    DetectDone(Option<ServerCfg>),
    Sent(Result<Vec<u8>, String>),
    ExportProgress(usize, usize),
    ExportDone(Result<usize, String>),
}

pub struct Preview {
    pub full: EmailFull,
    pub atts: Vec<crate::db::AttachmentInfo>,
    pub text: String,
}

pub enum Confirm {
    Delete(Vec<i64>, bool),
    EmptyTrash,
    RemoveAccount(usize),
}

pub struct App {
    pub dirs: Dirs,
    pub cfg: Config,
    pub db: Arc<Db>,
    pub store: Arc<AccountStore>,
    pub accounts: Vec<Account>,
    pub tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub folder: usize,
    pub account_sel: Option<usize>,
    pub search: String,
    search_changed: Option<Instant>,
    pub emails: Vec<EmailRow>,
    pub selected: BTreeSet<i64>,
    pub current: Option<i64>,
    anchor: Option<usize>,
    pub preview: Option<Preview>,
    pub list_limit: usize,
    pub unread_inbox: usize,
    pub unread: std::collections::HashMap<Option<usize>, usize>,
    sort_col: usize,
    sort_asc: bool,
    pub status: String,
    pub syncing: bool,
    pub sync_cancel: Arc<AtomicBool>,
    last_sync: Instant,
    last_input_scan: Instant,
    pub log: Vec<String>,
    pub show_log: bool,
    pub show_accounts: bool,
    pub edit: Option<AccountEdit>,
    pub compose: Option<Compose>,
    pub stats: Option<StatsWin>,
    pub export: Option<ExportWin>,
    pub confirm: Option<Confirm>,
}

pub fn fmt_date(ts: i64, fallback: &str) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d/%m/%Y %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| fallback.chars().take(16).collect())
}

pub fn folder_query(idx: usize) -> Query {
    match idx {
        0 => Query {
            folder: Some("INBOX".into()),
            deleted: Some(false),
            ..Query::default()
        },
        1 => Query {
            folder_keywords: vec!["sent".into(), "inviat".into()],
            deleted: Some(false),
            ..Query::default()
        },
        2 => Query {
            starred: Some(true),
            deleted: Some(false),
            ..Query::default()
        },
        3 => Query {
            pec: Some(true),
            deleted: Some(false),
            ..Query::default()
        },
        _ => Query {
            deleted: Some(true),
            ..Query::default()
        },
    }
}

fn draw_star(ui: &egui::Ui, c: egui::Pos2, r: f32, color: Color32) {
    let pts: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let ang = std::f32::consts::PI * (i as f32) / 5.0 - std::f32::consts::FRAC_PI_2;
            let rad = if i % 2 == 0 { r } else { r * 0.45 };
            egui::pos2(c.x + rad * ang.cos(), c.y + rad * ang.sin())
        })
        .collect();
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        dirs: Dirs,
        db: Db,
        store: AccountStore,
        warns: Vec<String>,
    ) -> App {
        let cfg = Config::load(&dirs.config_file());
        theme::apply(&cc.egui_ctx, cfg.dark_mode);
        let (tx, rx) = channel();
        let (accounts, mut w2) = store.load_all();
        let mut log = warns;
        log.append(&mut w2);
        let mut app = App {
            list_limit: cfg.list_limit,
            dirs,
            cfg,
            db: Arc::new(db),
            store: Arc::new(store),
            accounts,
            tx,
            rx,
            folder: 0,
            account_sel: None,
            search: String::new(),
            search_changed: None,
            emails: Vec::new(),
            selected: BTreeSet::new(),
            current: None,
            anchor: None,
            preview: None,
            unread_inbox: 0,
            unread: std::collections::HashMap::new(),
            sort_col: 2,
            sort_asc: false,
            status: "Pronto".into(),
            syncing: false,
            sync_cancel: Arc::new(AtomicBool::new(false)),
            last_sync: Instant::now(),
            last_input_scan: Instant::now(),
            show_log: !log.is_empty(),
            log,
            show_accounts: false,
            edit: None,
            compose: None,
            stats: None,
            export: None,
            confirm: None,
        };
        let _ = crate::templates::ensure_defaults(&app.dirs.templates);
        app.reload();
        if app.accounts.is_empty() {
            app.status = "Nessun account: apri «Strumenti, Account» per aggiungerne uno".into();
        } else {
            app.refresh(&cc.egui_ctx);
        }
        app
    }

    pub fn p(&self) -> Palette {
        theme::palette(self.cfg.dark_mode)
    }

    pub fn save_cfg(&mut self) {
        self.cfg.list_limit = self.list_limit;
        self.cfg.save(&self.dirs.config_file());
    }

    pub fn add_log(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 500 {
            self.log.drain(0..100);
        }
    }

    // ---- dati ----

    pub fn current_query(&self) -> Query {
        let mut q = folder_query(self.folder);
        if let Some(i) = self.account_sel {
            q.account = self.accounts.get(i).map(|a| a.email.clone());
        }
        q.search = Some(self.search.trim().to_string()).filter(|s| !s.is_empty());
        q.limit = self.list_limit;
        q
    }

    pub fn reload(&mut self) {
        match self.db.query(&self.current_query()) {
            Ok(rows) => {
                self.emails = rows;
                self.sort_rows();
            }
            Err(e) => {
                self.emails.clear();
                self.status = format!("Errore: {e}");
            }
        }
        self.selected
            .retain(|id| self.emails.iter().any(|e| e.id == *id));
        if self.current.is_some_and(|c| !self.selected.contains(&c)) {
            self.current = None;
            self.preview = None;
        }
        let q = Query {
            unread: Some(true),
            account: self
                .account_sel
                .and_then(|i| self.accounts.get(i))
                .map(|a| a.email.clone()),
            ..folder_query(0)
        };
        self.unread_inbox = self.db.count(&q).unwrap_or(0);
        self.unread.clear();
        let mut total = 0;
        for (i, a) in self.accounts.iter().enumerate() {
            let qa = Query {
                account: Some(a.email.clone()),
                ..q.clone()
            };
            let n = self.db.count(&qa).unwrap_or(0);
            total += n;
            self.unread.insert(Some(i), n);
        }
        self.unread.insert(
            None,
            if self.accounts.is_empty() {
                self.unread_inbox
            } else {
                total
            },
        );
    }

    fn open_message(&mut self, id: i64) {
        self.current = Some(id);
        let full = match self.db.get_email(id) {
            Ok(Some(f)) => f,
            _ => {
                self.preview = None;
                return;
            }
        };
        let atts = self.db.attachments(id).unwrap_or_default();
        let text = if !full.body.trim().is_empty() {
            full.body.clone()
        } else if !full.html.trim().is_empty() {
            mime::html_to_text(&full.html)
        } else {
            "(nessun contenuto testuale)".into()
        };
        if !full.row.is_read {
            let _ = self.db.mark_read(id, true);
            if let Some(r) = self.emails.iter_mut().find(|e| e.id == id) {
                r.is_read = true;
            }
            self.unread_inbox = self
                .unread_inbox
                .saturating_sub(usize::from(self.folder == 0));
            let acc = self
                .accounts
                .iter()
                .position(|a| a.email == full.row.account);
            for k in [None, acc] {
                if let Some(n) = self.unread.get_mut(&k) {
                    *n = n.saturating_sub(1);
                }
            }
        }
        self.preview = Some(Preview { full, atts, text });
    }

    fn click_row(&mut self, idx: usize, m: Modifiers) {
        let Some(id) = self.emails.get(idx).map(|e| e.id) else {
            return;
        };
        if m.shift && self.anchor.is_some() {
            let a = self.anchor.unwrap_or(idx);
            let (lo, hi) = (a.min(idx), a.max(idx));
            self.selected = self
                .emails
                .iter()
                .skip(lo)
                .take(hi - lo + 1)
                .map(|e| e.id)
                .collect();
        } else if m.command || m.ctrl {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
            self.anchor = Some(idx);
        } else {
            self.selected = BTreeSet::from([id]);
            self.anchor = Some(idx);
        }
        self.open_message(id);
    }

    // ---- sincronizzazione ----

    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.syncing {
            return;
        }
        if self.accounts.is_empty() {
            self.status = "Nessun account configurato".into();
            return;
        }
        self.syncing = true;
        self.last_sync = Instant::now();
        self.sync_cancel = Arc::new(AtomicBool::new(false));
        self.status = "Aggiornamento in corso…".into();
        let accounts = self.accounts.clone();
        let db = self.db.clone();
        let tx = self.tx.clone();
        let cancel = self.sync_cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut new = 0;
            let mut errors = Vec::new();
            for acc in &accounts {
                let rep = service::sync_account(
                    acc,
                    &db,
                    &mut |m| {
                        let _ = tx.send(Msg::Progress(m));
                        ctx.request_repaint();
                    },
                    &cancel,
                );
                new += rep.new_messages;
                errors.extend(rep.errors);
            }
            let _ = tx.send(Msg::SyncDone { new, errors });
            ctx.request_repaint();
        });
    }

    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Progress(s) => self.status = s,
                Msg::SyncDone { new, errors } => {
                    self.syncing = false;
                    self.status = if errors.is_empty() {
                        format!("Aggiornato: {new} messaggi nuovi")
                    } else {
                        format!(
                            "Aggiornato con {} errori ({new} messaggi nuovi): vedi «Registro»",
                            errors.len()
                        )
                    };
                    for e in errors {
                        self.add_log(e);
                    }
                    self.reload();
                }
                Msg::Imported(n) => {
                    if n > 0 {
                        self.status = format!("{n} email importate dalla cartella input");
                        self.reload();
                    }
                }
                Msg::DetectLog(s) => {
                    if let Some(e) = &mut self.edit {
                        e.log.push(s);
                    }
                }
                Msg::DetectDone(cfg) => self.finish_detect(cfg, ctx),
                Msg::Sent(r) => self.finish_send(r),
                Msg::ExportProgress(d, t) => {
                    if let Some(e) = &mut self.export {
                        e.progress = (d, t);
                    }
                }
                Msg::ExportDone(r) => {
                    if let Some(e) = &mut self.export {
                        e.running = false;
                        e.result = Some(match r {
                            Ok(n) => format!("Esportati {n} messaggi"),
                            Err(e) => format!("Errore: {e}"),
                        });
                    }
                }
            }
        }
    }

    // ---- azioni sui messaggi ----

    fn selected_ids(&self) -> Vec<i64> {
        self.selected.iter().copied().collect()
    }

    fn toggle_star(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.status = "Nessuna email selezionata".into();
            return;
        }
        let all_starred = self
            .emails
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .all(|e| e.is_starred);
        for id in ids {
            let _ = self.db.set_starred(id, !all_starred);
        }
        self.reload();
        if let Some(c) = self.current {
            self.open_message(c);
        }
    }

    fn ask_delete(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.status = "Nessuna email selezionata".into();
            return;
        }
        self.confirm = Some(Confirm::Delete(ids, self.folder == TRASH));
    }

    pub fn do_delete(&mut self, ids: &[i64], permanent: bool) {
        for id in ids {
            let r = if permanent {
                self.db.delete_permanently(*id)
            } else {
                self.db.set_deleted(*id, true)
            };
            if let Err(e) = r {
                self.status = e;
            }
        }
        self.selected.clear();
        self.current = None;
        self.preview = None;
        self.reload();
    }

    fn restore(&mut self) {
        if self.folder != TRASH {
            self.status = "Il ripristino è disponibile solo dal Cestino".into();
            return;
        }
        for id in self.selected_ids() {
            let _ = self.db.set_deleted(id, false);
        }
        self.selected.clear();
        self.current = None;
        self.preview = None;
        self.reload();
    }

    fn export_selected(&mut self) {
        let ids = self.selected_ids();
        let Some(&first) = ids.first() else {
            self.status = "Nessuna email selezionata".into();
            return;
        };
        if ids.len() == 1 {
            let name = self.emails.iter().find(|e| e.id == first).map(|e| {
                let s: String = e.subject.chars().take(40).collect();
                format!("{}.eml", mime::sanitize_filename(&s))
            });
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(name.unwrap_or_else(|| "email.eml".into()))
                .save_file()
            {
                match self.db.raw(first) {
                    Ok(Some(raw)) => {
                        self.status = match std::fs::write(&path, raw) {
                            Ok(()) => format!("Salvata in {}", path.display()),
                            Err(e) => format!("Errore: {e}"),
                        }
                    }
                    _ => self.status = "Messaggio non disponibile".into(),
                }
            }
        } else {
            self.open_export(true);
        }
    }

    pub fn open_export(&mut self, only_selected: bool) {
        let rows: Vec<EmailRow> = if only_selected {
            self.emails
                .iter()
                .filter(|e| self.selected.contains(&e.id))
                .cloned()
                .collect()
        } else {
            self.emails.clone()
        };
        self.export = Some(ExportWin::new(rows, self.dirs.export.clone()));
    }

    fn save_attachments(&mut self) {
        let Some(p) = &self.preview else { return };
        if p.atts.is_empty() {
            return;
        }
        let ids: Vec<i64> = p.atts.iter().map(|a| a.id).collect();
        let Some(dir) = rfd::FileDialog::new()
            .set_directory(&self.dirs.attachments)
            .pick_folder()
        else {
            return;
        };
        self.status = self.write_attachments(&ids, &dir);
    }

    pub fn write_attachments(&self, ids: &[i64], dir: &std::path::Path) -> String {
        let mut saved = 0;
        for id in ids {
            if let Ok(Some((name, data))) = self.db.attachment_data(*id) {
                let target = crate::dialogs::unique_file(dir, &mime::sanitize_filename(&name));
                if std::fs::write(target, data).is_ok() {
                    saved += 1;
                }
            }
        }
        format!("{saved} allegati salvati in {}", dir.display())
    }

    fn reply_or_forward(&mut self, forward: bool) {
        let Some(p) = &self.preview else { return };
        let acc = self
            .accounts
            .iter()
            .position(|a| a.email == p.full.row.account)
            .or((!self.accounts.is_empty()).then_some(0));
        let Some(acc) = acc else {
            self.status = "Configura prima un account".into();
            return;
        };
        self.compose = Some(Compose::reply(acc, &p.full.row, &p.text, forward));
    }

    pub fn new_mail(&mut self) {
        if self.accounts.is_empty() {
            self.status = "Configura prima un account".into();
            return;
        }
        let idx = self.account_sel.unwrap_or(0).min(self.accounts.len() - 1);
        self.compose = Some(Compose::new(idx));
    }

    // ---- disegno ----

    fn menubar(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        let ctx = ui.ctx().clone();
        let t = |s: &str| RichText::new(s).color(p.toolbar_text);
        ui.horizontal(|ui| {
            let w = &mut ui.style_mut().visuals.widgets;
            w.inactive.weak_bg_fill = Color32::TRANSPARENT;
            w.inactive.bg_stroke = egui::Stroke::NONE;
            w.hovered.weak_bg_fill = Color32::from_white_alpha(35);
            w.active.weak_bg_fill = Color32::from_white_alpha(55);
            w.open.weak_bg_fill = Color32::from_white_alpha(55);
            ui.menu_button(t("File"), |ui| {
                if ui.button("Nuovo messaggio").clicked() {
                    self.new_mail();
                    ui.close();
                }
                if ui.button("Esporta messaggio").clicked() {
                    self.export_selected();
                    ui.close();
                }
                if ui.button("Esporta tutti quelli in elenco").clicked() {
                    self.open_export(false);
                    ui.close();
                }
                ui.separator();
                if ui.button("Esci").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(t("Messaggio"), |ui| {
                if ui.button("Rispondi").clicked() {
                    self.reply_or_forward(false);
                    ui.close();
                }
                if ui.button("Inoltra").clicked() {
                    self.reply_or_forward(true);
                    ui.close();
                }
                if ui.button("Preferito sì/no").clicked() {
                    self.toggle_star();
                    ui.close();
                }
                if ui.button("Elimina").clicked() {
                    self.ask_delete();
                    ui.close();
                }
                if ui.button("Ripristina dal cestino").clicked() {
                    self.restore();
                    ui.close();
                }
                if ui.button("Svuota il cestino").clicked() {
                    self.confirm = Some(Confirm::EmptyTrash);
                    ui.close();
                }
            });
            ui.menu_button(t("Strumenti"), |ui| {
                if ui.button("Account").clicked() {
                    self.show_accounts = true;
                    ui.close();
                }
                if ui.button("Statistiche").clicked() {
                    self.stats = Some(StatsWin::default());
                    ui.close();
                }
                if ui.button("Registro").clicked() {
                    self.show_log = true;
                    ui.close();
                }
                ui.separator();
                if ui
                    .button(if self.cfg.dark_mode {
                        "Tema chiaro"
                    } else {
                        "Tema scuro"
                    })
                    .clicked()
                {
                    self.cfg.dark_mode = !self.cfg.dark_mode;
                    theme::apply(&ctx, self.cfg.dark_mode);
                    self.save_cfg();
                    ui.close();
                }
            });
            ui.menu_button(t("Aiuto"), |ui| {
                ui.label(format!("{APP_TITLE} {APP_VERSION}"));
                ui.label("Client di posta scritto in Rust");
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        let ctx = ui.ctx().clone();
        egui::Frame::new()
            .fill(p.toolbar)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                self.menubar(ui);
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    let has_sel = !self.selected.is_empty();
                    if icon_button(ui, Icon::Receive, "Ricevi", &p, !self.syncing) {
                        self.refresh(&ctx);
                    }
                    if icon_button(ui, Icon::Compose, "Scrivi", &p, true) {
                        self.new_mail();
                    }
                    ui.add_space(6.0);
                    if icon_button(ui, Icon::Reply, "Rispondi", &p, self.preview.is_some()) {
                        self.reply_or_forward(false);
                    }
                    if icon_button(ui, Icon::Forward, "Inoltra", &p, self.preview.is_some()) {
                        self.reply_or_forward(true);
                    }
                    ui.add_space(6.0);
                    if icon_button(ui, Icon::Star, "Preferito", &p, has_sel) {
                        self.toggle_star();
                    }
                    if icon_button(ui, Icon::Delete, "Elimina", &p, has_sel) {
                        self.ask_delete();
                    }
                    if self.folder == TRASH
                        && icon_button(ui, Icon::Restore, "Ripristina", &p, has_sel)
                    {
                        self.restore();
                    }
                    ui.add_space(6.0);
                    if icon_button(ui, Icon::Accounts, "Account", &p, true) {
                        self.show_accounts = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .desired_width(220.0)
                                .hint_text("Cerca nei messaggi…"),
                        );
                        if r.changed() {
                            self.search_changed = Some(Instant::now());
                        }
                    });
                });
            });
    }

    fn side(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        ui.add_space(6.0);
        let mut nodes: Vec<(Option<usize>, String)> = Vec::new();
        if self.accounts.len() != 1 {
            nodes.push((
                None,
                if self.accounts.is_empty() {
                    "Cartelle locali".into()
                } else {
                    "Tutti gli account".into()
                },
            ));
        }
        nodes.extend(
            self.accounts
                .iter()
                .enumerate()
                .map(|(i, a)| (Some(i), a.email.clone())),
        );
        let mut pick: Option<(Option<usize>, usize)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (acc, name) in &nodes {
                    let id = acc.map_or(usize::MAX, |i| i);
                    let unread = self.unread.get(acc).copied().unwrap_or(0);
                    let title = RichText::new(name).strong().color(if unread > 0 {
                        p.unread
                    } else {
                        p.text
                    });
                    egui::CollapsingHeader::new(title)
                        .id_salt(("tree", id))
                        .default_open(true)
                        .show(ui, |ui| {
                            for (j, f) in FOLDERS.iter().enumerate() {
                                let mut label = (*f).to_string();
                                if j == 0 && unread > 0 {
                                    label = format!("{f} ({unread})");
                                }
                                let sel = self.account_sel == *acc && self.folder == j;
                                let text = RichText::new(label).color(if j == 0 && unread > 0 {
                                    p.unread
                                } else {
                                    p.text
                                });
                                if ui.selectable_label(sel, text).clicked() {
                                    pick = Some((*acc, j));
                                }
                            }
                        });
                }
            });
        if let Some((a, f)) = pick {
            self.account_sel = a;
            self.folder = f;
            self.selected.clear();
            self.current = None;
            self.preview = None;
            self.reload();
        }
    }

    fn sort_rows(&mut self) {
        let (col, asc) = (self.sort_col, self.sort_asc);
        let sent = self.folder == 1;
        self.emails.sort_by(|a, b| {
            let o = match col {
                0 => (if sent { &a.to } else { &a.from })
                    .to_lowercase()
                    .cmp(&(if sent { &b.to } else { &b.from }).to_lowercase()),
                1 => a.subject.to_lowercase().cmp(&b.subject.to_lowercase()),
                3 => a.size.cmp(&b.size),
                _ => a.timestamp.cmp(&b.timestamp),
            };
            if asc {
                o
            } else {
                o.reverse()
            }
        });
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        let total = self.emails.len();
        let sent_folder = self.folder == 1;
        let row_h = 24.0;
        let icons_w = 104.0;
        let (date_w, size_w) = (122.0, 70.0);
        // Intestazione a colonne
        let (hrect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::hover());
        ui.painter().rect_filled(hrect, 0.0, p.accent_light);
        ui.painter().line_segment(
            [hrect.left_bottom(), hrect.right_bottom()],
            egui::Stroke::new(1.0, p.border),
        );
        let w = hrect.width();
        let from_w = (w * 0.26).clamp(120.0, 300.0);
        let x_from = hrect.left() + icons_w;
        let x_subj = x_from + from_w;
        let x_date = hrect.right() - date_w - size_w - 8.0;
        let x_size = hrect.right() - size_w - 8.0;
        let cols = [
            (
                0usize,
                x_from,
                x_subj - 4.0,
                if sent_folder { "A" } else { "Da" },
            ),
            (1, x_subj, x_date - 4.0, "Oggetto"),
            (2, x_date, x_size - 4.0, "Data"),
            (3, x_size, hrect.right() - 4.0, "Dim."),
        ];
        let mut sort_click = None;
        for (id, x0, x1, label) in cols {
            let r = egui::Rect::from_min_max(
                egui::pos2(x0 - 4.0, hrect.top()),
                egui::pos2(x1, hrect.bottom()),
            );
            let resp = ui.interact(r, ui.id().with(("hdr", id)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, p.selected);
            }
            ui.painter().text(
                egui::pos2(x0, hrect.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(13.0),
                p.text,
            );
            if self.sort_col == id {
                let c = egui::pos2(x0 + 14.0 + label.len() as f32 * 7.0, hrect.center().y);
                let up = self.sort_asc;
                let pts = if up {
                    vec![
                        c + egui::vec2(-4.0, 3.0),
                        c + egui::vec2(4.0, 3.0),
                        c + egui::vec2(0.0, -3.0),
                    ]
                } else {
                    vec![
                        c + egui::vec2(-4.0, -3.0),
                        c + egui::vec2(4.0, -3.0),
                        c + egui::vec2(0.0, 3.0),
                    ]
                };
                ui.painter().add(egui::Shape::convex_polygon(
                    pts,
                    p.text_secondary,
                    egui::Stroke::NONE,
                ));
            }
            if resp.clicked() {
                sort_click = Some(id);
            }
        }
        if let Some(c) = sort_click {
            if self.sort_col == c {
                self.sort_asc = !self.sort_asc;
            } else {
                self.sort_col = c;
                self.sort_asc = c != 2 && c != 3;
            }
            self.sort_rows();
        }
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, total, |ui, range| {
                for i in range {
                    let Some(e) = self.emails.get(i) else {
                        continue;
                    };
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        Sense::click(),
                    );
                    let sel = self.selected.contains(&e.id);
                    let bg = if sel {
                        p.selected
                    } else if resp.hovered() {
                        p.accent_light
                    } else if i % 2 == 1 {
                        p.stripe
                    } else {
                        p.list_bg
                    };
                    ui.painter().rect_filled(rect, 0.0, bg);
                    let col = if e.is_read { p.text_secondary } else { p.text };
                    let font = FontId::proportional(14.0);
                    let mid = rect.left_center();
                    draw_envelope(
                        ui.painter(),
                        egui::pos2(mid.x + 14.0, mid.y),
                        6.0,
                        if e.is_read { p.text_muted } else { p.unread },
                        !e.is_read,
                    );
                    if e.is_starred {
                        draw_star(ui, egui::pos2(mid.x + 36.0, mid.y), 6.5, p.star);
                    }
                    if e.has_attachments {
                        draw_clip(
                            ui.painter(),
                            egui::pos2(mid.x + 56.0, mid.y),
                            p.text_secondary,
                        );
                    }
                    if e.is_pec {
                        ui.painter().text(
                            egui::pos2(mid.x + 70.0, mid.y),
                            Align2::LEFT_CENTER,
                            "PEC",
                            FontId::proportional(11.0),
                            p.pec,
                        );
                    }
                    let rw = rect.width();
                    let from_w = (rw * 0.26).clamp(120.0, 300.0);
                    let x0 = rect.left() + icons_w;
                    let who = if sent_folder { &e.to } else { &e.from };
                    let clip = egui::Rect::from_min_max(
                        egui::pos2(x0, rect.top()),
                        egui::pos2(x0 + from_w - 8.0, rect.bottom()),
                    );
                    ui.painter().with_clip_rect(clip).text(
                        egui::pos2(x0, mid.y),
                        Align2::LEFT_CENTER,
                        who,
                        font.clone(),
                        col,
                    );
                    let sx = x0 + from_w;
                    let dx = rect.right() - date_w - size_w - 8.0;
                    let clip = egui::Rect::from_min_max(
                        egui::pos2(sx, rect.top()),
                        egui::pos2(dx - 4.0, rect.bottom()),
                    );
                    ui.painter().with_clip_rect(clip).text(
                        egui::pos2(sx, mid.y),
                        Align2::LEFT_CENTER,
                        &e.subject,
                        font.clone(),
                        col,
                    );
                    ui.painter().text(
                        egui::pos2(dx, mid.y),
                        Align2::LEFT_CENTER,
                        fmt_date(e.timestamp, &e.date),
                        FontId::proportional(12.5),
                        p.text_muted,
                    );
                    ui.painter().text(
                        egui::pos2(rect.right() - 8.0, mid.y),
                        Align2::RIGHT_CENTER,
                        human_size(e.size),
                        FontId::proportional(12.5),
                        p.text_muted,
                    );
                    if resp.clicked() {
                        clicked = Some((i, ui.input(|s| s.modifiers)));
                    }
                }
            });
        if let Some((i, m)) = clicked {
            self.click_row(i, m);
        }
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        let Some(pv) = &self.preview else {
            ui.add_space(10.0);
            ui.label(RichText::new("Nessuna email selezionata").color(p.text_muted));
            return;
        };
        let row = pv.full.row.clone();
        let atts = pv.atts.clone();
        let mut save: Option<usize> = None;
        let mut save_all = false;
        egui::Frame::new()
            .fill(p.sidebar)
            .stroke(egui::Stroke::new(1.0, p.border))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(&row.subject).strong().size(16.0));
                egui::Grid::new("hdr_grid")
                    .num_columns(2)
                    .spacing([10.0, 2.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Da:").color(p.text_secondary));
                        ui.label(&row.from);
                        ui.end_row();
                        ui.label(RichText::new("A:").color(p.text_secondary));
                        ui.label(&row.to);
                        ui.end_row();
                        ui.label(RichText::new("Data:").color(p.text_secondary));
                        ui.label(fmt_date(row.timestamp, &row.date));
                        ui.end_row();
                    });
                if !atts.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Allegati:").color(p.text_secondary));
                        for (i, a) in atts.iter().enumerate() {
                            if ui
                                .small_button(format!("{} ({})", a.filename, human_size(a.size)))
                                .clicked()
                            {
                                save = Some(i);
                            }
                        }
                        if atts.len() > 1 && ui.small_button("Salva tutti").clicked() {
                            save_all = true;
                        }
                    });
                }
            });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(pv) = &self.preview {
                    let mut s: &str = &pv.text;
                    ui.add(
                        egui::TextEdit::multiline(&mut s)
                            .desired_width(f32::INFINITY)
                            .frame(egui::Frame::NONE),
                    );
                }
            });
        if let Some(i) = save {
            if let Some(a) = atts.get(i) {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.dirs.attachments)
                    .set_file_name(a.filename.clone())
                    .save_file()
                {
                    self.status = match self.db.attachment_data(a.id) {
                        Ok(Some((_, data))) => match std::fs::write(&path, data) {
                            Ok(()) => format!("Allegato salvato in {}", path.display()),
                            Err(e) => format!("Errore: {e}"),
                        },
                        _ => "Allegato non disponibile".into(),
                    };
                }
            }
        }
        if save_all {
            self.save_attachments();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Receive,
    Compose,
    Reply,
    Forward,
    Delete,
    Star,
    Accounts,
    Restore,
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, p: &Palette, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(66.0, 56.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && resp.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_white_alpha(30));
    }
    let col = if enabled {
        p.toolbar_text
    } else {
        p.toolbar_sub.gamma_multiply(0.45)
    };
    draw_icon(
        ui.painter(),
        icon,
        egui::pos2(rect.center().x, rect.top() + 21.0),
        col,
        enabled,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 9.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        col,
    );
    enabled && resp.clicked()
}

fn draw_envelope(painter: &egui::Painter, c: egui::Pos2, r: f32, color: Color32, filled: bool) {
    let rect = egui::Rect::from_center_size(c, egui::vec2(r * 2.0, r * 1.45));
    let st = egui::Stroke::new(1.3, color);
    if filled {
        painter.rect_filled(rect, 1.5, color);
        painter.line_segment(
            [rect.left_top(), c + egui::vec2(0.0, 1.0)],
            egui::Stroke::new(1.2, Color32::from_black_alpha(90)),
        );
        painter.line_segment(
            [rect.right_top(), c + egui::vec2(0.0, 1.0)],
            egui::Stroke::new(1.2, Color32::from_black_alpha(90)),
        );
    } else {
        painter.rect_stroke(rect, 1.5, st, egui::StrokeKind::Inside);
        painter.line_segment([rect.left_top(), c + egui::vec2(0.0, 1.0)], st);
        painter.line_segment([rect.right_top(), c + egui::vec2(0.0, 1.0)], st);
    }
}

fn draw_clip(painter: &egui::Painter, c: egui::Pos2, color: Color32) {
    let st = egui::Stroke::new(1.3, color);
    let r = egui::Rect::from_center_size(c, egui::vec2(6.0, 12.0));
    painter.rect_stroke(r, 3.0, st, egui::StrokeKind::Inside);
    painter.line_segment([c + egui::vec2(0.0, -3.0), c + egui::vec2(0.0, 4.0)], st);
}

fn arrow_head(painter: &egui::Painter, tip: egui::Pos2, dir: egui::Vec2, color: Color32) {
    let n = egui::vec2(-dir.y, dir.x);
    let pts = vec![tip, tip - dir * 6.0 + n * 4.5, tip - dir * 6.0 - n * 4.5];
    painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
}

fn draw_icon(painter: &egui::Painter, icon: Icon, c: egui::Pos2, color: Color32, enabled: bool) {
    let st = egui::Stroke::new(1.8, color);
    let gold = if enabled {
        Color32::from_rgb(255, 205, 70)
    } else {
        color
    };
    let v = |x: f32, y: f32| c + egui::vec2(x, y);
    match icon {
        Icon::Receive => {
            draw_envelope(painter, v(0.0, 2.0), 11.0, color, false);
            painter.line_segment([v(8.0, -12.0), v(8.0, -3.0)], egui::Stroke::new(2.4, gold));
            arrow_head(painter, v(8.0, 0.0), egui::vec2(0.0, 1.0), gold);
        }
        Icon::Compose => {
            let r = egui::Rect::from_center_size(v(-2.0, 1.0), egui::vec2(18.0, 22.0));
            painter.rect_stroke(r, 2.0, st, egui::StrokeKind::Inside);
            for y in [-4.0, 1.0, 6.0] {
                painter.line_segment([v(-8.0, y), v(3.0, y)], egui::Stroke::new(1.2, color));
            }
            painter.line_segment([v(11.0, -11.0), v(1.0, 5.0)], egui::Stroke::new(3.0, gold));
        }
        Icon::Reply => {
            painter.line_segment([v(10.0, 8.0), v(10.0, -2.0)], st);
            painter.line_segment([v(10.0, -2.0), v(-3.0, -2.0)], st);
            arrow_head(painter, v(-11.0, -2.0), egui::vec2(-1.0, 0.0), gold);
        }
        Icon::Forward => {
            painter.line_segment([v(-10.0, 8.0), v(-10.0, -2.0)], st);
            painter.line_segment([v(-10.0, -2.0), v(3.0, -2.0)], st);
            arrow_head(painter, v(11.0, -2.0), egui::vec2(1.0, 0.0), gold);
        }
        Icon::Delete => {
            painter.line_segment([v(-10.0, -8.0), v(10.0, -8.0)], st);
            painter.line_segment([v(-3.0, -8.0), v(-3.0, -11.0)], st);
            painter.line_segment([v(3.0, -8.0), v(3.0, -11.0)], st);
            painter.line_segment([v(-3.0, -11.0), v(3.0, -11.0)], st);
            let pts = vec![v(-8.0, -6.0), v(8.0, -6.0), v(6.0, 11.0), v(-6.0, 11.0)];
            painter.add(egui::Shape::closed_line(pts, st));
            for x in [-2.5, 2.5] {
                painter.line_segment([v(x, -2.0), v(x, 7.0)], egui::Stroke::new(1.3, color));
            }
        }
        Icon::Star => {
            let pts: Vec<egui::Pos2> = (0..10)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32) / 5.0 - std::f32::consts::FRAC_PI_2;
                    let rad = if i % 2 == 0 { 12.0 } else { 5.4 };
                    v(rad * a.cos(), rad * a.sin() + 1.0)
                })
                .collect();
            painter.add(egui::Shape::convex_polygon(pts, gold, egui::Stroke::NONE));
        }
        Icon::Accounts => {
            painter.circle_stroke(v(0.0, -5.0), 5.5, st);
            let arc: Vec<egui::Pos2> = (0..=10)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32) / 10.0;
                    v(-11.0 * a.cos(), 12.0 - 11.0 * a.sin())
                })
                .collect();
            painter.add(egui::Shape::line(arc, st));
        }
        Icon::Restore => {
            painter.line_segment([v(0.0, 10.0), v(0.0, -4.0)], st);
            arrow_head(painter, v(0.0, -11.0), egui::vec2(0.0, -1.0), gold);
            painter.line_segment([v(-10.0, 4.0), v(-10.0, 11.0)], st);
            painter.line_segment([v(10.0, 4.0), v(10.0, 11.0)], st);
            painter.line_segment([v(-10.0, 11.0), v(10.0, 11.0)], st);
        }
    }
}

pub fn human_size(n: usize) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f < K {
        format!("{n} B")
    } else if f < K * K {
        format!("{:.0} KB", f / K)
    } else {
        format!("{:.1} MB", f / K / K)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain(&ctx);
        // Ricerca con ritardo di 300 ms.
        if let Some(t) = self.search_changed {
            if t.elapsed() >= Duration::from_millis(300) {
                self.search_changed = None;
                self.reload();
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        // Cartella input: ogni 5 secondi.
        if self.last_input_scan.elapsed() >= Duration::from_secs(5) {
            self.last_input_scan = Instant::now();
            let (db, dir, tx, c2) = (
                self.db.clone(),
                self.dirs.input.clone(),
                self.tx.clone(),
                ctx.clone(),
            );
            std::thread::spawn(move || {
                let n = service::import_dir(&dir, &db);
                if n > 0 {
                    let _ = tx.send(Msg::Imported(n));
                    c2.request_repaint();
                }
            });
        }
        // Aggiornamento automatico.
        if self.cfg.auto_refresh_minutes > 0
            && !self.syncing
            && !self.accounts.is_empty()
            && self.last_sync.elapsed()
                >= Duration::from_secs(u64::from(self.cfg.auto_refresh_minutes) * 60)
        {
            self.refresh(&ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        egui::Panel::top("toolbar")
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} messaggi, {} non letti",
                        self.emails.len(),
                        self.unread_inbox
                    ))
                    .color(self.p().text_secondary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.syncing {
                        if ui.small_button("Interrompi").clicked() {
                            self.sync_cancel.store(true, Ordering::Relaxed);
                        }
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.status).color(self.p().text_secondary));
                });
            });
        });
        egui::Panel::left("side")
            .default_size(215.0)
            .min_size(180.0)
            .resizable(true)
            .show(ui, |ui| self.side(ui));
        egui::Panel::bottom("preview")
            .default_size(300.0)
            .min_size(240.0)
            .resizable(true)
            .show(ui, |ui| self.preview_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.list(ui));
        self.dialogs(&ctx);
    }
}
