use eframe::egui::{self, RichText};

use crate::app::{AMBER, App, GREEN, RED};
use crate::caddy;
use crate::config::{ApiToken, LogSource, Settings, TlsMode, service_env_snippet};
use crate::ollama::Client;
use crate::util::{generate_token, mask_token};
use crate::views::kit::{self, choice, panel, setting, switch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPane {
    Connection,
    Server,
    Memory,
    Proxy,
    Tokens,
    Logs,
    Application,
}

impl SettingsPane {
    const ALL: [SettingsPane; 7] = [
        SettingsPane::Connection,
        SettingsPane::Server,
        SettingsPane::Memory,
        SettingsPane::Proxy,
        SettingsPane::Tokens,
        SettingsPane::Logs,
        SettingsPane::Application,
    ];

    fn label(self) -> &'static str {
        match self {
            SettingsPane::Connection => ic!(CABLE, "Connection"),
            SettingsPane::Server => ic!(SERVER, "Server"),
            SettingsPane::Memory => ic!(GPU, "Models & memory"),
            SettingsPane::Proxy => ic!(LOCK, "HTTPS proxy"),
            SettingsPane::Tokens => ic!(KEY, "API tokens"),
            SettingsPane::Logs => ic!(LOGS, "Logs"),
            SettingsPane::Application => ic!(PALETTE, "Application"),
        }
    }
}

const INPUT_WIDTH: f32 = 380.0;

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
                if self.settings.server.manage_server || self.draft.server.manage_server {
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
                self.draft = self.settings.clone();
            }
            if dirty {
                kit::badge(ui, "Unsaved changes", AMBER);
            }
        });
    }

    pub fn settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Settings");
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
                        if kit::nav_item(ui, self.settings_pane == pane, pane.label()) {
                            self.settings_pane = pane;
                        }
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| match self.settings_pane {
                        SettingsPane::Connection => self.connection_pane(ui),
                        SettingsPane::Server => self.server_pane(ui),
                        SettingsPane::Memory => self.memory_pane(ui),
                        SettingsPane::Proxy => self.proxy_pane(ui),
                        SettingsPane::Tokens => self.tokens_pane(ui),
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
            Some("Point it at a remote proxy URL and token to manage another machine."),
            |ui| {
                let c = &mut self.draft.connection;
                setting(ui, "API URL", None, |ui| {
                    text(ui, &mut c.base_url, "http://127.0.0.1:11434");
                });
                setting(
                    ui,
                    "Bearer token",
                    Some("Only needed through the proxy"),
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut c.api_token)
                                .password(!self.reveal_tokens)
                                .desired_width(INPUT_WIDTH.min(ui.available_width() - 100.0)),
                        );
                        switch(ui, &mut self.reveal_tokens, "Show");
                    },
                );
                setting(
                    ui,
                    "Self-signed certificates",
                    Some("Accept Caddy's internal CA"),
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
                    .on_disabled_hover_text("Enable the proxy first");
                if proxy.clicked() {
                    self.draft.connection.base_url = caddy::public_url(&self.draft);
                    self.draft.connection.accept_invalid_certs =
                        self.draft.proxy.tls == TlsMode::Internal;
                    if let Some(t) = self.draft.proxy.tokens.first() {
                        self.draft.connection.api_token = t.token.clone();
                    }
                }
            });
        });
    }

    fn server_pane(&mut self, ui: &mut egui::Ui) {
        let proxy_enabled = self.draft.proxy.enabled;
        panel(
            ui,
            ic!(SERVER, "Process"),
            Some("Run `ollama serve` from this app, or monitor a server started elsewhere."),
            |ui| {
                let s = &mut self.draft.server;
                setting(
                    ui,
                    "Manage the server",
                    Some("Quit the tray app / stop the service first"),
                    |ui| switch(ui, &mut s.manage_server, ""),
                );
                ui.add_enabled_ui(s.manage_server, |ui| {
                    setting(
                        ui,
                        "ollama binary",
                        Some("Name in PATH or full path"),
                        |ui| {
                            text(ui, &mut s.ollama_path, "ollama");
                        },
                    );
                    setting(ui, "Start with the app", None, |ui| {
                        switch(ui, &mut s.auto_start, "")
                    });
                    setting(ui, "Stop when the app exits", None, |ui| {
                        switch(ui, &mut s.stop_on_exit, "")
                    });
                    setting(
                        ui,
                        "Restart on crash",
                        Some("At most 3 times a minute"),
                        |ui| switch(ui, &mut s.restart_on_crash, ""),
                    );
                });
            },
        );

        panel(ui, ic!(CABLE, "Network binding"), None, |ui| {
            let s = &mut self.draft.server;
            setting(ui, "Bind host", Some("OLLAMA_HOST"), |ui| {
                ui.add_enabled(
                    !proxy_enabled,
                    egui::TextEdit::singleline(&mut s.host).desired_width(180.0),
                );
                if ui
                    .selectable_label(s.host == "127.0.0.1", "Local only")
                    .clicked()
                {
                    s.host = "127.0.0.1".into();
                }
                if ui
                    .selectable_label(s.host == "0.0.0.0", "All interfaces")
                    .clicked()
                {
                    s.host = "0.0.0.0".into();
                }
            });
            if proxy_enabled {
                kit::badge(ui, "Forced to 127.0.0.1 behind the proxy", GREEN);
                ui.add_space(8.0);
            } else if s.host != "127.0.0.1" && s.host != "localhost" {
                kit::badge(ui, ic!(WARNING, "Exposed without authentication"), AMBER);
                ui.add_space(8.0);
            }
            setting(ui, "Port", None, |ui| {
                ui.add(egui::DragValue::new(&mut s.port).range(1..=65535));
            });
        });

        panel(ui, ic!(FOLDER, "Paths & access"), None, |ui| {
            let s = &mut self.draft.server;
            setting(ui, "Models directory", Some("OLLAMA_MODELS"), |ui| {
                text(ui, &mut s.models_dir, "default (~/.ollama/models)");
            });
            setting(
                ui,
                "Allowed origins",
                Some("OLLAMA_ORIGINS, comma separated"),
                |ui| {
                    text(ui, &mut s.origins, "http://localhost:3000,app://*");
                },
            );
            setting(ui, "Debug logging", Some("OLLAMA_DEBUG=1"), |ui| {
                switch(ui, &mut s.debug_logging, "")
            });
        });

        panel(
            ui,
            ic!(TERMINAL, "Extra environment"),
            Some("One KEY=VALUE per line, passed to `ollama serve`."),
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

        let gpus = self.backend.shared.lock().unwrap().gpus.clone();
        let env = self.draft.ollama_env(&gpus);
        panel(
            ui,
            ic!(COPY, "System-installed Ollama"),
            Some("Apply the same settings to the service or tray app:"),
            |ui| kit::code_block(ui, &service_env_snippet(&env)),
        );
    }

    fn memory_pane(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.draft.server;
        panel(
            ui,
            ic!(MODELS, "Context window"),
            Some("Default for every model (OLLAMA_CONTEXT_LENGTH). Larger needs more VRAM."),
            |ui| {
                ui.horizontal(|ui| {
                    crate::views::models::context_slider(ui, &mut s.context_length);
                });
            },
        );

        panel(ui, ic!(GPU, "Model loading"), None, |ui| {
            setting(
                ui,
                "Keep alive",
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
                "Max loaded models",
                Some("OLLAMA_MAX_LOADED_MODELS"),
                |ui| {
                    optional_count(ui, &mut s.max_loaded_models, 32, "auto");
                },
            );
            setting(
                ui,
                "Parallel requests",
                Some("OLLAMA_NUM_PARALLEL, per model"),
                |ui| {
                    optional_count(ui, &mut s.num_parallel, 64, "auto");
                },
            );
            setting(ui, "Max queue", Some("OLLAMA_MAX_QUEUE"), |ui| {
                optional_count(ui, &mut s.max_queue, 10_000, "default");
            });
        });

        panel(ui, ic!(CHART, "Performance"), None, |ui| {
            setting(
                ui,
                "Flash attention",
                Some("Less VRAM for long contexts"),
                |ui| switch(ui, &mut s.flash_attention, ""),
            );
            ui.add_enabled_ui(s.flash_attention, |ui| {
                setting(
                    ui,
                    "KV cache type",
                    Some("Requires flash attention"),
                    |ui| {
                        choice(
                            ui,
                            &mut s.kv_cache_type,
                            &[
                                ("f16".to_string(), "f16"),
                                ("q8_0".to_string(), "q8_0 · ½"),
                                ("q4_0".to_string(), "q4_0 · ¼"),
                            ],
                        );
                    },
                );
            });
        });
    }

    fn proxy_pane(&mut self, ui: &mut egui::Ui) {
        panel(
            ui,
            ic!(LOCK, "Caddy reverse proxy"),
            Some("Adds HTTPS and token checks in front of Ollama, which has no authentication."),
            |ui| {
                let p = &mut self.draft.proxy;
                setting(ui, "Enable the HTTPS proxy", None, |ui| {
                    switch(ui, &mut p.enabled, "")
                });
                ui.add_enabled_ui(p.enabled, |ui| {
                    setting(
                        ui,
                        "caddy binary",
                        Some("Name in PATH or full path"),
                        |ui| {
                            text(ui, &mut p.caddy_path, "caddy");
                            ui.hyperlink_to("Get Caddy", "https://caddyserver.com/download");
                        },
                    );
                    setting(ui, "Start with the app", None, |ui| {
                        switch(ui, &mut p.auto_start, "")
                    });
                    setting(ui, "Listen address", None, |ui| {
                        text(ui, &mut p.listen_host, "0.0.0.0");
                    });
                    setting(ui, "Port", None, |ui| {
                        ui.add(egui::DragValue::new(&mut p.port).range(1..=65535));
                    });
                    setting(ui, "Domain / hostname", None, |ui| {
                        text(ui, &mut p.domain, "ollama.example.com");
                    });
                    setting(
                        ui,
                        "CORS origin",
                        Some("Optional, for browser clients"),
                        |ui| {
                            text(ui, &mut p.cors_origin, "https://chat.example.com");
                        },
                    );
                });
            },
        );

        let enabled = self.draft.proxy.enabled;
        ui.add_enabled_ui(enabled, |ui| {
            panel(ui, ic!(KEY, "TLS certificate"), None, |ui| {
                let p = &mut self.draft.proxy;
                choice(
                    ui,
                    &mut p.tls,
                    &[
                        (TlsMode::Internal, "Self-signed"),
                        (TlsMode::Acme, "Let's Encrypt"),
                        (TlsMode::Custom, "My certificate"),
                        (TlsMode::Off, "Off (HTTP)"),
                    ],
                );
                ui.add_space(10.0);
                match p.tls {
                    TlsMode::Internal => {
                        ui.label(
                            RichText::new(
                                "Run `caddy trust` on clients, or accept the certificate.",
                            )
                            .weak(),
                        );
                    }
                    TlsMode::Acme => {
                        setting(
                            ui,
                            "ACME e-mail",
                            Some("Needs public DNS and ports 80/443"),
                            |ui| {
                                text(ui, &mut p.acme_email, "you@example.com");
                            },
                        );
                    }
                    TlsMode::Custom => {
                        setting(ui, "Certificate (PEM)", None, |ui| {
                            text(ui, &mut p.cert_path, "/path/to/fullchain.pem");
                        });
                        setting(ui, "Private key (PEM)", None, |ui| {
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
            let url = caddy::public_url(&self.draft);
            panel(ui, ic!(TEST, "Status & test"), None, |ui| {
                setting(ui, "Public URL", None, |ui| {
                    ui.label(RichText::new(&url).monospace().strong());
                    if ui.button(icon!(COPY)).on_hover_text("Copy").clicked() {
                        ui.ctx().copy_text(url.clone());
                    }
                });
                self.proxy_test_ui(ui, &url);
                ui.add_space(10.0);
                ui.label(RichText::new("Client example").strong());
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
                ic!(LOGS, "Generated Caddyfile"),
                Some(&format!("Written to {}", caddy::caddyfile_path().display())),
                |ui| {
                    let mut content = caddy::generate(&self.draft);
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
                .on_hover_text("Calls /api/version through the proxy with the first token")
                .clicked()
            {
                let conn = crate::config::Connection {
                    base_url: url.to_string(),
                    api_token: self
                        .settings
                        .proxy
                        .tokens
                        .first()
                        .map(|t| t.token.clone())
                        .unwrap_or_default(),
                    accept_invalid_certs: self.settings.proxy.tls == TlsMode::Internal,
                };
                let slot = self.proxy_test.clone();
                *slot.lock().unwrap() = Some((false, "testing…".into()));
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let res = Client::new(&conn).version();
                    *slot.lock().unwrap() = Some(match res {
                        Ok(v) => (true, format!("OK — Ollama {v} reachable with token")),
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
                RichText::new("The test uses saved settings; save first to test your changes.")
                    .weak(),
            );
        }
    }

    fn tokens_pane(&mut self, ui: &mut egui::Ui) {
        panel(
            ui,
            ic!(KEY, "Token authentication"),
            Some("Clients send `Authorization: Bearer <token>` to the HTTPS proxy."),
            |ui| {
                let p = &mut self.draft.proxy;
                setting(ui, "Require an API token", None, |ui| {
                    switch(ui, &mut p.require_token, "")
                });
                if !p.enabled {
                    kit::badge(
                        ui,
                        "The HTTPS proxy is disabled — tokens are not enforced",
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
                &format!("Tokens ({})", self.draft.proxy.tokens.len()),
                |ui| {
                    switch(ui, &mut reveal, "Reveal");
                },
                |ui| {
                    let p = &mut self.draft.proxy;
                    if p.tokens.is_empty() {
                        kit::badge(ui, "No token yet: every request will be rejected", AMBER);
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
                                        .on_hover_text("Regenerate")
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
                                .hint_text("Label, e.g. laptop or open-webui")
                                .desired_width(300.0),
                        );
                        if ui.button(ic!(ADD, "Generate token")).clicked() {
                            let label = if self.new_token_label.trim().is_empty() {
                                format!("token {}", p.tokens.len() + 1)
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
            Some("Where the Logs tab reads Ollama's output."),
            |ui| {
                choice(
                    ui,
                    &mut l.source,
                    &[
                        (LogSource::Managed, "Managed server"),
                        (LogSource::File, "Log file"),
                        (LogSource::Journal, "systemd journal"),
                    ],
                );
                ui.add_space(10.0);
                match l.source {
                    LogSource::Managed => {
                        ui.label(RichText::new("Output of the server started by this app.").weak());
                    }
                    LogSource::File => {
                        setting(ui, "Log file", Some("server.log of the tray app"), |ui| {
                            text(ui, &mut l.file_path, "server.log path");
                        });
                    }
                    LogSource::Journal => {
                        setting(ui, "systemd unit", None, |ui| {
                            text(ui, &mut l.journal_unit, "ollama");
                        });
                    }
                }
            },
        );
        panel(ui, ic!(TUNE, "Buffer"), None, |ui| {
            setting(ui, "Lines kept in memory", None, |ui| {
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
                "Interface scale",
                Some("Ctrl + / Ctrl − also zoom"),
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.ui.scale, 0.75..=2.5)
                            .step_by(0.05)
                            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
                    );
                },
            );
            setting(ui, "Presets", None, |ui| {
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
            setting(ui, "Theme", None, |ui| {
                let mut dark = self.settings.ui.dark_mode;
                choice(
                    ui,
                    &mut dark,
                    &[(true, ic!(DARK, "Dark")), (false, ic!(LIGHT, "Light"))],
                );
                if dark != self.settings.ui.dark_mode {
                    // Applied immediately, like the toggle in the top bar.
                    self.settings.ui.dark_mode = dark;
                    self.draft.ui.dark_mode = dark;
                    crate::app::apply_theme(ui.ctx(), dark);
                    let _ = self.settings.save();
                }
            });
        });

        panel(ui, ic!(SPEED, "Monitoring"), None, |ui| {
            let u = &mut self.draft.ui;
            setting(
                ui,
                "Refresh interval",
                Some("How often the API and metrics are polled"),
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut u.poll_interval_ms)
                            .range(250..=10_000)
                            .suffix(" ms"),
                    );
                },
            );
            setting(ui, "Chart history", None, |ui| {
                ui.add(
                    egui::DragValue::new(&mut u.history_seconds)
                        .range(30..=3600)
                        .suffix(" s"),
                );
            });
        });

        panel(ui, ic!(FOLDER, "Configuration files"), None, |ui| {
            setting(ui, "Folder", None, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(crate::config::config_dir().display().to_string())
                            .monospace(),
                    )
                    .wrap(),
                );
            });
            setting(
                ui,
                "Reset",
                Some("Restores every default, including API tokens"),
                |ui| {
                    if !self.confirm_reset {
                        if ui.button(ic!(UNDO, "Reset all settings")).clicked() {
                            self.confirm_reset = true;
                        }
                    } else {
                        if ui.button(RichText::new("Yes, reset").color(RED)).clicked() {
                            self.draft = Settings::default();
                            self.confirm_reset = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_reset = false;
                        }
                    }
                },
            );
        });
    }
}
