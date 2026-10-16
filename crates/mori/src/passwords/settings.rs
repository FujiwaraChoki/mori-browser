//! Settings > Passwords: saved logins (search, reveal, copy, edit,
//! delete), importing a CSV export from another browser, exporting one,
//! and where the passwords are kept.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    *,
};
use mori_core::passwords::{
    Secret,
    csv::{export_csv, import_csv},
};
use uuid::Uuid;

use super::{Passwords, VaultStatus, vault};
use crate::{
    browser::{Browser, ToastAction, ToastStyle},
    theme::{ActiveTokens, metrics},
    ui::{
        components::{favicon, h_flex, icon, icon_button, text_button, v_flex},
        settings::{card, field, hint, section},
    },
};

/// Answers file choosers without a dialog (UI tests), like page file
/// choosers do: the path to import from or export to.
const TEST_FILE_CHOOSER_ENV: &str = "MORI_TEST_FILE_CHOOSER";
/// How long a revealed password stays visible.
const REVEAL_TIME: Duration = Duration::from_secs(30);
/// How long a copied password stays on the clipboard.
const CLIPBOARD_TIME: Duration = Duration::from_secs(60);
/// Biggest CSV file Mori reads.
const MAX_IMPORT_BYTES: u64 = 16 * 1024 * 1024;

/// A login being edited in place.
struct Editing {
    id: Uuid,
    username: Entity<InputState>,
    password: Entity<InputState>,
}

/// State of the Passwords settings page.
pub struct PasswordsPage {
    search: Entity<InputState>,
    /// Passwords the user chose to show, until they hide again.
    revealed: HashMap<Uuid, Secret>,
    editing: Option<Editing>,
    confirm_delete: Option<Uuid>,
    confirm_export: bool,
    /// An import or export is running.
    busy: bool,
}

impl Browser {
    fn password_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> &mut PasswordsPage {
        if self.passwords_ui.page.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search passwords"));
            let sub = cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            });
            self.push_subscription(sub);
            self.passwords_ui.page = Some(PasswordsPage {
                search,
                revealed: HashMap::new(),
                editing: None,
                confirm_delete: None,
                confirm_export: false,
                busy: false,
            });
            // Opening the page checks the keyring again (it may have
            // started since).
            Passwords::get(cx);
            Passwords::probe(cx);
        }
        self.passwords_ui.page.as_mut().expect("just created")
    }

    pub(crate) fn render_password_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.tokens().clone();
        let search = self.password_page(window, cx).search.clone();
        let query = search.read(cx).value().to_string();
        let p = Passwords::get(cx);
        let status = p.status.clone();
        let offer_to_save = p.store.offer_to_save;
        let total = p.store.credentials().len();
        let never: Vec<String> = p.store.never_save().map(str::to_string).collect();
        let logins: Vec<mori_core::passwords::Credential> = p.store.search(&query).into_iter().cloned().collect();
        let page = self.passwords_ui.page.as_ref().expect("made above");
        let (busy, confirm_export) = (page.busy, page.confirm_export);
        let unavailable = matches!(status, VaultStatus::Unavailable(_));

        let status_card = match &status {
            VaultStatus::Unavailable(reason) => Some(
                card(cx)
                    .border_color(t.status_warning.opacity(0.5))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(icon("shield-alert", px(16.), t.status_warning))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Passwords can't be saved")),
                    )
                    .child(div().text_color(t.foreground.opacity(0.85)).child(reason.clone()))
                    .child(hint(
                        "Mori keeps passwords in your system keyring, through the Secret Service that GNOME Keyring, \
                         KWallet and KeePassXC provide. Start one of them and Mori will use it.",
                        cx,
                    ))
                    .child(
                        h_flex().child(
                            text_button("passwords-recheck", "Check Again", false, cx)
                                .on_click(cx.listener(|_, _, _, cx| Passwords::probe(cx))),
                        ),
                    ),
            ),
            _ => None,
        };

        let rows: Vec<AnyElement> = logins
            .iter()
            .enumerate()
            .map(|(i, cred)| self.render_password_row(i, cred, cx))
            .collect();

        v_flex()
            .gap(px(22.))
            .children(status_card)
            .child(
                section("Saving", cx).child(
                    card(cx)
                        .child(field("Offer to save passwords", offer_toggle(offer_to_save, cx), cx))
                        .child(hint(
                            if unavailable {
                                "Unavailable until a keyring is running.".to_string()
                            } else {
                                "Passwords are encrypted in your system keyring; Mori's profile only lists the sites."
                                    .to_string()
                            },
                            cx,
                        ))
                        .when(!never.is_empty(), |c| {
                            c.child(div().pt_1().text_color(t.foreground.opacity(0.85)).child("Never saved for"))
                                .children(never.iter().enumerate().map(|(i, origin)| {
                                    let origin = origin.clone();
                                    h_flex()
                                        .h(px(28.))
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .child(mori_core::passwords::display_origin(&origin)),
                                        )
                                        .child(icon_button(("never-remove", i), "x", px(24.), true, cx).on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                let p = Passwords::get(cx);
                                                p.store.set_never_save(&origin, false);
                                                p.save();
                                                cx.notify();
                                            }),
                                        ))
                                }))
                        }),
                ),
            )
            .child(
                section(&format!("Saved passwords ({total})"), cx).child(
                    card(cx)
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(Input::new(&search).cleanable(true)))
                                .child(
                                    text_button("passwords-import", "Import…", false, cx)
                                        .flex_none()
                                        .when(busy || unavailable, |s| s.opacity(0.5))
                                        .when(!busy && !unavailable, |s| {
                                            s.on_click(cx.listener(|this, _, _, cx| this.import_passwords(cx)))
                                        }),
                                )
                                .child(
                                    text_button("passwords-export", "Export…", false, cx)
                                        .flex_none()
                                        .when(busy || total == 0 || unavailable, |s| s.opacity(0.5))
                                        .when(!busy && total > 0 && !unavailable, |s| {
                                            s.on_click(cx.listener(|this, _, _, cx| {
                                                if let Some(page) = &mut this.passwords_ui.page {
                                                    page.confirm_export = true;
                                                }
                                                cx.notify();
                                            }))
                                        }),
                                ),
                        )
                        .when(confirm_export, |c| {
                            c.child(
                                v_flex()
                                    .gap_2()
                                    .p_3()
                                    .rounded(metrics::RADIUS_LG)
                                    .bg(t.status_warning.opacity(0.10))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(icon("triangle-alert", px(14.), t.status_warning))
                                            .child(div().font_weight(FontWeight::MEDIUM).child("Export passwords as plain text?")),
                                    )
                                    .child(hint(
                                        "The file isn't encrypted: anyone who can open it can read every password in it. \
                                         Delete it once you've imported it elsewhere.",
                                        cx,
                                    ))
                                    .child(
                                        h_flex()
                                            .justify_end()
                                            .gap_2()
                                            .child(text_button("export-cancel", "Cancel", false, cx).on_click(cx.listener(
                                                |this, _, _, cx| {
                                                    if let Some(page) = &mut this.passwords_ui.page {
                                                        page.confirm_export = false;
                                                    }
                                                    cx.notify();
                                                },
                                            )))
                                            .child(text_button("export-confirm", "Export", true, cx).on_click(
                                                cx.listener(|this, _, _, cx| this.export_passwords(cx)),
                                            )),
                                    ),
                            )
                        })
                        .when(total == 0, |c| {
                            c.child(hint(
                                "No saved passwords yet. Mori offers to save one when you sign in to a site, or import \
                                 a CSV export from Chrome, Arc, Firefox, Safari or Bitwarden.",
                                cx,
                            ))
                        })
                        .when(total > 0 && rows.is_empty(), |c| c.child(hint("No passwords match.", cx)))
                        .children(rows),
                ),
            )
    }

    fn render_password_row(
        &mut self,
        i: usize,
        cred: &mori_core::passwords::Credential,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.tokens().clone();
        let id = cred.id;
        let host = cred.host();
        let icon_host = host.split(':').next().unwrap_or(&host).to_string();
        let fav = self.favicons.get(&icon_host);
        let page = self.passwords_ui.page.as_ref().expect("page exists while rendering it");
        // One line, even for a password with a line break in it (imports).
        let revealed = page.revealed.get(&id).map(|s| s.expose().replace(['\r', '\n'], "\u{21b5}"));
        let editing = page.editing.as_ref().filter(|e| e.id == id).map(|e| (e.username.clone(), e.password.clone()));
        let deleting = page.confirm_delete == Some(id);
        let username: SharedString = if cred.username.is_empty() { "No username".into() } else { cred.username.clone().into() };
        let identity = h_flex()
            .flex_1()
            .min_w_0()
            .gap(px(10.))
            .child(favicon(fav, px(16.), false, cx))
            .child(
                v_flex()
                    .min_w_0()
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(host.clone()))
                    .child(
                        div()
                            .truncate()
                            .text_size(metrics::TEXT_SMALL)
                            .text_color(t.muted_foreground)
                            .child(username),
                    ),
            );

        if let Some((user_input, pass_input)) = editing {
            return v_flex()
                .id(("password-row", i))
                .gap_2()
                .py_2()
                .border_t_1()
                .border_color(t.border.opacity(0.5))
                .child(identity)
                .child(field("Username", div().flex_1().child(Input::new(&user_input)), cx))
                .child(field("Password", div().flex_1().child(Input::new(&pass_input).mask_toggle()), cx))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(text_button(("edit-cancel", i), "Cancel", false, cx).on_click(cx.listener(|this, _, _, cx| {
                            if let Some(page) = &mut this.passwords_ui.page {
                                page.editing = None;
                            }
                            cx.notify();
                        })))
                        .child(text_button(("edit-save", i), "Save", true, cx).on_click(cx.listener(move |this, _, _, cx| {
                            let username = user_input.read(cx).value().trim().to_string();
                            let password = Secret::new(pass_input.read(cx).value().to_string());
                            this.save_password_edit(id, username, password, cx);
                        }))),
                )
                .into_any_element();
        }

        if deleting {
            return h_flex()
                .id(("password-row", i))
                .min_h(px(44.))
                .gap_2()
                .border_t_1()
                .border_color(t.border.opacity(0.5))
                .child(identity)
                .child(div().text_color(t.foreground.opacity(0.85)).child("Delete this password?"))
                .child(text_button(("delete-cancel", i), "Cancel", false, cx).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(page) = &mut this.passwords_ui.page {
                        page.confirm_delete = None;
                    }
                    cx.notify();
                })))
                .child(
                    text_button(("delete-confirm", i), "Delete", false, cx)
                        .bg(t.destructive.opacity(0.12))
                        .text_color(t.destructive)
                        .on_click(cx.listener(move |this, _, _, cx| this.delete_password(id, cx))),
                )
                .into_any_element();
        }

        let shown = revealed.is_some();
        let keyring = !matches!(Passwords::get(cx).status, VaultStatus::Unavailable(_));
        let secret_text: SharedString = revealed.map(SharedString::from).unwrap_or_else(|| "••••••••••".into());
        h_flex()
            .id(("password-row", i))
            .min_h(px(44.))
            .gap(px(6.))
            .border_t_1()
            .border_color(t.border.opacity(0.5))
            .child(identity)
            .child(
                div()
                    .w(px(150.))
                    .flex_none()
                    .truncate()
                    .text_color(t.foreground.opacity(0.8))
                    .when(shown, |s| s.font_family("monospace").text_size(metrics::TEXT_LABEL))
                    .child(secret_text),
            )
            // Showing, copying and editing need the keyring.
            .when(keyring, |row| {
                row.child(
                    icon_button(("password-reveal", i), if shown { "eye-off" } else { "eye" }, px(26.), true, cx)
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(if shown { "Hide" } else { "Show" })
                                .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_reveal(id, cx))),
                )
                .child(
                    icon_button(("password-copy", i), "copy", px(26.), true, cx)
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new("Copy Password").build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.copy_password(id, cx))),
                )
                .child(
                    icon_button(("password-edit", i), "pencil", px(26.), true, cx)
                        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Edit").build(window, cx))
                        .on_click(cx.listener(move |this, _, window, cx| this.start_password_edit(id, window, cx))),
                )
            })
            .child(
                icon_button(("password-delete", i), "trash", px(26.), true, cx)
                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Delete").build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(page) = &mut this.passwords_ui.page {
                            page.confirm_delete = Some(id);
                        }
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Show a password (fetched from the keyring) for a while, or hide it.
    fn toggle_reveal(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(page) = &mut self.passwords_ui.page
            && page.revealed.remove(&id).is_some()
        {
            cx.notify();
            return;
        }
        Passwords::vault_call(
            cx,
            move |ss| vault::fetch(ss, id),
            move |this, result, cx| match result {
                Ok(Some(secret)) => {
                    if let Some(page) = &mut this.passwords_ui.page {
                        page.revealed.insert(id, secret);
                    }
                    cx.notify();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(REVEAL_TIME).await;
                        let _ = this.update(cx, |this, cx| {
                            if let Some(page) = &mut this.passwords_ui.page {
                                page.revealed.remove(&id);
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
                Ok(None) => this.toast("This password is no longer in your keyring", ToastStyle::Error, None, cx),
                Err(e) => this.toast(e, ToastStyle::Error, None, cx),
            },
        );
    }

    fn copy_password(&mut self, id: Uuid, cx: &mut Context<Self>) {
        Passwords::vault_call(
            cx,
            move |ss| vault::fetch(ss, id),
            move |this, result, cx| match result {
                Ok(Some(secret)) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(secret.expose().to_string()));
                    this.toast("Password copied", ToastStyle::Success, None, cx);
                    // Don't leave it there for whatever reads the clipboard
                    // later, unless something else was copied since. (On
                    // Wayland this only works while a Mori window has the
                    // focus.)
                    cx.spawn(async move |_, cx| {
                        cx.background_executor().timer(CLIPBOARD_TIME).await;
                        cx.update(|cx| {
                            let mut current = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                            if current == secret.expose() {
                                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
                            }
                            zeroize::Zeroize::zeroize(&mut current);
                        });
                    })
                    .detach();
                }
                Ok(None) => this.toast("This password is no longer in your keyring", ToastStyle::Error, None, cx),
                Err(e) => this.toast(e, ToastStyle::Error, None, cx),
            },
        );
    }

    fn start_password_edit(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cred) = Passwords::get(cx).store.get(id).cloned() else { return };
        let username = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("No username");
            s.set_value(cred.username.clone(), window, cx);
            s
        });
        let password = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("Unchanged"));
        if let Some(page) = &mut self.passwords_ui.page {
            page.editing = Some(Editing { id, username, password });
            page.confirm_delete = None;
        }
        cx.notify();
    }

    /// Save an edited login: a new username, and a new password unless the
    /// field was left empty.
    fn save_password_edit(&mut self, id: Uuid, username: String, password: Secret, cx: &mut Context<Self>) {
        let p = Passwords::get(cx);
        let Some(cred) = p.store.get(id).cloned() else { return };
        if username != cred.username && p.store.find(&cred.origin, &username).is_some() {
            self.toast(format!("{} already has a password for “{username}”", cred.host()), ToastStyle::Warning, None, cx);
            return;
        }
        let origin = cred.origin.clone();
        let user = username.clone();
        let changed_password = !password.is_empty();
        Passwords::vault_call(
            cx,
            move |ss| {
                if changed_password {
                    vault::store(ss, id, &origin, &user, &password)
                } else {
                    vault::relabel(ss, id, &origin, &user)
                }
            },
            move |this, result, cx| {
                if let Err(e) = result {
                    this.toast(format!("Couldn't save the change. {e}"), ToastStyle::Error, None, cx);
                    return;
                }
                let p = Passwords::get(cx);
                p.store.set_username(id, &username);
                if changed_password {
                    p.store.mark_updated(id, mori_core::paths::now());
                }
                p.save();
                if let Some(page) = &mut this.passwords_ui.page {
                    page.editing = None;
                    page.revealed.remove(&id);
                }
                this.toast("Password saved", ToastStyle::Success, None, cx);
                cx.notify();
            },
        );
    }

    fn delete_password(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let forget = move |this: &mut Browser, cx: &mut Context<Browser>| {
            let p = Passwords::get(cx);
            p.store.remove(id);
            p.save();
            if let Some(page) = &mut this.passwords_ui.page {
                page.confirm_delete = None;
                page.revealed.remove(&id);
            }
            this.toast("Password deleted", ToastStyle::Info, None, cx);
            cx.notify();
        };
        // Without a keyring there is no secret to delete, only the entry.
        if matches!(Passwords::get(cx).status, VaultStatus::Unavailable(_)) {
            forget(self, cx);
            return;
        }
        Passwords::vault_call(cx, move |ss| vault::delete(ss, id), move |this, result, cx| match result {
            Ok(()) => forget(this, cx),
            Err(e) => this.toast(format!("Couldn't delete the password. {e}"), ToastStyle::Error, None, cx),
        });
    }

    fn set_passwords_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        if let Some(page) = &mut self.passwords_ui.page {
            page.busy = busy;
            page.confirm_export = false;
        }
        cx.notify();
    }

    /// Pick a CSV export and add its logins (replacing the passwords of
    /// logins Mori already has).
    fn import_passwords(&mut self, cx: &mut Context<Self>) {
        let test = test_file_choice();
        let dialog = test.is_none().then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Import".into()),
            })
        });
        self.set_passwords_busy(true, cx);
        cx.spawn(async move |this, cx| {
            let chosen = match dialog {
                Some(rx) => rx.await.ok().and_then(|r| r.ok()).flatten().and_then(|paths| paths.into_iter().next()),
                None => test,
            };
            let Some(path) = chosen else {
                let _ = this.update(cx, |this, cx| this.set_passwords_busy(false, cx));
                return;
            };
            let parsed = cx
                .background_spawn(async move {
                    let size = std::fs::metadata(&path).map(|m| m.len()).map_err(|e| e.to_string())?;
                    if size > MAX_IMPORT_BYTES {
                        return Err("The file is too big to be a password export.".to_string());
                    }
                    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                    let text = Secret::from_utf8(std::mem::take(&mut bytes)).ok_or("The file isn't text.")?;
                    import_csv(text.expose())
                })
                .await;
            let _ = this.update(cx, |this, cx| match parsed {
                Ok(import) => this.store_imported(import, cx),
                Err(e) => {
                    this.set_passwords_busy(false, cx);
                    this.toast(format!("Couldn't import: {e}"), ToastStyle::Error, None, cx);
                }
            });
        })
        .detach();
    }

    fn store_imported(&mut self, import: mori_core::passwords::csv::CsvImport, cx: &mut Context<Self>) {
        let p = Passwords::get(cx);
        // Logins Mori has keep their id: their password is replaced.
        let jobs: Vec<(Uuid, bool, mori_core::passwords::csv::ImportedLogin)> = import
            .logins
            .into_iter()
            .map(|login| match p.store.find(&login.origin, &login.username) {
                Some(existing) => (existing.id, true, login),
                None => (Uuid::new_v4(), false, login),
            })
            .collect();
        let skipped = import.skipped;
        Passwords::vault_call(
            cx,
            move |ss| {
                let mut stored = Vec::new();
                for (id, existing, login) in jobs {
                    match vault::store(ss, id, &login.origin, &login.username, &login.password) {
                        Ok(()) => stored.push((id, existing, login.origin, login.username, login.created)),
                        // The keyring is gone: report what made it.
                        Err(e) if stored.is_empty() => return Err(e),
                        Err(e) => {
                            log::warn!("passwords: import stopped: {e}");
                            break;
                        }
                    }
                }
                Ok(stored)
            },
            move |this, result, cx| {
                this.set_passwords_busy(false, cx);
                let stored = match result {
                    Ok(stored) => stored,
                    Err(e) => {
                        this.toast(format!("Couldn't import: {e}"), ToastStyle::Error, None, cx);
                        return;
                    }
                };
                let now = mori_core::paths::now();
                let p = Passwords::get(cx);
                let (mut added, mut updated) = (0, 0);
                for (id, existing, origin, username, created) in stored {
                    if existing {
                        p.store.mark_updated(id, now);
                        updated += 1;
                    } else {
                        p.store.add(id, &origin, &username, created.unwrap_or(now));
                        added += 1;
                    }
                }
                p.save();
                let mut text = match (added, updated) {
                    (0, 0) => "No passwords to import".to_string(),
                    (a, 0) => format!("Imported {a} password{}", if a == 1 { "" } else { "s" }),
                    (a, u) => format!("Imported {a}, updated {u}"),
                };
                if skipped > 0 {
                    text.push_str(&format!(" · {skipped} skipped"));
                }
                this.toast(text, ToastStyle::Success, None, cx);
            },
        );
    }

    /// Write every saved login to a CSV file the user picks.
    fn export_passwords(&mut self, cx: &mut Context<Self>) {
        let test = test_file_choice();
        let dialog = test.is_none().then(|| cx.prompt_for_new_path(&self.downloads_dir, Some("Mori Passwords.csv")));
        self.set_passwords_busy(true, cx);
        cx.spawn(async move |this, cx| {
            let chosen = match dialog {
                Some(rx) => rx.await.ok().and_then(|r| r.ok()).flatten(),
                None => test,
            };
            let Some(path) = chosen else {
                let _ = this.update(cx, |this, cx| this.set_passwords_busy(false, cx));
                return;
            };
            let _ = this.update(cx, |_, cx| {
                let logins: Vec<(Uuid, String, String)> = Passwords::get(cx)
                    .store
                    .credentials()
                    .iter()
                    .map(|c| (c.id, c.origin.clone(), c.username.clone()))
                    .collect();
                let ids: Vec<Uuid> = logins.iter().map(|(id, ..)| *id).collect();
                Passwords::vault_call(
                    cx,
                    move |ss| {
                        let secrets: HashMap<Uuid, Secret> = vault::fetch_many(ss, &ids)?.into_iter().collect();
                        let rows = logins
                            .iter()
                            .filter_map(|(id, origin, user)| secrets.get(id).map(|s| (origin.as_str(), user.as_str(), s)));
                        let csv = export_csv(rows);
                        write_private(&path, csv.expose().as_bytes()).map_err(|e| format!("Couldn't write the file: {e}"))?;
                        Ok((path, secrets.len()))
                    },
                    |this, result, cx| {
                        this.set_passwords_busy(false, cx);
                        match result {
                            Ok((path, n)) => this.toast(
                                format!("Exported {n} password{}", if n == 1 { "" } else { "s" }),
                                ToastStyle::Success,
                                Some(("Show".into(), ToastAction::Reveal(path))),
                                cx,
                            ),
                            Err(e) => this.toast(e, ToastStyle::Error, None, cx),
                        }
                    },
                );
            });
        })
        .detach();
    }
}

/// The "Offer to save passwords" switch (the setting lives in the
/// password store, not in Mori's settings).
fn offer_toggle(on: bool, cx: &mut Context<Browser>) -> Stateful<Div> {
    let t = cx.tokens().clone();
    div()
        .id("passwords-offer")
        .w(px(36.))
        .h(px(20.))
        .flex_none()
        .p(px(2.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.primary } else { t.foreground.opacity(0.15) })
        .child(div().size(px(16.)).rounded_full().bg(white()).shadow_sm().when(on, |s| s.ml(px(16.))))
        .on_click(cx.listener(move |_, _, _, cx| {
            let p = Passwords::get(cx);
            p.store.offer_to_save = !on;
            p.save();
            cx.notify();
        }))
}

fn test_file_choice() -> Option<PathBuf> {
    std::env::var_os(TEST_FILE_CHOOSER_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Write `bytes` to a new file only the user can read.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::{
        io::Write as _,
        os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    };
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    // `mode` only applies to a new file: also lock down one being replaced.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()
}
