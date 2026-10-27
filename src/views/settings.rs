use eframe::egui::{self, RichText};
use std::path::Path;
use std::process::Command;

use crate::app::{AMBER, App, GREEN, RED};
use crate::config::{ApiToken, Backend, LogSource, Settings, TlsMode, service_env_snippet};
use crate::ferron;
use crate::i18n::tr;
use crate::ollama::Client;
use crate::util::{generate_token, mask_token};
use crate::views::kit::{self, choice, panel, setting, switch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPane {
    Connection,
    Server,
    Memory,
    Security,
    Tokens,
    Logs,
    Application,
}

impl SettingsPane {
    const ALL: [SettingsPane; 7] = [
        SettingsPane::Connection,
        SettingsPane::Server,
        SettingsPane::Memory,
        SettingsPane::Security,
        SettingsPane::Tokens,
        SettingsPane::Logs,
        SettingsPane::Application,
    ];

    fn label(self) -> &'static str {
        match self {
            SettingsPane::Connection => ic!(CABLE, "Connection"),
            SettingsPane::Server => ic!(SERVER, "Server"),
            SettingsPane::Memory => ic!(GPU, "Models & memory"),
            SettingsPane::Security => ic!(LOCK, "Security"),
            SettingsPane::Tokens => ic!(KEY, "API tokens"),
            SettingsPane::Logs => ic!(LOGS, "Logs"),
            SettingsPane::Application => ic!(PALETTE, "Application"),
        }
    }
}

const INPUT_WIDTH: f32 = 380.0;

/// Checks that a load-balanced server answers, as the proxy will call it (no token).
fn test_backend(ctx: &egui::Context, tests: &BackendTests, url: String) {
    tests
        .lock()
        .unwrap()
        .insert(url.clone(), (false, "testing…".into()));
    let (tests, ctx) = (tests.clone(), ctx.clone());
    std::thread::spawn(move || {
        let conn = crate::config::Connection {
            base_url: url.clone(),
            ..Default::default()
        };
        let result = match Client::new(&conn).version() {
            Ok(v) => (true, trf!("OK — Ollama {version}", version = v)),
            Err(e) => (false, format!("{e:#}")),
        };
        tests.lock().unwrap().insert(url, result);
        ctx.request_repaint();
    });
}

/// Result of the last test of each load-balanced server, by URL.
pub type BackendTests =
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, (bool, String)>>>;

fn open_folder(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg(path).spawn()?;
    Ok(())
}

fn text(ui: &mut egui::Ui, value: &mut String, hint: &str) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(INPUT_WIDTH.min(ui.available_width())),
    )
}

/// Wide, tall button filling its column, for the main actions of a box.
fn big_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(18.0))
            .min_size(egui::vec2(ui.available_width(), 56.0)),
    )
}

/// DragValue where 0 means "let Ollama decide".
fn optional_count(ui: &mut egui::Ui, value: &mut u32, max: u32, zero: &'static str) {
    ui.add(
        egui::DragValue::new(value)
            .range(0..=max)
            .custom_formatter(move |v, _| {
                if v == 0.0 {
                    zero.into()
                } else {
                    format!("{v}")
                }
            }),
    );
}

impl App {
    pub fn save_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let dirty = self.draft_dirty();
            if ui
                .add_enabled(dirty, egui::Button::new(ic!(SAVE, "Save")))
                .clicked()
            {
                self.apply_draft(false);
            }
            let restart_label =
                if self.ctl.settings.server.manage_server || self.draft.server.manage_server {
                    ic!(RESTART, "Save & restart services")
                } else {
                    ic!(RESTART, "Save & restart proxy")
                };
            if ui
                .add_enabled(dirty, egui::Button::new(restart_label))
                .clicked()
            {
                self.apply_draft(true);
            }
            if ui
                .add_enabled(dirty, egui::Button::new(ic!(UNDO, "Revert")))
                .clicked()
            {
                self.draft = self.ctl.settings.clone();
            }
            if dirty {
                kit::badge(ui, tr("Unsaved changes"), AMBER);
            }
        });
    }

    pub fn settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(tr("Settings"));
            ui.add_space(24.0);
            self.save_buttons(ui);
        });
        ui.add_space(12.0);

        egui::Panel::left("settings_panes")
            .resizable(false)
            .exact_size(240.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                right: 18,
                ..Default::default()
            }))
            .show(ui, |ui| {
                kit::card_frame(ui).inner_margin(10.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for pane in SettingsPane::ALL {
                        let selected = self.settings_pane == pane;
                        let item = ui.horizontal(|ui| {
                            // API tokens is a subsection of Security.
                            if pane == SettingsPane::Tokens {
                                ui.add_space(20.0);
                            }
                            kit::nav_item(ui, selected, pane.label())
                        });
                        if item.inner.clicked() {
                            self.settings_pane = pane;
                        }
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 16,
                ..Default::default()
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| match self.settings_pane {
                        SettingsPane::Connection => self.connection_pane(ui),
                        SettingsPane::Server => self.server_pane(ui),
                        SettingsPane::Memory => self.memory_pane(ui),
                        SettingsPane::Security => self.managed_only(ui, Self::proxy_pane),
                        SettingsPane::Tokens => self.managed_only(ui, Self::tokens_pane),
                        SettingsPane::Logs => self.logs_pane(ui),
                        SettingsPane::Application => self.app_pane(ui),
                    });
            });
    }

    // ------------------------------------------------------------------

    fn connection_pane(&mut self, ui: &mut egui::Ui) {
        panel(
            ui,
            ic!(CABLE, "Ollama API endpoint"),
            Some(tr(
                "Point it at a remote proxy URL and token to manage another machine.",
            )),
            |ui| {
                let c = &mut self.draft.connection;
                setting(ui, "API URL", None, |ui| {
                    text(ui, &mut c.base_url, "http://127.0.0.1:11434");
                });
                setting(
                    ui,
                    tr("Bearer token"),
                    Some(tr("Only needed through the proxy")),
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut c.api_token)
                                .password(!self.reveal_tokens)
                                .desired_width(INPUT_WIDTH.min(ui.available_width() - 100.0)),
                        );
                        switch(ui, &mut self.reveal_tokens, tr("Show"));
                    },
                );
                setting(
                    ui,
                    tr("Self-signed certificates"),
                    Some(tr("Accept the proxy's self-signed certificate")),
                    |ui| switch(ui, &mut c.accept_invalid_certs, ""),
                );
            },
        );

        panel(ui, ic!(PLAY, "Quick setup"), None, |ui| {
            let local = format!("http://127.0.0.1:{}", self.draft.server.port);
            ui.columns(2, |cols| {
                if big_button(&mut cols[0], ic!(SERVER, "Use the local server"))
                    .on_hover_text(&local)
                    .clicked()
                {
                    self.draft.connection.base_url = local.clone();
                    self.draft.connection.api_token.clear();
                }
                let proxy = cols[1]
                    .add_enabled_ui(self.draft.proxy.enabled, |ui| {
                        big_button(ui, ic!(LOCK, "Use the HTTPS proxy"))
                    })
                    .inner
                    .on_disabled_hover_text(tr("Enable the proxy first"));
                if proxy.clicked() {
                    self.draft.connection.base_url = ferron::public_url(&self.draft);
                    self.draft.connection.accept_invalid_certs =
                        self.draft.proxy.tls == TlsMode::Internal;
                    if let Some(t) = self.draft.proxy.tokens.first() {
                        self.draft.connection.api_token = t.token.clone();
                    }
                }
            });
        });
        self.backends_pane(ui);
    }

    /// Shown on server option panes while the server is not managed by this app.
    fn unmanaged_notice(&self, ui: &mut egui::Ui) {
        if !self.draft.server.manage_server {
            kit::badge(
                ui,
                ic!(
                    INFO,
                    "Only used for a server started by this app: enable \"Manage the server\" in Server"
                ),
                AMBER,
            );
            ui.add_space(8.0);
        }
    }

    fn server_pane(&mut self, ui: &mut egui::Ui) {
        let proxy_enabled = self.draft.proxy.enabled;
        let managed = self.draft.server.manage_server;
        panel(
            ui,
            ic!(SERVER, "Process"),
            Some(tr(
                "Run `ollama serve` from this app, or monitor a server started elsewhere.",
            )),
            |ui| {
                let s = &mut self.draft.server;
                setting(
                    ui,
                    tr("Manage the server"),
                    Some(tr("Quit the tray app / stop the service first")),
                    |ui| switch(ui, &mut s.manage_server, ""),
                );
                ui.add_enabled_ui(s.manage_server, |ui| {
                    setting(
                        ui,
                        tr("ollama binary"),
                        Some(tr("Name in PATH or full path")),
                        |ui| {
                            text(ui, &mut s.ollama_path, "ollama");
                        },
                    );
                    setting(ui, tr("Start with the app"), None, |ui| {
                        switch(ui, &mut s.auto_start, "")
                    });
                    setting(ui, tr("Stop when the app exits"), None, |ui| {
                        switch(ui, &mut s.stop_on_exit, "")
                    });
                    setting(
                        ui,
                        tr("Restart on crash"),
                        Some(tr("At most 3 times a minute")),
                        |ui| switch(ui, &mut s.restart_on_crash, ""),
                    );
                });
            },
        );

        panel(ui, ic!(CABLE, "Network binding"), None, |ui| {
            let s = &mut self.draft.server;
            ui.add_enabled_ui(managed, |ui| {
                setting(ui, tr("Bind host"), Some("OLLAMA_HOST"), |ui| {
                    ui.add_enabled(
                        !proxy_enabled,
                        egui::TextEdit::singleline(&mut s.host).desired_width(180.0),
                    );
                    if ui
                        .selectable_label(s.host == "127.0.0.1", tr("Local only"))
                        .clicked()
                    {
                        s.host = "127.0.0.1".into();
                    }
                    if ui
                        .selectable_label(s.host == "0.0.0.0", tr("All interfaces"))
                        .clicked()
                    {
                        s.host = "0.0.0.0".into();
                    }
                });
            });
            if managed && proxy_enabled {
                kit::badge(ui, tr("Forced to 127.0.0.1 behind the proxy"), GREEN);
                ui.add_space(8.0);
            } else if managed && s.host != "127.0.0.1" && s.host != "localhost" {
                kit::badge(ui, ic!(WARNING, "Exposed without authentication"), AMBER);
                ui.add_space(8.0);
            }
            ui.add_enabled_ui(managed, |ui| {
                setting(
                    ui,
                    tr("Port"),
                    Some(tr("Also the HTTPS proxy's upstream")),
                    |ui| {
                        ui.add(egui::DragValue::new(&mut s.port).range(1..=65535));
                    },
                );
            });
        });

        self.unmanaged_notice(ui);
        ui.add_enabled_ui(managed, |ui| self.server_options(ui));

        let gpus = self.ctl.backend.shared.lock().unwrap().gpus.clone();
        let env = self.draft.ollama_env(&gpus);
        panel(
            ui,
            ic!(COPY, "System-installed Ollama"),
            Some(tr("Apply the same settings to the service or tray app:")),
            |ui| kit::code_block(ui, &service_env_snippet(&env)),
        );
    }

    /// Paths, access and extra environment of the managed server.
    fn server_options(&mut self, ui: &mut egui::Ui) {
        panel(ui, ic!(FOLDER, "Paths & access"), None, |ui| {
            let s = &mut self.draft.server;
            setting(ui, tr("Models directory"), Some("OLLAMA_MODELS"), |ui| {
                text(ui, &mut s.models_dir, tr("default (~/.ollama/models)"));
            });
            setting(
                ui,
                tr("Allowed origins"),
                Some(tr("OLLAMA_ORIGINS, comma separated")),
                |ui| {
                    text(ui, &mut s.origins, "http://localhost:3000,app://*");
                },
            );
            setting(ui, tr("Debug logging"), Some("OLLAMA_DEBUG=1"), |ui| {
                switch(ui, &mut s.debug_logging, "")
            });
        });

        panel(
            ui,
            ic!(TERMINAL, "Extra environment"),
            Some(tr("One KEY=VALUE per line, passed to `ollama serve`.")),
            |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.draft.server.extra_env)
                        .hint_text("OLLAMA_SCHED_SPREAD=1")
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            },
        );
    }

    fn memory_pane(&mut self, ui: &mut egui::Ui) {
        self.managed_only(ui, Self::memory_options);
    }

    /// Draws `pane` greyed out, with a notice, while the server is not managed by this app.
    fn managed_only(&mut self, ui: &mut egui::Ui, pane: fn(&mut Self, &mut egui::Ui)) {
        self.unmanaged_notice(ui);
        let managed = self.draft.server.manage_server;
        ui.add_enabled_ui(managed, |ui| pane(self, ui));
    }

    fn memory_options(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.draft.server;
        panel(
            ui,
            ic!(MODELS, "Context window"),
            Some(tr(
                "Default for every model (OLLAMA_CONTEXT_LENGTH). Larger needs more VRAM.",
            )),
            |ui| {
                ui.horizontal(|ui| {
                    crate::views::models::context_slider(ui, &mut s.context_length);
                });
            },
        );

        panel(ui, ic!(GPU, "Model loading"), None, |ui| {
            setting(
                ui,
                tr("Keep alive"),
                Some("-1 = forever, 0 = unload now"),
                |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.keep_alive).desired_width(80.0));
                    for preset in ["5m", "30m", "1h", "-1", "0"] {
                        if ui
                            .selectable_label(s.keep_alive == preset, preset)
                            .clicked()
                        {
                            s.keep_alive = preset.into();
                        }
                    }
                },
            );
            setting(
                ui,
                tr("Max loaded models"),
                Some("OLLAMA_MAX_LOADED_MODELS"),
                |ui| {
                    optional_count(ui, &mut s.max_loaded_models, 32, "auto");
                },
            );
            setting(
                ui,
                tr("Parallel requests"),
                Some(tr("OLLAMA_NUM_PARALLEL, per model")),
                |ui| {
                    optional_count(ui, &mut s.num_parallel, 64, "auto");
                },
            );
            setting(ui, tr("Max queue"), Some("OLLAMA_MAX_QUEUE"), |ui| {
                optional_count(ui, &mut s.max_queue, 10_000, "default");
            });
        });

        panel(ui, ic!(CHART, "Performance"), None, |ui| {
            setting(
                ui,
                tr("Flash attention"),
                Some(tr("Less VRAM for long contexts")),
                |ui| switch(ui, &mut s.flash_attention, ""),
            );
            ui.add_enabled_ui(s.flash_attention, |ui| {
                setting(
                    ui,
                    tr("KV cache type"),
                    Some(tr("Requires flash attention")),
                    |ui| {
                        choice(
                            ui,
                            &mut s.kv_cache_type,
                            &[
                                ("f16".to_string(), "f16"),
                                ("q8_0".to_string(), tr("q8_0 · ½")),
                                ("q4_0".to_string(), tr("q4_0 · ¼")),
                            ],
                        );
                    },
                );
            });
        });
    }

    /// HTTPS proxy and the API tokens it checks.
    fn backends_pane(&mut self, ui: &mut egui::Ui) {
        let p = &mut self.draft.proxy;
        let new_url = &mut self.new_backend_url;
        let tests = &self.backend_tests;
        if !p.enabled {
            kit::badge(
                ui,
                tr("Balanced by the proxy: enable it in Security"),
                AMBER,
            );
            ui.add_space(8.0);
        } else if !p.load_balancing {
            kit::badge(
                ui,
                tr("Load balancing is off in Security: the proxy ignores these servers"),
                AMBER,
            );
            ui.add_space(8.0);
        }
        ui.add_enabled_ui(p.enabled, |ui| {
            panel(
                ui,
                ic!(SERVER, "Load balancing"),
                Some(tr("The proxy sends requests to this machine's Ollama and the enabled servers below in turn (round robin). Each server needs the models clients ask for, and must accept requests without a token.")),
                |ui| {
                    let mut remove = None;
                    egui::Grid::new("backends")
                        .striped(true)
                        .num_columns(3)
                        .spacing([24.0, 12.0])
                        .show(ui, |ui| {
                            for (i, b) in p.backends.iter_mut().enumerate() {
                                switch(ui, &mut b.enabled, "");
                                ui.add_sized(
                                    [320.0, 30.0],
                                    egui::TextEdit::singleline(&mut b.url),
                                );
                                ui.horizontal(|ui| {
                                    let url = b.url.trim().to_string();
                                    if ui
                                        .add_enabled(b.is_valid(), egui::Button::new(ic!(TEST, "Test")))
                                        .on_hover_text(tr("Calls /api/version on this server, without a token"))
                                        .clicked()
                                    {
                                        test_backend(ui.ctx(), tests, url.clone());
                                    }
                                    if ui
                                        .button(RichText::new(ic!(DELETE, "Remove")).color(RED))
                                        .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                    if !b.is_valid() {
                                        kit::badge(ui, "Needs an http:// or https:// URL", AMBER);
                                    } else if let Some((ok, msg)) = tests.lock().unwrap().get(&url) {
                                        kit::badge(ui, msg.clone(), if *ok { GREEN } else { RED });
                                    }
                                });
                                ui.end_row();
                            }
                        });
                    if let Some(i) = remove {
                        p.backends.remove(i);
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(new_url)
                                .hint_text("http://192.168.1.20:11434")
                                .desired_width(300.0),
                        );
                        let backend = Backend {
                            url: new_url.trim().to_string(),
                            enabled: true,
                        };
                        if ui
                            .add_enabled(backend.is_valid(), egui::Button::new(ic!(ADD, "Add server")))
                            .clicked()
                        {
                            p.backends.push(backend);
                            new_url.clear();
                        }
                    });
                },
            );
        });
    }

    fn proxy_pane(&mut self, ui: &mut egui::Ui) {
        panel(
            ui,
            ic!(LOCK, "Ferron reverse proxy"),
            Some(tr(
                "Adds HTTPS and token checks in front of Ollama, which has no authentication.",
            )),
            |ui| {
                let p = &mut self.draft.proxy;
                setting(ui, tr("Enable the HTTPS proxy"), None, |ui| {
                    switch(ui, &mut p.enabled, "")
                });
                ui.add_enabled_ui(p.enabled, |ui| {
                    setting(
                        ui,
                        tr("ferron binary"),
                        Some(tr("Name in PATH or full path")),
                        |ui| {
                            text(ui, &mut p.ferron_path, "ferron");
                            ui.hyperlink_to("Get Ferron", "https://ferron.sh/download");
                        },
                    );
                    setting(ui, tr("Start with the app"), None, |ui| {
                        switch(ui, &mut p.auto_start, "")
                    });
                    setting(ui, tr("Listen address"), None, |ui| {
                        text(ui, &mut p.listen_host, "0.0.0.0");
                    });
                    setting(ui, tr("Port"), None, |ui| {
                        ui.add(egui::DragValue::new(&mut p.port).range(1..=65535));
                    });
                    setting(ui, tr("Domain / hostname"), None, |ui| {
                        text(ui, &mut p.domain, "ollama.example.com");
                    });
                    setting(
                        ui,
                        tr("CORS origin"),
                        Some(tr("Optional, for browser clients")),
                        |ui| {
                            text(ui, &mut p.cors_origin, "https://chat.example.com");
                        },
                    );
                    setting(
                        ui,
                        tr("Load balancing"),
                        Some(tr("Round robin with the servers in Connection; off proxies to this machine's Ollama only")),
                        |ui| switch(ui, &mut p.load_balancing, ""),
                    );
                });
            },
        );

        let enabled = self.draft.proxy.enabled;
        let self_signed = ferron::self_signed_paths(&self.draft).0;
        ui.add_enabled_ui(enabled, |ui| {
            panel(ui, ic!(KEY, "TLS certificate"), None, |ui| {
                let p = &mut self.draft.proxy;
                choice(
                    ui,
                    &mut p.tls,
                    &[
                        (TlsMode::Internal, tr("Self-signed")),
                        (TlsMode::Acme, tr("Let's Encrypt")),
                        (TlsMode::Custom, tr("My certificate")),
                        (TlsMode::Off, tr("Off (HTTP)")),
                    ],
                );
                ui.add_space(10.0);
                match p.tls {
                    TlsMode::Internal => {
                        ui.label(
                            RichText::new(trf!(
                                "Trust {path} on clients, or accept the certificate.",
                                path = self_signed.display()
                            ))
                            .weak(),
                        );
                    }
                    TlsMode::Acme => {
                        setting(
                            ui,
                            tr("ACME e-mail"),
                            Some(tr("Needs public DNS and ports 80/443")),
                            |ui| {
                                text(ui, &mut p.acme_email, "you@example.com");
                            },
                        );
                    }
                    TlsMode::Custom => {
                        setting(ui, tr("Certificate (PEM)"), None, |ui| {
                            text(ui, &mut p.cert_path, "/path/to/fullchain.pem");
                        });
                        setting(ui, tr("Private key (PEM)"), None, |ui| {
                            text(ui, &mut p.key_path, "/path/to/privkey.pem");
                        });
                    }
                    TlsMode::Off => {
                        kit::badge(
                            ui,
                            ic!(
                                WARNING,
                                "Tokens travel in clear text — trusted networks only"
                            ),
                            AMBER,
                        );
                    }
                }
            });
        });

        if enabled {
            let url = ferron::public_url(&self.draft);
            panel(ui, ic!(TEST, "Status & test"), None, |ui| {
                setting(ui, tr("Public URL"), None, |ui| {
                    ui.label(RichText::new(&url).monospace().strong());
                    if ui.button(icon!(COPY)).on_hover_text(tr("Copy")).clicked() {
                        ui.ctx().copy_text(url.clone());
                    }
                });
                self.proxy_test_ui(ui, &url);
                ui.add_space(10.0);
                ui.label(RichText::new(tr("Client example")).strong());
                let token = self
                    .draft
                    .proxy
                    .tokens
                    .first()
                    .map(|t| t.token.as_str())
                    .unwrap_or("<token>");
                let insecure = if self.draft.proxy.tls == TlsMode::Internal {
                    "-k "
                } else {
                    ""
                };
                kit::code_block(
                    ui,
                    &format!(
                        "curl {insecure}{url}/api/tags \\\n  -H \"Authorization: Bearer {token}\""
                    ),
                );
            });

            panel(
                ui,
                ic!(LOGS, "Generated Ferron configuration"),
                Some(&trf!(
                    "Written to {path}",
                    path = ferron::config_path().display()
                )),
                |ui| {
                    let mut content = ferron::generate(&self.draft);
                    if !self.reveal_tokens {
                        for t in &self.draft.proxy.tokens {
                            if !t.token.is_empty() {
                                content = content.replace(&t.token, &mask_token(&t.token));
                            }
                        }
                    }
                    kit::code_block(ui, &content);
                },
            );
        }
    }

    fn proxy_test_ui(&mut self, ui: &mut egui::Ui, url: &str) {
        ui.horizontal(|ui| {
            if ui
                .button(ic!(TEST, "Test proxy"))
                .on_hover_text(tr(
                    "Calls /api/version through the proxy with the first token",
                ))
                .clicked()
            {
                let conn = crate::config::Connection {
                    base_url: url.to_string(),
                    api_token: self
                        .ctl
                        .settings
                        .proxy
                        .tokens
                        .first()
                        .map(|t| t.token.clone())
                        .unwrap_or_default(),
                    accept_invalid_certs: self.ctl.settings.proxy.tls == TlsMode::Internal,
                };
                let slot = self.proxy_test.clone();
                *slot.lock().unwrap() = Some((false, "testing…".into()));
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let res = Client::new(&conn).version();
                    *slot.lock().unwrap() = Some(match res {
                        Ok(v) => (
                            true,
                            trf!("OK — Ollama {version} reachable with token", version = v),
                        ),
                        Err(e) => (false, format!("{e:#}")),
                    });
                    ctx.request_repaint();
                });
            }
            if let Some((ok, msg)) = self.proxy_test.lock().unwrap().clone() {
                kit::badge(ui, msg, if ok { GREEN } else { RED });
            }
        });
        if self.draft_dirty() {
            ui.label(
                RichText::new(tr(
                    "The test uses saved settings; save first to test your changes.",
                ))
                .weak(),
            );
        }
    }

    fn tokens_pane(&mut self, ui: &mut egui::Ui) {
        panel(
            ui,
            ic!(KEY, "Token authentication"),
            Some(tr(
                "Clients send `Authorization: Bearer <token>` to the HTTPS proxy.",
            )),
            |ui| {
                let p = &mut self.draft.proxy;
                setting(ui, tr("Require an API token"), None, |ui| {
                    switch(ui, &mut p.require_token, "")
                });
                if !p.enabled {
                    kit::badge(
                        ui,
                        tr("The HTTPS proxy is disabled — tokens are not enforced"),
                        AMBER,
                    );
                }
            },
        );

        let require = self.draft.proxy.require_token;
        // Toggled in the header, applied after the box is drawn (both closures borrow `self`).
        let mut reveal = self.reveal_tokens;
        ui.add_enabled_ui(require, |ui| {
            kit::panel_with_actions(
                ui,
                &trf!("Tokens ({count})", count = self.draft.proxy.tokens.len()),
                |ui| {
                    switch(ui, &mut reveal, tr("Reveal"));
                },
                |ui| {
                    let p = &mut self.draft.proxy;
                    if p.tokens.is_empty() {
                        kit::badge(
                            ui,
                            tr("No token yet: every request will be rejected"),
                            AMBER,
                        );
                        ui.add_space(8.0);
                    }
                    let mut remove = None;
                    egui::Grid::new("tokens")
                        .striped(true)
                        .num_columns(3)
                        .spacing([24.0, 12.0])
                        .show(ui, |ui| {
                            for (i, t) in p.tokens.iter_mut().enumerate() {
                                ui.add_sized(
                                    [200.0, 30.0],
                                    egui::TextEdit::singleline(&mut t.label),
                                );
                                let shown = if self.reveal_tokens {
                                    t.token.clone()
                                } else {
                                    mask_token(&t.token)
                                };
                                ui.label(RichText::new(shown).monospace());
                                ui.horizontal(|ui| {
                                    if ui.button(ic!(COPY, "Copy")).clicked() {
                                        ui.ctx().copy_text(t.token.clone());
                                    }
                                    if ui
                                        .button(icon!(REGENERATE))
                                        .on_hover_text(tr("Regenerate"))
                                        .clicked()
                                    {
                                        t.token = generate_token();
                                    }
                                    if ui
                                        .button(RichText::new(ic!(DELETE, "Revoke")).color(RED))
                                        .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                });
                                ui.end_row();
                            }
                        });
                    if let Some(i) = remove {
                        p.tokens.remove(i);
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_token_label)
                                .hint_text(tr("Label, e.g. laptop or open-webui"))
                                .desired_width(300.0),
                        );
                        if ui.button(ic!(ADD, "Generate token")).clicked() {
                            let label = if self.new_token_label.trim().is_empty() {
                                trf!("token {n}", n = p.tokens.len() + 1)
                            } else {
                                self.new_token_label.trim().to_string()
                            };
                            p.tokens.push(ApiToken {
                                label,
                                token: generate_token(),
                            });
                            self.new_token_label.clear();
                        }
                    });
                },
            );
        });
        self.reveal_tokens = reveal;
    }

    fn logs_pane(&mut self, ui: &mut egui::Ui) {
        let l = &mut self.draft.logs;
        panel(
            ui,
            ic!(LOGS, "Log source"),
            Some(tr("Where the Logs tab reads Ollama's output.")),
            |ui| {
                choice(
                    ui,
                    &mut l.source,
                    &[
                        (LogSource::Managed, tr("Managed server")),
                        (LogSource::File, tr("Log file")),
                        (LogSource::Journal, tr("systemd journal")),
                    ],
                );
                ui.add_space(10.0);
                match l.source {
                    LogSource::Managed => {
                        ui.label(
                            RichText::new(tr("Output of the server started by this app.")).weak(),
                        );
                    }
                    LogSource::File => {
                        setting(
                            ui,
                            tr("Log file"),
                            Some(tr("server.log of the tray app")),
                            |ui| {
                                text(ui, &mut l.file_path, tr("server.log path"));
                            },
                        );
                    }
                    LogSource::Journal => {
                        setting(ui, tr("systemd unit"), None, |ui| {
                            text(ui, &mut l.journal_unit, "ollama");
                        });
                    }
                }
            },
        );
        panel(ui, ic!(TUNE, "Buffer"), None, |ui| {
            setting(ui, tr("Lines kept in memory"), None, |ui| {
                ui.add(
                    egui::DragValue::new(&mut l.max_lines)
                        .range(500..=500_000)
                        .suffix(" lines"),
                );
            });
        });
    }

    fn app_pane(&mut self, ui: &mut egui::Ui) {
        panel(ui, ic!(PALETTE, "Appearance"), None, |ui| {
            setting(
                ui,
                tr("Interface scale"),
                Some(tr("Ctrl + / Ctrl − also zoom")),
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.ui.scale, 0.75..=2.5)
                            .step_by(0.05)
                            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
                    );
                },
            );
            setting(ui, tr("Presets"), None, |ui| {
                for preset in [1.0, 1.25, 1.5, 1.75, 2.0] {
                    if ui
                        .selectable_label(
                            (self.draft.ui.scale - preset).abs() < 0.01,
                            format!("{:.0} %", preset * 100.0),
                        )
                        .clicked()
                    {
                        self.draft.ui.scale = preset;
                    }
                }
            });
            setting(ui, tr("Language"), None, |ui| {
                let current = self.ctl.settings.ui.language.clone();
                let name = |code: &str| {
                    crate::i18n::LANGUAGES
                        .iter()
                        .find(|(c, _)| *c == code)
                        .map_or("", |(_, name)| *name)
                };
                let system = trf!(
                    "System ({language})",
                    language = name(crate::i18n::system_language())
                );
                let mut picked = current.clone();
                egui::ComboBox::from_id_salt("language")
                    .selected_text(if current.is_empty() {
                        system.clone()
                    } else {
                        name(&current).to_string()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut picked, String::new(), system);
                        for (code, native) in crate::i18n::LANGUAGES {
                            ui.selectable_value(&mut picked, code.to_string(), native);
                        }
                    });
                if picked != current {
                    // Applied immediately, like the theme.
                    self.ctl.settings.ui.language = picked.clone();
                    self.draft.ui.language = picked;
                    crate::i18n::set_language(&self.ctl.settings.ui.language);
                    let _ = self.ctl.settings.save();
                }
            });
            setting(ui, tr("Theme"), None, |ui| {
                let mut dark = self.ctl.settings.ui.dark_mode;
                choice(
                    ui,
                    &mut dark,
                    &[(true, ic!(DARK, "Dark")), (false, ic!(LIGHT, "Light"))],
                );
                if dark != self.ctl.settings.ui.dark_mode {
                    // Applied immediately, like the toggle in the top bar.
                    self.ctl.settings.ui.dark_mode = dark;
                    self.draft.ui.dark_mode = dark;
                    crate::app::apply_theme(ui.ctx(), dark);
                    let _ = self.ctl.settings.save();
                }
            });
        });

        panel(ui, ic!(SPEED, "Monitoring"), None, |ui| {
            let u = &mut self.draft.ui;
            setting(
                ui,
                tr("Refresh interval"),
                Some(tr("How often the API and metrics are polled")),
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut u.poll_interval_ms)
                            .range(250..=10_000)
                            .suffix(" ms"),
                    );
                },
            );
            setting(ui, tr("Chart history"), None, |ui| {
                ui.add(
                    egui::DragValue::new(&mut u.history_seconds)
                        .range(30..=3600)
                        .suffix(" s"),
                );
            });
        });

        panel(ui, ic!(FOLDER, "Configuration files"), None, |ui| {
            setting(ui, tr("Configuration folder"), None, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::config::config_dir().display().to_string())
                            .monospace(),
                    )
                    .wrap(),
                );
                if ui.button(tr("Open")).clicked()
                    && let Err(e) = open_folder(&crate::config::config_dir())
                {
                    self.ctl
                        .logs
                        .app_error(format!("Failed to open configuration folder: {e}"));
                }
            });
            setting(ui, tr("Data folder"), None, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::config::data_dir().display().to_string()).monospace(),
                    )
                    .wrap(),
                );
                if ui.button(tr("Open")).clicked()
                    && let Err(e) = open_folder(&crate::config::data_dir())
                {
                    self.ctl
                        .logs
                        .app_error(format!("Failed to open data folder: {e}"));
                }
            });
            setting(
                ui,
                tr("Reset"),
                Some(tr("Restores every default, including API tokens")),
                |ui| {
                    if !self.confirm_reset {
                        if ui.button(ic!(UNDO, "Reset all settings")).clicked() {
                            self.confirm_reset = true;
                        }
                    } else {
                        if ui
                            .button(RichText::new(tr("Yes, reset")).color(RED))
                            .clicked()
                        {
                            self.draft = Settings::default();
                            self.confirm_reset = false;
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            self.confirm_reset = false;
                        }
                    }
                },
            );
        });
    }
}
