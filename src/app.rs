//! egui front-end: log in, scan your items, review the diff, apply.

use crate::ia::{self, Creds, Item};
use crate::replace;
use eframe::egui;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Pause between writes, so a large batch stays polite to archive.org.
const WRITE_DELAY: Duration = Duration::from_millis(500);

#[derive(PartialEq)]
enum LoginMode {
    Password,
    S3Keys,
}

#[derive(Clone, PartialEq)]
enum RowState {
    Pending,
    Working,
    Done,
    Skipped(String),
    Failed(String),
}

struct Row {
    item: Item,
    new_description: String,
    selected: bool,
    state: RowState,
}

enum Msg {
    LoginOk(Creds),
    LoginErr(String),
    ScanProgress(usize, usize),
    ScanOk(Vec<Item>),
    ScanErr(String),
    RowState(String, RowState),
    ApplyDone { ok: usize, failed: usize },
}

pub struct App {
    client: Arc<Client>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    mode: LoginMode,
    email: String,
    password: String,
    access: String,
    secret: String,

    creds: Option<Creds>,
    status: String,
    busy: bool,
    scan_progress: Option<(usize, usize)>,
    rows: Vec<Row>,
    scanned: bool,
    confirming: bool,
    auto_clear: bool,
    cleared: usize,
    log: Vec<String>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        Self {
            client: Arc::new(ia::client()),
            tx,
            rx,
            mode: LoginMode::Password,
            email: String::new(),
            password: String::new(),
            access: String::new(),
            secret: String::new(),
            creds: None,
            status: String::new(),
            busy: false,
            scan_progress: None,
            rows: Vec::new(),
            scanned: false,
            confirming: false,
            auto_clear: false,
            cleared: 0,
            log: Vec::new(),
        }
    }

    fn spawn<F>(&self, ctx: &egui::Context, job: F)
    where
        F: FnOnce(Sender<Msg>, Arc<Client>, egui::Context) + Send + 'static,
    {
        let (tx, client, ctx) = (self.tx.clone(), self.client.clone(), ctx.clone());
        thread::spawn(move || job(tx, client, ctx));
    }

    fn start_login(&mut self, ctx: &egui::Context) {
        self.busy = true;
        self.status = "Signing in...".into();
        let (email, password) = (self.email.trim().to_owned(), self.password.clone());
        let (access, secret) = (self.access.trim().to_owned(), self.secret.trim().to_owned());
        let by_password = self.mode == LoginMode::Password;

        self.spawn(ctx, move |tx, client, ctx| {
            let result = if by_password {
                ia::login(&client, &email, &password)
            } else {
                ia::login_with_keys(&client, &access, &secret)
            };
            let _ = tx.send(match result {
                Ok(c) => Msg::LoginOk(c),
                Err(e) => Msg::LoginErr(e),
            });
            ctx.request_repaint();
        });
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        let Some(creds) = self.creds.clone() else {
            return;
        };
        self.busy = true;
        self.scanned = false;
        self.rows.clear();
        self.cleared = 0;
        self.scan_progress = Some((0, 0));
        self.status = "Scanning your items...".into();

        self.spawn(ctx, move |tx, client, ctx| {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let result = ia::list_items(&client, &creds, |done, total| {
                let _ = progress_tx.send(Msg::ScanProgress(done, total));
                progress_ctx.request_repaint();
            });
            let _ = tx.send(match result {
                Ok(items) => Msg::ScanOk(items),
                Err(e) => Msg::ScanErr(e),
            });
            ctx.request_repaint();
        });
    }

    fn start_apply(&mut self, ctx: &egui::Context) {
        let Some(creds) = self.creds.clone() else {
            return;
        };
        let targets: Vec<String> = self
            .rows
            .iter()
            .filter(|r| r.selected && r.state != RowState::Done)
            .map(|r| r.item.identifier.clone())
            .collect();
        if targets.is_empty() {
            return;
        }

        self.busy = true;
        self.status = format!("Updating {} item(s)...", targets.len());
        for row in self.rows.iter_mut().filter(|r| targets.contains(&r.item.identifier)) {
            row.state = RowState::Pending;
        }

        self.spawn(ctx, move |tx, client, ctx| {
            let (mut ok, mut failed) = (0, 0);
            for id in targets {
                let _ = tx.send(Msg::RowState(id.clone(), RowState::Working));
                ctx.request_repaint();

                let state = match apply_one(&client, &creds, &id) {
                    Ok(Some(())) => {
                        ok += 1;
                        RowState::Done
                    }
                    Ok(None) => RowState::Skipped("already up to date".into()),
                    Err(e) => {
                        failed += 1;
                        RowState::Failed(e)
                    }
                };
                let _ = tx.send(Msg::RowState(id, state));
                ctx.request_repaint();
                thread::sleep(WRITE_DELAY);
            }
            let _ = tx.send(Msg::ApplyDone { ok, failed });
            ctx.request_repaint();
        });
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::LoginOk(c) => {
                    self.status = format!("Signed in as {}", c.screenname);
                    self.creds = Some(c);
                    self.password.clear();
                    self.busy = false;
                }
                Msg::LoginErr(e) => {
                    self.status = e;
                    self.busy = false;
                }
                Msg::ScanProgress(done, total) => self.scan_progress = Some((done, total)),
                Msg::ScanOk(items) => {
                    let total = items.len();
                    self.rows = items
                        .into_iter()
                        .filter(|i| replace::needs_fix(&i.description))
                        .map(|i| Row {
                            new_description: replace::fix(&i.description),
                            item: i,
                            selected: true,
                            state: RowState::Pending,
                        })
                        .collect();
                    self.status = format!(
                        "{} of your {} item(s) mention redump.org.",
                        self.rows.len(),
                        total
                    );
                    self.scan_progress = None;
                    self.scanned = true;
                    self.busy = false;
                }
                Msg::ScanErr(e) => {
                    self.status = format!("Scan failed: {e}");
                    self.scan_progress = None;
                    self.busy = false;
                }
                Msg::RowState(id, state) => {
                    // An item that is done or skipped needs no further work. With
                    // auto-clear on it leaves the list, so only outstanding items
                    // stay on screen - but it always leaves a line in the log, so
                    // nothing disappears without a record. Failures always stay
                    // visible so they can be retried.
                    let settled = settles(&state);
                    match &state {
                        RowState::Failed(e) => self.log.push(format!("FAILED {id}: {e}")),
                        RowState::Done if self.auto_clear => {
                            self.log.push(format!("fixed   {id}"))
                        }
                        RowState::Skipped(why) if self.auto_clear => {
                            self.log.push(format!("skipped {id} ({why})"))
                        }
                        _ => {}
                    }

                    if self.auto_clear && settled {
                        self.rows.retain(|r| r.item.identifier != id);
                        self.cleared += 1;
                    } else if let Some(row) =
                        self.rows.iter_mut().find(|r| r.item.identifier == id)
                    {
                        row.state = state;
                    }
                }
                Msg::ApplyDone { ok, failed } => {
                    self.status = if failed == 0 {
                        format!("Done - {ok} item(s) updated.")
                    } else {
                        format!("Done - {ok} updated, {failed} failed (see log).")
                    };
                    self.busy = false;
                }
            }
        }
    }
}

/// Whether an item has reached a state that needs no further work, and so may
/// be cleared from the list when "Clear when fixed" is on. Anything still in
/// flight or failed stays put.
fn settles(state: &RowState) -> bool {
    match state {
        RowState::Done | RowState::Skipped(_) => true,
        RowState::Pending | RowState::Working | RowState::Failed(_) => false,
    }
}

/// Re-read the item, fix its description, and write it back. Returns `Ok(None)`
/// when the live description no longer needs the fix. Reading fresh here means
/// we never write back a stale copy from the earlier scan.
fn apply_one(client: &Client, creds: &Creds, identifier: &str) -> Result<Option<()>, String> {
    let meta = ia::get_metadata(client, identifier)?;
    let current = meta
        .pointer("/metadata/description")
        .ok_or("item has no description")?;

    // A description is normally a string, but IA allows a list of strings.
    let fixed = match current {
        Value::String(s) => Value::String(replace::fix(s)),
        Value::Array(parts) => Value::Array(
            parts
                .iter()
                .map(|p| match p {
                    Value::String(s) => Value::String(replace::fix(s)),
                    other => other.clone(),
                })
                .collect(),
        ),
        _ => return Err("description is not text".into()),
    };

    if &fixed == current {
        return Ok(None);
    }
    ia::patch_description(client, creds, identifier, &fixed)?;
    Ok(Some(()))
}

/// A short before/after window around the first change, for the preview.
fn snippet(text: &str) -> String {
    const PAD: usize = 45;
    match text.to_lowercase().find("redump.") {
        Some(hit) => {
            let start = text[..hit].char_indices().rev().nth(PAD).map_or(0, |(i, _)| i);
            let end = text[hit..]
                .char_indices()
                .nth(PAD)
                .map_or(text.len(), |(i, _)| hit + i);
            format!(
                "{}{}{}",
                if start > 0 { "..." } else { "" },
                &text[start..end],
                if end < text.len() { "..." } else { "" }
            )
        }
        None => text.chars().take(PAD * 2).collect(),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain();
        let ctx = ui.ctx().clone();

        egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(crate::APP_NAME);
                ui.label(egui::RichText::new("redump.org  ->  redump.info").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(who) = self.creds.as_ref().map(|c| c.screenname.clone()) {
                        if ui.button("Sign out").clicked() {
                            self.creds = None;
                            self.rows.clear();
                            self.scanned = false;
                            self.status.clear();
                        }
                        ui.label(format!("Signed in as {who}"));
                    }
                });
            });
            ui.add_space(6.0);
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.busy {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
            if let Some((done, total)) = self.scan_progress {
                let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .text(format!("{done} / {total} items"))
                        .desired_height(10.0),
                );
            }
            if !self.log.is_empty() {
                egui::CollapsingHeader::new(format!("Log ({})", self.log.len()))
                    .id_salt("log")
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(120.0)
                            .show(ui, |ui| {
                                for line in &self.log {
                                    ui.monospace(line);
                                }
                            });
                    });
            }
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if self.creds.is_none() {
                self.login_ui(ui, &ctx);
            } else {
                self.items_ui(ui, &ctx);
            }
        });

        if self.confirming {
            self.confirm_ui(&ctx);
        }
    }
}

impl App {
    fn login_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(420.0);
            ui.label("Sign in with your archive.org account.");
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, LoginMode::Password, "Email + password");
                ui.selectable_value(
                    &mut self.mode,
                    LoginMode::S3Keys,
                    "S3 keys (Advanced Only)",
                );
            });
            ui.add_space(12.0);

            let submit = match self.mode {
                LoginMode::Password => {
                    ui.horizontal(|ui| {
                        ui.label("Email  ");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.email)
                                .hint_text("you@example.com")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    let mut submit = false;
                    ui.horizontal(|ui| {
                        ui.label("Password");
                        submit = ui
                            .add(
                                egui::TextEdit::singleline(&mut self.password)
                                    .password(true)
                                    .desired_width(f32::INFINITY),
                            )
                            .lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    });
                    submit
                }
                LoginMode::S3Keys => {
                    ui.label(
                        egui::RichText::new("Get these from archive.org/account/s3.php")
                            .weak()
                            .small(),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Access");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.access)
                                .desired_width(f32::INFINITY),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Secret");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.secret)
                                .password(true)
                                .desired_width(f32::INFINITY),
                        );
                    });
                    false
                }
            };

            ui.add_space(16.0);
            let filled = match self.mode {
                LoginMode::Password => !self.email.trim().is_empty() && !self.password.is_empty(),
                LoginMode::S3Keys => {
                    !self.access.trim().is_empty() && !self.secret.trim().is_empty()
                }
            };
            let clicked = ui
                .add_enabled(!self.busy && filled, egui::Button::new("Sign in"))
                .clicked();
            if clicked || (submit && filled && !self.busy) {
                self.start_login(ctx);
            }

            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(
                    "Your password is exchanged for archive.org access keys and is never saved to disk.",
                )
                .weak()
                .small(),
            );
        });
    }

    fn items_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.busy, egui::Button::new("Scan my items"))
                .clicked()
            {
                self.start_scan(ctx);
            }
            ui.checkbox(&mut self.auto_clear, "Clear when fixed")
                .on_hover_text(
                    "Remove each item from this list once it no longer needs fixing.\n\
                     Failures stay put, and every cleared item is recorded in the log.",
                );
            if !self.rows.is_empty() {
                if ui.button("Select all").clicked() {
                    self.rows.iter_mut().for_each(|r| r.selected = true);
                }
                if ui.button("Select none").clicked() {
                    self.rows.iter_mut().for_each(|r| r.selected = false);
                }

                // Same rule as auto-clear, on demand: finished items go, failures
                // stay. Each one still leaves a line in the log.
                let completed = self.rows.iter().filter(|r| settles(&r.state)).count();
                if completed > 0
                    && ui
                        .button(format!("Clear completed ({completed})"))
                        .on_hover_text("Remove finished items from the list. Failures stay.")
                        .clicked()
                {
                    let notes: Vec<String> = self
                        .rows
                        .iter()
                        .filter(|r| settles(&r.state))
                        .map(|r| match &r.state {
                            RowState::Skipped(why) => {
                                format!("skipped {} ({why})", r.item.identifier)
                            }
                            _ => format!("fixed   {}", r.item.identifier),
                        })
                        .collect();
                    self.log.extend(notes);
                    self.rows.retain(|r| !settles(&r.state));
                    self.cleared += completed;
                }

                let n = self.rows.iter().filter(|r| r.selected).count();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !self.busy && n > 0,
                            egui::Button::new(format!("Apply to {n} item(s)")),
                        )
                        .clicked()
                    {
                        self.confirming = true;
                    }
                });
            }
        });
        ui.separator();

        if self.rows.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                // Distinguish "the scan found nothing" from "everything found has
                // since been fixed and cleared away".
                ui.label(match (self.scanned, self.cleared) {
                    (_, n) if n > 0 => {
                        format!("All done - {n} item(s) handled and cleared from the list.")
                    }
                    (true, _) => {
                        "Nothing to fix - none of your descriptions mention redump.org.".into()
                    }
                    (false, _) => {
                        "Press \"Scan my items\" to find descriptions that mention redump.org."
                            .into()
                    }
                });
            });
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in &mut self.rows {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut row.selected, "");
                    ui.monospace(&row.item.identifier);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match &row.state {
                            RowState::Pending => {}
                            RowState::Working => {
                                ui.spinner();
                            }
                            RowState::Done => {
                                ui.colored_label(egui::Color32::from_rgb(60, 160, 90), "updated");
                            }
                            RowState::Skipped(why) => {
                                ui.colored_label(egui::Color32::GRAY, why);
                            }
                            RowState::Failed(e) => {
                                ui.colored_label(egui::Color32::from_rgb(200, 70, 70), "failed")
                                    .on_hover_text(e);
                            }
                        }
                    });
                });

                egui::CollapsingHeader::new(
                    egui::RichText::new(if row.item.title.is_empty() {
                        "(untitled)"
                    } else {
                        &row.item.title
                    })
                    .small(),
                )
                .id_salt(&row.item.identifier)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("before").weak().small());
                    ui.monospace(snippet(&row.item.description));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("after").weak().small());
                    ui.colored_label(
                        egui::Color32::from_rgb(60, 160, 90),
                        egui::RichText::new(snippet(&row.new_description)).monospace(),
                    );
                });
                ui.separator();
            }
        });
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let n = self.rows.iter().filter(|r| r.selected).count();
        egui::Modal::new(egui::Id::new("confirm-apply"))
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.heading("Apply changes?");
                ui.add_space(8.0);
                ui.label(format!(
                    "This rewrites redump.org to redump.info in the description of {n} item(s) on archive.org."
                ));
                ui.label(
                    egui::RichText::new("Only descriptions are touched. This cannot be undone automatically.")
                        .weak()
                        .small(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.confirming = false;
                    }
                    if ui.button("Apply").clicked() {
                        self.confirming = false;
                        self.start_apply(ctx);
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_finished_items_are_cleared() {
        assert!(settles(&RowState::Done));
        assert!(settles(&RowState::Skipped("already up to date".into())));
    }

    #[test]
    fn failures_and_in_flight_items_are_never_cleared() {
        // A failure must stay on screen so it can be seen and retried.
        assert!(!settles(&RowState::Failed("403 forbidden".into())));
        assert!(!settles(&RowState::Pending));
        assert!(!settles(&RowState::Working));
    }

    #[test]
    fn snippet_shows_the_changed_region_of_a_long_description() {
        let long = format!("{}redump.org{}", "x".repeat(500), "y".repeat(500));
        let s = snippet(&long);
        assert!(s.contains("redump.org"), "snippet lost the match: {s}");
        assert!(s.len() < long.len(), "snippet did not shorten the text");
        assert!(s.starts_with("..."), "expected a leading ellipsis: {s}");
        assert!(s.ends_with("..."), "expected a trailing ellipsis: {s}");
    }

    #[test]
    fn snippet_handles_short_text_and_no_match() {
        assert_eq!(snippet("redump.org"), "redump.org");
        assert_eq!(snippet("no domain here"), "no domain here");
        assert_eq!(snippet(""), "");
    }
}
