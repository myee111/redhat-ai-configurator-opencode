//! The interactive installer.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::apply::{self, Selection};
use crate::catalog::{self, ProviderSpec};
use crate::error::{Fail, Result, code};
use crate::goose::config::GooseMode;
use crate::goose::runner;
use crate::goose::secrets::{self, SecretsMode};
use crate::netcheck::{self, Validation};
use crate::output::{self, ProviderReport, Report};
use crate::platform::GooseInfo;
use crate::settings;

const ACCENT: Color = Color::Rgb(238, 0, 0);
const OK: Color = Color::Rgb(90, 200, 130);
const WARN: Color = Color::Rgb(230, 180, 70);
const MUTED: Color = Color::Rgb(140, 140, 150);
const TEXT: Color = Color::Rgb(215, 215, 220);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Preflight,
    Providers,
    Keys,
    Settings,
    Review,
    Applying,
    Done,
}

impl Screen {
    fn step(self) -> usize {
        match self {
            Screen::Preflight => 0,
            Screen::Providers => 1,
            Screen::Keys => 2,
            Screen::Settings => 3,
            Screen::Review | Screen::Applying | Screen::Done => 4,
        }
    }
}

#[derive(Clone, Debug)]
enum KeyStatus {
    Idle,
    Checking,
    Ok(String),
    /// The service failed (5xx); the key was not rejected, so the user may
    /// continue at their own risk.
    Warn(String),
    Err(String),
}

#[derive(Clone, Debug)]
struct ModelChoice {
    name: String,
    display: String,
    provider: String,
}

struct ProviderUi {
    spec: &'static ProviderSpec,
    selected: bool,
    key_text: String,
    keep_existing: bool,
    status: KeyStatus,
    models: Vec<ModelChoice>,
    model_index: usize,
    reachable: Option<bool>,
}

impl ProviderUi {
    fn new(spec: &'static ProviderSpec) -> Self {
        let models = spec
            .declaratives
            .iter()
            .flat_map(|declarative| {
                declarative.models.iter().map(move |model| ModelChoice {
                    name: model.name.to_string(),
                    display: model.display.to_string(),
                    provider: model.provider.to_string(),
                })
            })
            .collect();
        Self {
            spec,
            selected: true,
            key_text: String::new(),
            keep_existing: false,
            status: KeyStatus::Idle,
            models,
            model_index: 0,
            reachable: None,
        }
    }

    fn done(&self) -> bool {
        self.keep_existing || matches!(self.status, KeyStatus::Ok(_) | KeyStatus::Warn(_))
    }

    fn model(&self) -> &ModelChoice {
        &self.models[self.model_index.min(self.models.len().saturating_sub(1))]
    }

    fn merge_models(&mut self, names: Vec<String>) {
        for name in names {
            if self.models.iter().any(|model| model.name == name) {
                continue;
            }
            let provider = catalog::resolve_model(self.spec, &name)
                .map(|(declarative, _)| declarative.name.to_string())
                .unwrap_or_else(|| self.spec.default_provider.to_string());
            self.models.push(ModelChoice {
                name: name.clone(),
                display: name,
                provider,
            });
        }
    }
}

struct Review {
    providers: Vec<(String, String)>,
    mode: String,
    secrets: String,
    files: Vec<String>,
    warnings: Vec<String>,
    error: Option<String>,
}

struct DoneInfo {
    report: Report,
    specs: Vec<&'static ProviderSpec>,
    code: u8,
}

enum Msg {
    Reach(usize, bool),
    Validated(usize, Validation),
    Applied(Box<Result<Report>>, Vec<&'static ProviderSpec>),
}

struct App {
    config_dir: PathBuf,
    goose: GooseInfo,
    os: String,
    vpn: String,
    screen: Screen,
    providers: Vec<ProviderUi>,
    provider_cursor: usize,
    key_cursor: usize,
    settings_row: usize,
    mode_index: usize,
    secrets_mode: SecretsMode,
    secrets_existing: BTreeSet<String>,
    review: Option<Review>,
    done: Option<DoneInfo>,
    status_line: Option<(String, bool)>,
    applying_log: Vec<String>,
    spinner: usize,
    quit: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl App {
    fn base(config_dir: PathBuf, goose: GooseInfo) -> Self {
        let (tx, rx) = mpsc::channel();
        let os = crate::platform::os_pretty();
        let secrets_existing = secrets::existing_names(&config_dir);
        let vpn = crate::platform::vpn_state().text();
        Self {
            config_dir,
            goose,
            os,
            vpn,
            screen: Screen::Preflight,
            providers: catalog::all().iter().map(ProviderUi::new).collect(),
            provider_cursor: 0,
            key_cursor: 0,
            settings_row: 0,
            mode_index: 0,
            secrets_mode: SecretsMode::File,
            secrets_existing,
            review: None,
            done: None,
            status_line: None,
            applying_log: Vec::new(),
            spinner: 0,
            quit: false,
            tx,
            rx,
        }
    }

    fn new(config_dir: PathBuf, goose: GooseInfo) -> Self {
        let app = Self::base(config_dir, goose);
        app.probe_reachability();
        app
    }

    #[cfg(test)]
    fn for_test(config_dir: PathBuf) -> Self {
        let mut app = Self::base(
            config_dir,
            GooseInfo {
                path: PathBuf::from("/usr/bin/goose"),
                version: Some((1, 45, 0)),
                raw_version: "1.45.0".into(),
            },
        );
        app.secrets_existing = BTreeSet::new();
        app
    }

    fn probe_reachability(&self) {
        for (index, provider) in self.providers.iter().enumerate() {
            let tx = self.tx.clone();
            let url = provider.spec.validate_url();
            thread::spawn(move || {
                let ok = netcheck::reachable(&url);
                let _ = tx.send(Msg::Reach(index, ok));
            });
        }
    }

    fn selected(&self) -> Vec<usize> {
        self.providers
            .iter()
            .enumerate()
            .filter(|(_, provider)| provider.selected)
            .map(|(index, _)| index)
            .collect()
    }

    fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Reach(index, ok) => {
                if let Some(provider) = self.providers.get_mut(index) {
                    provider.reachable = Some(ok);
                }
            }
            Msg::Validated(index, result) => {
                if let Some(provider) = self.providers.get_mut(index) {
                    if result.ok {
                        provider.status = KeyStatus::Ok(result.message.clone());
                        provider.merge_models(result.models.clone());
                        self.advance_key();
                    } else if result.server_error() {
                        provider.status = KeyStatus::Warn(result.message.clone());
                        self.advance_key();
                    } else {
                        provider.status = KeyStatus::Err(result.message.clone());
                    }
                }
            }
            Msg::Applied(result, specs) => {
                let (report, code) = match *result {
                    Ok(report) => {
                        let code = if report.ok { 0 } else { code::VERIFY };
                        (report, code)
                    }
                    Err(fail) => {
                        let report = Report {
                            ok: false,
                            goose_path: self.goose.path.display().to_string(),
                            goose_version: self.goose.version_string(),
                            config: self.config_dir.join("config.yaml").display().to_string(),
                            secrets: self
                                .config_dir
                                .join("secrets.yaml")
                                .display()
                                .to_string(),
                            error: Some(fail.message.clone()),
                            error_code: Some(fail.code),
                            ..Report::default()
                        };
                        (report, fail.code)
                    }
                };
                self.done = Some(DoneInfo {
                    report,
                    specs,
                    code,
                });
                self.screen = Screen::Done;
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }

        match self.screen {
            Screen::Preflight => match key.code {
                KeyCode::Enter | KeyCode::Tab => self.screen = Screen::Providers,
                KeyCode::Esc | KeyCode::Char('q') => self.quit = true,
                _ => {}
            },
            Screen::Providers => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.provider_cursor = self.provider_cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j')
                    if self.provider_cursor + 1 < self.providers.len() =>
                {
                    self.provider_cursor += 1;
                }
                KeyCode::Char(' ') => {
                    if let Some(provider) = self.providers.get_mut(self.provider_cursor) {
                        provider.selected = !provider.selected;
                    }
                }
                KeyCode::Enter => {
                    if self.selected().is_empty() {
                        self.status_line =
                            Some(("Choose at least one provider.".into(), true));
                    } else if let Some(first) = self.selected().first() {
                        self.key_cursor = self
                            .selected()
                            .into_iter()
                            .find(|index| !self.providers[*index].done())
                            .unwrap_or(*first);
                        self.screen = Screen::Keys;
                    }
                }
                KeyCode::Esc => self.screen = Screen::Preflight,
                KeyCode::Char('q') => self.quit = true,
                _ => {}
            },
            Screen::Keys => self.handle_keys_key(key),
            Screen::Settings => self.handle_settings_key(key),
            Screen::Review => match key.code {
                KeyCode::Enter => self.start_apply(),
                KeyCode::Esc => self.screen = Screen::Settings,
                KeyCode::Char('q') => self.quit = true,
                _ => {}
            },
            Screen::Applying => {}
            Screen::Done => match key.code {
                KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
                _ => {}
            },
        }
    }

    fn handle_keys_key(&mut self, key: KeyEvent) {
        let selected = self.selected();
        match key.code {
            KeyCode::Esc => self.screen = Screen::Providers,
            KeyCode::Tab | KeyCode::Down => {
                if let Some(position) = selected.iter().position(|index| *index == self.key_cursor) {
                    let next = (position + 1) % selected.len();
                    self.key_cursor = selected[next];
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(position) = selected.iter().position(|index| *index == self.key_cursor) {
                    let next = (position + selected.len() - 1) % selected.len();
                    self.key_cursor = selected[next];
                }
            }
            KeyCode::Enter => self.validate_current(),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(provider) = self.providers.get_mut(self.key_cursor) {
                    provider.key_text.clear();
                    provider.keep_existing = false;
                    provider.status = KeyStatus::Idle;
                }
            }
            KeyCode::Backspace => {
                if let Some(provider) = self.providers.get_mut(self.key_cursor) {
                    provider.key_text.pop();
                    if matches!(provider.status, KeyStatus::Err(_) | KeyStatus::Warn(_)) {
                        provider.status = KeyStatus::Idle;
                    }
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(provider) = self.providers.get_mut(self.key_cursor) {
                    provider.key_text.push(c);
                    provider.keep_existing = false;
                    if matches!(provider.status, KeyStatus::Err(_) | KeyStatus::Warn(_)) {
                        provider.status = KeyStatus::Idle;
                    }
                }
            }
            _ => {}
        }
    }

    fn validate_current(&mut self) {
        let index = self.key_cursor;
        let Some(provider) = self.providers.get_mut(index) else {
            return;
        };
        if provider.key_text.trim().is_empty() {
            if self.secrets_existing.contains(provider.spec.secret) {
                provider.keep_existing = true;
                provider.status = KeyStatus::Ok("using the key already stored".into());
                self.advance_key();
            } else {
                provider.status = KeyStatus::Err(format!(
                    "Paste your {} API key first.",
                    provider.spec.display
                ));
            }
            return;
        }
        provider.status = KeyStatus::Checking;
        let url = provider.spec.validate_url();
        let key = provider.key_text.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = netcheck::validate(&url, &key);
            let _ = tx.send(Msg::Validated(index, result));
        });
    }

    fn advance_key(&mut self) {
        let selected = self.selected();
        let next = selected
            .iter()
            .copied()
            .find(|index| !self.providers[*index].done());
        match next {
            Some(index) => self.key_cursor = index,
            None => {
                self.screen = Screen::Settings;
                self.settings_row = 0;
            }
        }
    }

    fn settings_rows(&self) -> usize {
        self.selected().len() + 2
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        let rows = self.settings_rows();
        match key.code {
            KeyCode::Esc => self.screen = Screen::Keys,
            KeyCode::Up => self.settings_row = self.settings_row.saturating_sub(1),
            KeyCode::Down if self.settings_row + 1 < rows => {
                self.settings_row += 1;
            }
            KeyCode::Left | KeyCode::Right => {
                let delta: i32 = if key.code == KeyCode::Right { 1 } else { -1 };
                let selected = self.selected();
                let providers = selected.len();
                if self.settings_row < providers {
                    let index = selected[self.settings_row];
                    let provider = &mut self.providers[index];
                    let count = provider.models.len() as i32;
                    if count > 0 {
                        let next = (provider.model_index as i32 + delta).rem_euclid(count);
                        provider.model_index = next as usize;
                    }
                } else if self.settings_row == providers {
                    let count = GooseMode::ALL.len() as i32;
                    self.mode_index = (self.mode_index as i32 + delta).rem_euclid(count) as usize;
                } else {
                    self.secrets_mode = match self.secrets_mode {
                        SecretsMode::File => SecretsMode::Keyring,
                        SecretsMode::Keyring => SecretsMode::File,
                    };
                }
            }
            KeyCode::Enter => {
                self.review = Some(self.build_review());
                self.screen = Screen::Review;
            }
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn selection(&self) -> Result<Selection> {
        let selected = self.selected();
        let mut keys = Vec::new();
        let mut models = Vec::new();
        for index in &selected {
            let provider = &self.providers[*index];
            if !provider.key_text.trim().is_empty() {
                keys.push((provider.spec.secret.to_string(), provider.key_text.clone()));
            }
            models.push((
                provider.spec.id.to_string(),
                provider.model().name.clone(),
            ));
        }
        Ok(Selection {
            provider_ids: selected
                .iter()
                .map(|index| self.providers[*index].spec.id)
                .collect(),
            model_overrides: models,
            mode: GooseMode::ALL[self.mode_index],
            secrets_mode: self.secrets_mode,
            keys,
            keep_existing: true,
            config_dir: self.config_dir.clone(),
        })
    }

    fn build_review(&self) -> Review {
        let selected = self.selected();
        let providers = selected
            .iter()
            .map(|index| {
                let provider = &self.providers[*index];
                (
                    format!("{} ({})", provider.spec.display, provider.spec.id),
                    provider.model().name.clone(),
                )
            })
            .collect();

        match self.selection().and_then(|selection| apply::build(&selection)) {
            Ok(plan) => Review {
                providers,
                mode: format!(
                    "{} ({})",
                    GooseMode::ALL[self.mode_index].as_str(),
                    GooseMode::ALL[self.mode_index].label()
                ),
                secrets: plan.secrets_mode.label().to_string(),
                files: plan
                    .changed
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                warnings: plan.warnings,
                error: None,
            },
            Err(fail) => Review {
                providers,
                mode: GooseMode::ALL[self.mode_index].as_str().to_string(),
                secrets: self.secrets_mode.label().to_string(),
                files: Vec::new(),
                warnings: Vec::new(),
                error: Some(fail.message),
            },
        }
    }

    fn start_apply(&mut self) {
        let selection = match self.selection() {
            Ok(selection) => selection,
            Err(fail) => {
                self.status_line = Some((fail.message, true));
                return;
            }
        };

        let mut validation_warnings: Vec<String> = Vec::new();
        let mut validated: Vec<(String, Option<bool>, String)> = Vec::new();
        for index in self.selected() {
            let provider = &self.providers[index];
            let status = match &provider.status {
                KeyStatus::Ok(_) => Some(true),
                KeyStatus::Warn(message) => {
                    validation_warnings
                        .push(format!("{}: {}", provider.spec.display, message));
                    None
                }
                KeyStatus::Err(_) => Some(false),
                _ => None,
            };
            validated.push((
                provider.spec.id.to_string(),
                status,
                if provider.keep_existing {
                    "existing".to_string()
                } else {
                    "input".to_string()
                },
            ));
        }

        let goose = self.goose.clone();
        let tx = self.tx.clone();
        self.applying_log = vec![
            "Writing provider definitions".to_string(),
            "Updating config.yaml".to_string(),
            "Storing keys".to_string(),
            "Verifying with goose info --check".to_string(),
        ];
        self.screen = Screen::Applying;
        self.spinner = 0;

        thread::spawn(move || {
            let specs: Vec<&'static ProviderSpec> = selection
                .provider_ids
                .iter()
                .filter_map(|id| catalog::find(id))
                .collect();
            let result = (|| -> Result<Report> {
                let plan = apply::build(&selection)?;
                let commit = apply::commit(&plan, false)?;
                let mut report = Report {
                    ok: true,
                    goose_path: goose.path.display().to_string(),
                    goose_version: goose.version_string(),
                    providers: specs
                        .iter()
                        .map(|spec| ProviderReport {
                            id: spec.id.to_string(),
                            display: spec.display.to_string(),
                            model: plan
                                .models
                                .iter()
                                .find(|(id, _)| id == spec.id)
                                .map(|(_, model)| model.clone())
                                .unwrap_or_default(),
                            validated: validated
                                .iter()
                                .find(|(id, _, _)| id == spec.id)
                                .and_then(|(_, validated, _)| *validated),
                            key_source: validated
                                .iter()
                                .find(|(id, _, _)| id == spec.id)
                                .map(|(_, _, source)| source.clone())
                                .unwrap_or_else(|| "none".into()),
                        })
                        .collect(),
                    config: plan.config_path.display().to_string(),
                    backup: commit
                        .backups
                        .first()
                        .map(|backup| backup.path.display().to_string()),
                    secrets: plan.secrets_path.display().to_string(),
                    secrets_mode: plan.secrets_mode.label().to_string(),
                    changed: !plan.changed.is_empty(),
                    changed_files: plan
                        .changed
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect(),
                    warnings: plan
                        .warnings
                        .iter()
                        .chain(commit.warnings.iter())
                        .cloned()
                        .chain(validation_warnings)
                        .collect(),
                    ..Report::default()
                };
                report.warnings.dedup();

                let check = runner::info_check(&goose.path);
                report.verified = Some(check.ok);
                report.verify_output = Some(check.output);
                if !check.ok {
                    report.ok = false;
                    report.error =
                        Some("goose info --check failed after configuration.".to_string());
                    report.error_code = Some(code::VERIFY);
                }
                Ok(report)
            })();
            let _ = tx.send(Msg::Applied(Box::new(result), specs));
        });
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            while let Ok(msg) = self.rx.try_recv() {
                self.handle_msg(msg);
            }
            terminal
                .draw(|frame| draw(frame, self))
                .map_err(|error| Fail::new(code::INTERNAL, format!("UI error: {error}")))?;
            if self.quit {
                return Ok(());
            }
            let polled = event::poll(Duration::from_millis(80))
                .map_err(|error| Fail::new(code::INTERNAL, format!("Input error: {error}")))?;
            if polled {
                let event = event::read()
                    .map_err(|error| Fail::new(code::INTERNAL, format!("Input error: {error}")))?;
                if let Event::Key(key) = event
                    && matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                {
                    self.handle_key(key);
                }
            } else {
                self.spinner = self.spinner.wrapping_add(1);
            }
        }
    }
}

pub fn run(config_dir: PathBuf, goose: GooseInfo) -> Result<u8> {
    let mut app = App::new(config_dir, goose);
    let mut terminal = ratatui::init();
    let outcome = app.event_loop(&mut terminal);
    ratatui::restore();
    outcome?;

    match app.done.take() {
        Some(done) => {
            output::print_recap(&done.specs, &done.report);
            Ok(done.code)
        }
        None => {
            println!("Cancelled; nothing was written.");
            Ok(0)
        }
    }
}

// ---------------------------------------------------------------- rendering

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(6), Constraint::Length(2)])
            .areas(area);

    draw_header(frame, app, header);
    match app.screen {
        Screen::Preflight => draw_preflight(frame, app, body),
        Screen::Providers => draw_providers(frame, app, body),
        Screen::Keys => draw_keys(frame, app, body),
        Screen::Settings => draw_settings(frame, app, body),
        Screen::Review => draw_review(frame, app, body),
        Screen::Applying => draw_applying(frame, app, body),
        Screen::Done => draw_done(frame, app, body),
    }
    draw_footer(frame, app, footer);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let steps = ["System", "Providers", "Keys", "Model", "Review"];
    let active = app.screen.step();
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ›  ", Style::new().fg(MUTED)));
        }
        let style = if index == active {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else if index < active || app.screen == Screen::Done {
            Style::new().fg(OK)
        } else {
            Style::new().fg(MUTED)
        };
        spans.push(Span::styled(*step, style));
    }

    let title = Line::from(vec![
        Span::styled(
            " Red Hat AI Configurator",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   goose {} · {}", app.goose.version_string(), app.os),
            Style::new().fg(MUTED),
        ),
    ]);
    let separator = "─".repeat(area.width.saturating_sub(1) as usize);
    let text = Text::from(vec![
        title,
        Line::from(spans),
        Line::from(Span::styled(separator, Style::new().fg(Color::Rgb(60, 60, 68)))),
    ]);
    frame.render_widget(Paragraph::new(text), area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let hints = match app.screen {
        Screen::Preflight => "enter continue    q quit",
        Screen::Providers => "space toggle    ↑↓ move    enter continue    esc back    q quit",
        Screen::Keys => "enter validate    tab switch    ctrl+u clear    esc back    ctrl+c quit",
        Screen::Settings => "←→ change    ↑↓ move    enter review    esc back    q quit",
        Screen::Review => "enter apply    esc back    q quit",
        Screen::Applying => "working…",
        Screen::Done => "enter close",
    };
    let mut lines = Vec::new();
    if let Some((message, is_error)) = &app.status_line {
        lines.push(Line::from(Span::styled(
            message.clone(),
            Style::new().fg(if *is_error { ACCENT } else { OK }),
        )));
    } else {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        hints,
        Style::new().fg(MUTED),
    )));
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_card(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    focused: bool,
) {
    let colour = if focused { ACCENT } else { Color::Rgb(70, 70, 80) };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(colour))
        .title(Span::styled(
            format!(" {title} "),
            Style::new()
                .fg(if focused { ACCENT } else { MUTED })
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1));
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn field(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10}"), Style::new().fg(MUTED)),
        Span::styled(value.into(), Style::new().fg(TEXT)),
    ])
}

fn draw_preflight(frame: &mut Frame, app: &App, area: Rect) {
    let [system, goose_area, network] = Layout::vertical([
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Min(6),
    ])
    .areas(area);

    let config_path = app.config_dir.join("config.yaml");
    render_card(
        frame,
        system,
        "System",
        vec![
            field("os", app.os.clone()),
            field("user", std::env::var("USER").unwrap_or_else(|_| "unknown".into())),
            field("config", app.config_dir.display().to_string()),
            field(
                "existing",
                if config_path.exists() {
                    "config.yaml found (it will be backed up)"
                } else {
                    "no config.yaml yet"
                },
            ),
            field("vpn", app.vpn.clone()),
        ],
        false,
    );

    let goose_ok = app.goose.meets_min();
    render_card(
        frame,
        goose_area,
        "goose",
        vec![
            field("binary", app.goose.path.display().to_string()),
            field("version", app.goose.version_string()),
            Line::from(Span::styled(
                if goose_ok {
                    "✓ supported version"
                } else {
                    "✗ goose is too old; upgrade the goose package"
                },
                Style::new().fg(if goose_ok { OK } else { ACCENT }),
            )),
        ],
        false,
    );

    let mut lines = Vec::new();
    let selected_ids: Vec<&str> = app
        .selected()
        .iter()
        .map(|index| app.providers[*index].spec.id)
        .collect();
    if settings::endpoints().uses_placeholders_for(selected_ids.iter().copied()) {
        lines.push(Line::from(Span::styled(
            "Endpoints are placeholders. Pass --rhai-maas-base-url and --enmaas-base-url,",
            Style::new().fg(WARN),
        )));
        lines.push(Line::from(Span::styled(
            "set RHAI_MAAS_BASE_URL and ENMAAS_BASE_URL, or edit endpoints.conf.",
            Style::new().fg(WARN),
        )));
        lines.push(Line::from(""));
    }
    for spec in app
        .selected()
        .iter()
        .map(|index| app.providers[*index].spec)
    {
        if let Some(warning) = settings::endpoints().shape_warning(spec.id) {
            lines.push(Line::from(Span::styled(warning, Style::new().fg(WARN))));
        }
    }
    for provider in &app.providers {
        let (dot, style) = match provider.reachable {
            Some(true) => ("●", Style::new().fg(OK)),
            Some(false) => ("●", Style::new().fg(ACCENT)),
            None => ("○", Style::new().fg(MUTED)),
        };
        let status = match provider.reachable {
            Some(true) => "reachable",
            Some(false) => "unreachable",
            None => "checking…",
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{dot} "), style),
            Span::styled(
                format!("{:<20}", provider.spec.display),
                Style::new().fg(TEXT),
            ),
            Span::styled(status.to_string(), Style::new().fg(MUTED)),
            Span::styled(
                format!("   needs {}", provider.spec.vpn.text()),
                Style::new().fg(MUTED),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Keys are created by you in the browser; this tool never opens it for you.",
        Style::new().fg(MUTED),
    )));
    render_card(frame, network, "Connectivity", lines, true);
}

fn draw_providers(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (index, provider) in app.providers.iter().enumerate() {
        let focused = index == app.provider_cursor;
        let marker = if focused { "▸" } else { " " };
        let checkbox = if provider.selected { "[x]" } else { "[ ]" };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker} {checkbox} "),
                Style::new().fg(if focused { ACCENT } else { MUTED }),
            ),
            Span::styled(
                format!("{:<22}", provider.spec.display),
                Style::new()
                    .fg(TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({})", provider.spec.handle),
                Style::new().fg(MUTED),
            ),
            Span::styled(
                format!("   {}", provider.spec.vpn.text()),
                Style::new().fg(WARN),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("        keys: ", Style::new().fg(MUTED)),
            Span::styled(provider.spec.key_page(), Style::new().fg(MUTED)),
        ]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Both services are independent. Configure either or both.",
        Style::new().fg(MUTED),
    )));
    render_card(frame, area, "Providers", lines, true);
}

fn draw_keys(frame: &mut Frame, app: &App, area: Rect) {
    let selected = app.selected();
    let [left, right] =
        Layout::horizontal([Constraint::Length(30), Constraint::Min(30)]).areas(area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for index in &selected {
        let provider = &app.providers[*index];
        let focused = *index == app.key_cursor;
        let (badge, style) = match &provider.status {
            KeyStatus::Ok(_) if provider.keep_existing => ("existing", OK),
            KeyStatus::Ok(_) => ("validated", OK),
            KeyStatus::Checking => ("checking", WARN),
            KeyStatus::Warn(_) => ("service error", WARN),
            KeyStatus::Err(_) => ("error", ACCENT),
            KeyStatus::Idle => ("no key", MUTED),
        };
        lines.push(Line::from(vec![
            Span::styled(
                if focused { "▸ " } else { "  " },
                Style::new().fg(ACCENT),
            ),
            Span::styled(
                format!("{:<14}", provider.spec.display),
                Style::new().fg(TEXT),
            ),
            Span::styled(badge.to_string(), Style::new().fg(style)),
        ]));
        lines.push(Line::from(""));
    }
    render_card(frame, left, "Providers", lines, false);

    let Some(provider) = app.providers.get(app.key_cursor) else {
        render_card(frame, right, "Key", vec![], true);
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!("{} ({})", provider.spec.display, provider.spec.handle),
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Create or copy a key at:", Style::new().fg(MUTED)),
        ]),
        Line::from(Span::styled(
            format!("  {}", provider.spec.key_page()),
            Style::new().fg(ACCENT),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("This step needs: ", Style::new().fg(MUTED)),
            Span::styled(
                provider.spec.vpn.text().to_string(),
                Style::new().fg(WARN).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    if let Some(url) = provider.spec.eligibility_url() {
        lines.push(Line::from(vec![
            Span::styled("Access request: ", Style::new().fg(MUTED)),
            Span::styled(url.to_string(), Style::new().fg(MUTED)),
        ]));
    }
    if let Some(slack) = provider.spec.slack() {
        lines.push(Line::from(vec![
            Span::styled("Help: ", Style::new().fg(MUTED)),
            Span::styled(slack.to_string(), Style::new().fg(MUTED)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("VPN now: ", Style::new().fg(MUTED)),
        Span::styled(app.vpn.clone(), Style::new().fg(TEXT)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Guide: ", Style::new().fg(MUTED)),
        Span::styled(provider.spec.help_url(), Style::new().fg(MUTED)),
    ]));

    lines.push(Line::from(""));
    let masked = if provider.key_text.is_empty() {
        if app.secrets_existing.contains(provider.spec.secret) {
            "press enter to keep the stored key, or paste a new one".to_string()
        } else {
            "paste your key".to_string()
        }
    } else {
        "•".repeat(provider.key_text.chars().count())
    };
    let input_style = if provider.key_text.is_empty() {
        Style::new().fg(MUTED)
    } else {
        Style::new().fg(TEXT)
    };
    lines.push(Line::from(vec![
        Span::styled("Key  ", Style::new().fg(MUTED)),
        Span::styled(masked, input_style),
        if provider.key_text.is_empty() {
            Span::styled("", Style::new())
        } else {
            Span::styled("█", Style::new().fg(ACCENT))
        },
    ]));

    match &provider.status {
        KeyStatus::Idle => {}
        KeyStatus::Checking => lines.push(Line::from(Span::styled(
            "checking the key…",
            Style::new().fg(WARN),
        ))),
        KeyStatus::Ok(message) => lines.push(Line::from(Span::styled(
            format!("✓ {message}"),
            Style::new().fg(OK),
        ))),
        KeyStatus::Warn(message) => lines.push(Line::from(Span::styled(
            format!("! {message}"),
            Style::new().fg(WARN),
        ))),
        KeyStatus::Err(message) => lines.push(Line::from(Span::styled(
            format!("✗ {message}"),
            Style::new().fg(ACCENT),
        ))),
    }

    render_card(frame, right, "API key", lines, true);
}

fn draw_settings(frame: &mut Frame, app: &App, area: Rect) {
    let selected = app.selected();
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (row, index) in selected.iter().enumerate() {
        let provider = &app.providers[*index];
        let focused = row == app.settings_row;
        let model = provider.model();
        lines.push(Line::from(vec![
            Span::styled(
                if focused { "▸ " } else { "  " },
                Style::new().fg(ACCENT),
            ),
            Span::styled(
                format!("{:<18}", provider.spec.display),
                Style::new().fg(TEXT),
            ),
            Span::styled("◀ ", Style::new().fg(MUTED)),
            Span::styled(
                format!("{} ", model.display),
                Style::new().fg(if focused { ACCENT } else { TEXT }),
            ),
            Span::styled("▶", Style::new().fg(MUTED)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", Style::new()),
            Span::styled(model.name.clone(), Style::new().fg(MUTED)),
            Span::styled(
                format!("   → {}", model.provider),
                Style::new().fg(Color::Rgb(90, 90, 100)),
            ),
        ]));
        lines.push(Line::from(""));
    }

    let providers = selected.len();
    let focused = app.settings_row == providers;
    lines.push(Line::from(vec![
        Span::styled(
            if focused { "▸ " } else { "  " },
            Style::new().fg(ACCENT),
        ),
        Span::styled(format!("{:<18}", "goose mode"), Style::new().fg(TEXT)),
        Span::styled("◀ ", Style::new().fg(MUTED)),
        Span::styled(
            format!("{} ", GooseMode::ALL[app.mode_index].as_str()),
            Style::new().fg(if focused { ACCENT } else { TEXT }),
        ),
        Span::styled("▶", Style::new().fg(MUTED)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", Style::new()),
        Span::styled(
            GooseMode::ALL[app.mode_index].label().to_string(),
            Style::new().fg(MUTED),
        ),
    ]));
    lines.push(Line::from(""));

    let focused = app.settings_row == providers + 1;
    lines.push(Line::from(vec![
        Span::styled(
            if focused { "▸ " } else { "  " },
            Style::new().fg(ACCENT),
        ),
        Span::styled(format!("{:<18}", "key storage"), Style::new().fg(TEXT)),
        Span::styled("◀ ", Style::new().fg(MUTED)),
        Span::styled(
            format!("{} ", app.secrets_mode.label()),
            Style::new().fg(if focused { ACCENT } else { TEXT }),
        ),
        Span::styled("▶", Style::new().fg(MUTED)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    ", Style::new()),
        Span::styled(
            if app.secrets_mode == SecretsMode::Keyring {
                "needs secret-tool and an unlocked session keyring"
            } else {
                "plain file at ~/.config/goose/secrets.yaml, readable by you only"
            },
            Style::new().fg(MUTED),
        ),
    ]));

    render_card(frame, area, "Default model and behaviour", lines, true);
}

fn draw_review(frame: &mut Frame, app: &App, area: Rect) {
    let Some(review) = &app.review else {
        return;
    };

    if let Some(error) = &review.error {
        render_card(
            frame,
            area,
            "Cannot continue",
            vec![
                Line::from(Span::styled(error.clone(), Style::new().fg(ACCENT))),
                Line::from(""),
                Line::from(Span::styled(
                    "Press esc to go back and fix the missing piece.",
                    Style::new().fg(MUTED),
                )),
            ],
            true,
        );
        return;
    }

    let mut lines = Vec::new();
    for (provider, model) in &review.providers {
        lines.push(Line::from(vec![
            Span::styled("● ", Style::new().fg(OK)),
            Span::styled(provider.clone(), Style::new().fg(TEXT)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  model  ", Style::new().fg(MUTED)),
            Span::styled(model.clone(), Style::new().fg(TEXT)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(field("mode", review.mode.clone()));
    lines.push(field("keys", review.secrets.clone()));
    lines.push(field("config", app.config_dir.display().to_string()));
    lines.push(Line::from(""));
    if review.files.is_empty() {
        lines.push(Line::from(Span::styled(
            "Nothing to change; configuration is already up to date.",
            Style::new().fg(MUTED),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "Files to write (existing files are backed up):",
            Style::new().fg(MUTED),
        )));
        for file in &review.files {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::new()),
                Span::styled(file.clone(), Style::new().fg(TEXT)),
            ]));
        }
    }
    for warning in &review.warnings {
        lines.push(Line::from(Span::styled(
            format!("warning: {warning}"),
            Style::new().fg(WARN),
        )));
    }
    render_card(frame, area, "Review", lines, true);
}

fn draw_applying(frame: &mut Frame, app: &App, area: Rect) {
    let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} ", spinner[app.spinner % spinner.len()]),
            Style::new().fg(ACCENT),
        ),
        Span::styled("Applying configuration…", Style::new().fg(TEXT)),
    ])];
    lines.push(Line::from(""));
    for step in &app.applying_log {
        lines.push(Line::from(vec![
            Span::styled("  · ", Style::new().fg(MUTED)),
            Span::styled(step.clone(), Style::new().fg(MUTED)),
        ]));
    }
    render_card(frame, area, "Working", lines, true);
}

fn draw_done(frame: &mut Frame, app: &App, area: Rect) {
    let Some(done) = &app.done else {
        return;
    };
    let report = &done.report;
    let mut lines = Vec::new();

    if report.ok {
        lines.push(Line::from(Span::styled(
            "✓ Configuration applied",
            Style::new().fg(OK).add_modifier(Modifier::BOLD),
        )));
    } else if report.verified == Some(false) {
        lines.push(Line::from(Span::styled(
            "! Configuration written, but goose could not verify it",
            Style::new().fg(WARN).add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "✗ Configuration failed",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
    }
    if let Some(error) = &report.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::new().fg(ACCENT),
        )));
    }
    lines.push(Line::from(""));

    for provider in &report.providers {
        lines.push(Line::from(vec![
            Span::styled("● ", Style::new().fg(OK)),
            Span::styled(
                format!("{:<22}", provider.display),
                Style::new().fg(TEXT),
            ),
            Span::styled(provider.model.clone(), Style::new().fg(MUTED)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(field("config", report.config.clone()));
    if let Some(backup) = &report.backup {
        lines.push(field("backup", backup.clone()));
    }
    lines.push(field(
        "secrets",
        format!("{} ({})", report.secrets, report.secrets_mode),
    ));
    match report.verified {
        Some(true) => lines.push(field("verified", "goose info --check passed")),
        Some(false) => lines.push(field("verified", "goose info --check failed")),
        None => {}
    }
    if let Some(output) = report
        .verify_output
        .as_deref()
        .filter(|output| !output.is_empty())
        && report.verified == Some(false)
    {
        for line in output.lines().take(6) {
            lines.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::new().fg(MUTED),
            )));
        }
    }
    for warning in &report.warnings {
        lines.push(Line::from(Span::styled(
            format!("warning: {warning}"),
            Style::new().fg(WARN),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press enter to close. Start the agent with: goose",
        Style::new().fg(MUTED),
    )));

    render_card(frame, area, "Done", lines, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn preflight_lists_providers_and_connectivity() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::for_test(dir.path().to_path_buf());
        let screen = render(&app, 110, 34);
        assert!(screen.contains("Red Hat AI Configurator"));
        assert!(screen.contains("Connectivity"));
        assert!(screen.contains("Red Hat AI MaaS"));
        assert!(screen.contains("EnMaaS"));
    }

    #[test]
    fn provider_and_key_screens_show_the_key_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::for_test(dir.path().to_path_buf());
        app.screen = Screen::Providers;
        let providers = render(&app, 110, 34);
        assert!(providers.contains("[x]"));
        assert!(providers.contains("keys:"));

        app.screen = Screen::Keys;
        let keys = render(&app, 110, 34);
        assert!(keys.contains("Create or copy a key at:"));
        assert!(keys.contains("VPN off"));
    }

    #[test]
    fn settings_show_mode_and_storage() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::for_test(dir.path().to_path_buf());
        app.screen = Screen::Settings;
        let screen = render(&app, 110, 34);
        assert!(screen.contains("goose mode"));
        assert!(screen.contains("key storage"));
        assert!(screen.contains("secrets.yaml"));
    }

    #[test]
    fn review_and_done_render() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::for_test(dir.path().to_path_buf());
        app.providers[0].key_text = "secret".into();
        app.providers[0].status = KeyStatus::Ok("validated".into());
        app.providers[1].selected = false;
        app.review = Some(app.build_review());
        app.screen = Screen::Review;
        let review = render(&app, 110, 34);
        assert!(review.contains("Review"));
        assert!(review.contains("publishers/prelude-maas/models/glm-53-flash"));

        app.done = Some(DoneInfo {
            report: Report {
                ok: true,
                config: "/tmp/config.yaml".into(),
                ..Report::default()
            },
            specs: Vec::new(),
            code: 0,
        });
        app.screen = Screen::Done;
        let done = render(&app, 110, 34);
        assert!(done.contains("Configuration applied"));
    }
}
