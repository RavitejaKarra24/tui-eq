use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use libmpv2::Mpv;
use once_cell::sync::Lazy;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;
use regex::Regex;
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use walkdir::WalkDir;
use std::process::{Command, Stdio};

const APP_NAME: &str = "eqterm";
const AUTOEQ_ZIP_URL: &str = "https://github.com/jaakkopasanen/AutoEq/archive/refs/heads/master.zip";
const AUTOEQ_REPO_URL: &str = "https://github.com/jaakkopasanen/AutoEq.git";
const SUPPORTED_EXTS: &[&str] = &[
    "mp3", "flac", "wav", "aac", "m4a", "ogg", "opus", "alac", "aiff", "wma", "mka",
];

static PREAMP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^Preamp:\s*([+-]?\d+(?:\.\d+)?)\s*dB").expect("preamp regex")
});
static FILTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^Filter\s+\d+:\s+(ON|OFF)\s+([A-Z]+)\s+Fc\s+([\d.]+)\s+Hz\s+Gain\s+([+-]?[\d.]+)\s+dB\s+Q\s+([\d.]+)",
    )
    .expect("filter regex")
});

#[derive(Parser, Debug)]
#[command(author, version, about = "Vim-first terminal music player with AutoEq presets")]
struct Args {
    /// Path to music folder
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Additional preset roots to scan (can be repeated)
    #[arg(long = "presets-dir")]
    presets_dir: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
enum EntryKind {
    Dir,
    File,
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    kind: EntryKind,
}

#[derive(Clone, Debug)]
enum FilterType {
    Peak,
    LowShelf,
    HighShelf,
}

#[derive(Clone, Debug)]
struct EqFilter {
    freq: f32,
    gain: f32,
    q: f32,
    kind: FilterType,
}

#[derive(Clone, Debug)]
struct EqProfile {
    preamp_db: f32,
    filters: Vec<EqFilter>,
}

impl EqProfile {
    fn to_mpv_af(&self) -> String {
        let mut chain: Vec<String> = Vec::new();
        if self.preamp_db.abs() > 0.01 {
            chain.push(format!("volume={}dB", fmt_f32(self.preamp_db)));
        }
        for filter in &self.filters {
            let base = match filter.kind {
                FilterType::Peak => "equalizer",
                FilterType::LowShelf => "lowshelf",
                FilterType::HighShelf => "highshelf",
            };
            chain.push(format!(
                "{}=f={}:t=q:w={}:g={}",
                base,
                fmt_f32(filter.freq),
                fmt_f32(filter.q),
                fmt_f32(filter.gain)
            ));
        }
        if chain.is_empty() {
            String::new()
        } else {
            format!("lavfi=[{}]", chain.join(","))
        }
    }
}

#[derive(Clone, Debug)]
struct Preset {
    name: String,
    eq: EqProfile,
}

#[derive(Debug)]
enum Mode {
    Browser,
    PresetPicker,
    Search(SearchTarget),
}

#[derive(Debug, Copy, Clone)]
enum SearchTarget {
    Browser,
    Presets,
}

#[derive(Debug)]
enum BgMessage {
    Status(String),
    PresetsLoaded(Vec<Preset>),
    Error(String),
}

#[derive(Debug)]
struct BrowserState {
    cwd: PathBuf,
    entries: Vec<Entry>,
    filtered: Vec<usize>,
    selected: usize,
    search: String,
}

impl BrowserState {
    fn new(cwd: PathBuf, entries: Vec<Entry>) -> Self {
        let mut state = Self {
            cwd,
            entries,
            filtered: Vec::new(),
            selected: 0,
            search: String::new(),
        };
        state.apply_filter();
        state
    }

    fn refresh(&mut self) -> Result<()> {
        self.entries = list_entries(&self.cwd)?;
        self.apply_filter();
        Ok(())
    }

    fn apply_filter(&mut self) {
        let needle = self.search.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if needle.is_empty() {
                    true
                } else {
                    normalized_search(&e.name).contains(&normalized_search(&needle))
                }
            })
            .map(|(idx, _)| idx)
            .collect();
        if self.filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
    }

    fn set_search(&mut self, value: String) {
        self.search = value;
        self.apply_filter();
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.filtered
            .get(self.selected)
            .and_then(|idx| self.entries.get(*idx))
    }
}

#[derive(Debug)]
struct PresetState {
    presets: Vec<Preset>,
    filtered: Vec<usize>,
    selected: usize,
    search: String,
    loading: bool,
}

impl PresetState {
    fn new() -> Self {
        Self {
            presets: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            search: String::new(),
            loading: true,
        }
    }

    fn set_presets(&mut self, presets: Vec<Preset>) {
        self.presets = presets;
        self.loading = false;
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let needle = self.search.to_lowercase();
        self.filtered = self
            .presets
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                if needle.is_empty() {
                    true
                } else {
                    normalized_search(&p.name).contains(&normalized_search(&needle))
                }
            })
            .map(|(idx, _)| idx)
            .collect();
        if self.filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
    }

    fn set_search(&mut self, value: String) {
        self.search = value;
        self.apply_filter();
    }

    fn selected_preset(&self) -> Option<&Preset> {
        self.filtered
            .get(self.selected)
            .and_then(|idx| self.presets.get(*idx))
    }
}

struct App {
    mode: Mode,
    browser: BrowserState,
    presets: PresetState,
    mpv: Mpv,
    status: String,
    now_playing: Option<PathBuf>,
    paused: bool,
    active_preset: Option<String>,
    rx: Receiver<BgMessage>,
    autoeq_root: PathBuf,
    search_input: String,
    search_backup: String,
    g_pending: Option<Instant>,
    search_j_pending: Option<Instant>,
    extra_preset_dirs: Vec<PathBuf>,
}

impl App {
    fn new(path: PathBuf, extra_preset_dirs: Vec<PathBuf>, rx: Receiver<BgMessage>) -> Result<Self> {
        let mpv = init_mpv()?;
        let cwd = fs::canonicalize(&path).unwrap_or(path);
        let entries = list_entries(&cwd)?;
        let browser = BrowserState::new(cwd, entries);
        let presets = PresetState::new();
        let autoeq_root = default_autoeq_root();
        Ok(Self {
            mode: Mode::Browser,
            browser,
            presets,
            mpv,
            status: "Ready".to_string(),
            now_playing: None,
            paused: false,
            active_preset: None,
            rx,
            autoeq_root,
            search_input: String::new(),
            search_backup: String::new(),
            g_pending: None,
            search_j_pending: None,
            extra_preset_dirs,
        })
    }

    fn handle_bg_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMessage::Status(text) => self.status = text,
                BgMessage::PresetsLoaded(presets) => {
                    let count = presets.len();
                    self.presets.set_presets(presets);
                    self.status = format!("Loaded {} presets", count);
                }
                BgMessage::Error(err) => self.status = err,
            }
        }
    }

    fn enter_search(&mut self, target: SearchTarget) {
        self.search_backup = match target {
            SearchTarget::Browser => self.browser.search.clone(),
            SearchTarget::Presets => self.presets.search.clone(),
        };
        self.search_input = self.search_backup.clone();
        self.search_j_pending = None;
        self.mode = Mode::Search(target);
    }

    fn update_search(&mut self, target: SearchTarget) {
        match target {
            SearchTarget::Browser => self.browser.set_search(self.search_input.clone()),
            SearchTarget::Presets => self.presets.set_search(self.search_input.clone()),
        }
    }

    fn cancel_search(&mut self, target: SearchTarget) {
        match target {
            SearchTarget::Browser => self.browser.set_search(self.search_backup.clone()),
            SearchTarget::Presets => self.presets.set_search(self.search_backup.clone()),
        }
        self.search_j_pending = None;
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        let _ = self.mpv.set_property("pause", self.paused);
    }

    fn play_selected(&mut self) {
        let Some(entry) = self.browser.selected_entry() else {
            return;
        };
        if matches!(entry.kind, EntryKind::Dir) {
            return;
        }
        let path_str = entry.path.to_string_lossy().to_string();
        if self.mpv.command("loadfile", &[path_str.as_str(), "replace"]).is_ok() {
            self.now_playing = Some(entry.path.clone());
            self.paused = false;
            let _ = self.mpv.set_property("pause", false);
            self.status = format!("Playing: {}", entry.name);
        }
    }

    fn open_selected(&mut self) -> Result<()> {
        let Some(entry) = self.browser.selected_entry() else {
            return Ok(());
        };
        match entry.kind {
            EntryKind::Dir => {
                self.browser.cwd = entry.path.clone();
                self.browser.search.clear();
                self.browser.refresh()?;
            }
            EntryKind::File => self.play_selected(),
        }
        Ok(())
    }

    fn go_parent(&mut self) -> Result<()> {
        if let Some(parent) = self.browser.cwd.parent() {
            self.browser.cwd = parent.to_path_buf();
            self.browser.search.clear();
            self.browser.refresh()?;
        }
        Ok(())
    }

    fn step_file(&mut self, direction: i32) {
        if self.browser.filtered.is_empty() {
            return;
        }
        let mut idx = self.browser.selected as i32;
        while idx >= 0 && (idx as usize) < self.browser.filtered.len() {
            idx += direction;
            if idx < 0 || (idx as usize) >= self.browser.filtered.len() {
                break;
            }
            let entry = &self.browser.entries[self.browser.filtered[idx as usize]];
            if matches!(entry.kind, EntryKind::File) {
                self.browser.selected = idx as usize;
                self.play_selected();
                break;
            }
        }
    }

    fn apply_preset(&mut self) {
        let Some(preset) = self.presets.selected_preset() else {
            return;
        };
        let af = preset.eq.to_mpv_af();
        let result = if af.is_empty() {
            self.mpv.set_property("af", "")
        } else {
            self.mpv.set_property("af", af.as_str())
        };
        match result {
            Ok(()) => {
                self.active_preset = Some(preset.name.clone());
                self.status = format!("Applied preset: {}", preset.name);
            }
            Err(err) => {
                self.status = format!("Failed to apply preset: {}", err);
            }
        }
    }
}

fn main() -> Result<()> {
    // Ensure consistent decimal formatting for filter values.
    unsafe {
        std::env::set_var("LC_NUMERIC", "C");
    }
    let args = Args::parse();

    let (tx, rx) = mpsc::channel();
    let mut app = App::new(args.path, args.presets_dir, rx)?;

    start_autoeq_loader(
        tx.clone(),
        app.autoeq_root.clone(),
        app.extra_preset_dirs.clone(),
    );

    run_app(&mut app)?;
    Ok(())
}

fn run_app(app: &mut App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let tick_rate = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, app))?;
        app.handle_bg_messages();

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::from_millis(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let should_quit = handle_key_event(app, key)?;
                    if should_quit {
                        break;
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('c') = key.code {
            return Ok(true);
        }
    }

    match app.mode {
        Mode::Search(target) => handle_search_input(app, key, target),
        Mode::Browser => handle_browser_keys(app, key),
        Mode::PresetPicker => handle_preset_keys(app, key),
    }
}

fn handle_search_input(app: &mut App, key: KeyEvent, target: SearchTarget) -> Result<bool> {
    let now = Instant::now();
    match key.code {
        KeyCode::Esc => {
            app.cancel_search(target);
            app.mode = match target {
                SearchTarget::Browser => Mode::Browser,
                SearchTarget::Presets => Mode::PresetPicker,
            };
        }
        KeyCode::Enter => {
            app.update_search(target);
            app.mode = match target {
                SearchTarget::Browser => Mode::Browser,
                SearchTarget::Presets => Mode::PresetPicker,
            };
        }
        KeyCode::Backspace => {
            app.search_input.pop();
            app.search_j_pending = None;
            app.update_search(target);
        }
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                if ch == 'k' {
                    if let Some(pending_at) = app.search_j_pending {
                        if now.duration_since(pending_at) <= Duration::from_millis(350) {
                            if app.search_input.ends_with('j') {
                                app.search_input.pop();
                            }
                            app.cancel_search(target);
                            app.mode = match target {
                                SearchTarget::Browser => Mode::Browser,
                                SearchTarget::Presets => Mode::PresetPicker,
                            };
                            return Ok(false);
                        }
                    }
                }

                if ch == 'j' {
                    app.search_j_pending = Some(now);
                } else {
                    app.search_j_pending = None;
                }

                app.search_input.push(ch);
                app.update_search(target);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_browser_keys(app: &mut App, key: KeyEvent) -> Result<bool> {
    let now = Instant::now();
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('j') | KeyCode::Down => {
            app.g_pending = None;
            if !app.browser.filtered.is_empty() {
                app.browser.selected =
                    (app.browser.selected + 1).min(app.browser.filtered.len() - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.g_pending = None;
            if !app.browser.filtered.is_empty() {
                if app.browser.selected > 0 {
                    app.browser.selected -= 1;
                }
            }
        }
        KeyCode::Char('h') | KeyCode::Left => {
            app.g_pending = None;
            app.go_parent()?;
        }
        KeyCode::Char('l') | KeyCode::Enter => {
            app.g_pending = None;
            app.open_selected()?;
        }
        KeyCode::Char('/') => {
            app.g_pending = None;
            app.enter_search(SearchTarget::Browser);
        }
        KeyCode::Char('e') => {
            app.g_pending = None;
            app.mode = Mode::PresetPicker;
        }
        KeyCode::Char(' ') => {
            app.g_pending = None;
            app.toggle_pause();
        }
        KeyCode::Char('n') => app.step_file(1),
        KeyCode::Char('p') => app.step_file(-1),
        KeyCode::Char('g') => {
            if let Some(pending_at) = app.g_pending {
                if now.duration_since(pending_at) <= Duration::from_millis(350) {
                    app.browser.selected = 0;
                    app.g_pending = None;
                } else {
                    app.g_pending = Some(now);
                }
            } else {
                app.g_pending = Some(now);
            }
        }
        KeyCode::Char('G') => {
            app.g_pending = None;
            if !app.browser.filtered.is_empty() {
                app.browser.selected = app.browser.filtered.len() - 1;
            }
        }
        _ => {}
    }
    Ok(false)
}

fn handle_preset_keys(app: &mut App, key: KeyEvent) -> Result<bool> {
    let now = Instant::now();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.g_pending = None;
            app.mode = Mode::Browser;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.g_pending = None;
            if !app.presets.filtered.is_empty() {
                app.presets.selected =
                    (app.presets.selected + 1).min(app.presets.filtered.len() - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.g_pending = None;
            if !app.presets.filtered.is_empty() && app.presets.selected > 0 {
                app.presets.selected -= 1;
            }
        }
        KeyCode::Char('/') => {
            app.g_pending = None;
            app.enter_search(SearchTarget::Presets);
        }
        KeyCode::Enter => {
            app.g_pending = None;
            app.apply_preset();
        }
        KeyCode::Char('g') => {
            if let Some(pending_at) = app.g_pending {
                if now.duration_since(pending_at) <= Duration::from_millis(350) {
                    app.presets.selected = 0;
                    app.g_pending = None;
                } else {
                    app.g_pending = Some(now);
                }
            } else {
                app.g_pending = Some(now);
            }
        }
        KeyCode::Char('G') => {
            app.g_pending = None;
            if !app.presets.filtered.is_empty() {
                app.presets.selected = app.presets.filtered.len() - 1;
            }
        }
        _ => {}
    }
    Ok(false)
}

fn ui(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(5),
        ])
        .split(f.size());

    draw_header(f, app, chunks[0]);
    draw_list(f, app, chunks[1]);
    draw_footer(f, app, chunks[2]);
}

fn draw_header(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    lines.push(Line::from(format!(
        "Path: {}",
        app.browser.cwd.to_string_lossy()
    )));

    let now = if let Some(path) = &app.now_playing {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        if app.paused {
            format!("Now: {} (paused)", name)
        } else {
            format!("Now: {}", name)
        }
    } else {
        "Now: (none)".to_string()
    };
    lines.push(Line::from(now));

    let preset = app
        .active_preset
        .as_deref()
        .unwrap_or("(none)")
        .to_string();
    lines.push(Line::from(format!("Preset: {}", preset)));

    let block = Block::default().borders(Borders::ALL).title(APP_NAME);
    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

fn draw_list(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let (items, title) = match app.mode {
        Mode::PresetPicker | Mode::Search(SearchTarget::Presets) => {
            let items: Vec<ListItem> = app
                .presets
                .filtered
                .iter()
                .filter_map(|idx| app.presets.presets.get(*idx))
                .map(|preset| {
                    let mut name = preset.name.clone();
                    if app
                        .active_preset
                        .as_ref()
                        .map(|active| active == &preset.name)
                        .unwrap_or(false)
                    {
                        name = format!("* {}", name);
                    }
                    ListItem::new(Line::from(name))
                })
                .collect();
            (items, "EQ Presets")
        }
        _ => {
            let items: Vec<ListItem> = app
                .browser
                .filtered
                .iter()
                .filter_map(|idx| app.browser.entries.get(*idx))
                .map(|entry| {
                    let mut name = match entry.kind {
                        EntryKind::Dir => format!("{}/", entry.name),
                        EntryKind::File => entry.name.clone(),
                    };
                    if app
                        .now_playing
                        .as_ref()
                        .map(|p| p == &entry.path)
                        .unwrap_or(false)
                    {
                        name = format!("> {}", name);
                    }
                    ListItem::new(Line::from(name))
                })
                .collect();
            (items, "Files")
        }
    };

    let has_items = !items.is_empty();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ratatui::widgets::ListState::default();
    let selected = match app.mode {
        Mode::PresetPicker | Mode::Search(SearchTarget::Presets) => app.presets.selected,
        _ => app.browser.selected,
    };
    if has_items {
        state.select(Some(selected));
    }

    f.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(app.status.clone()));

    match app.mode {
        Mode::Search(target) => {
            let label = match target {
                SearchTarget::Browser => "files",
                SearchTarget::Presets => "presets",
            };
            lines.push(Line::from(format!("Search {}: /{}", label, app.search_input)));
            let help = match app.mode {
                Mode::PresetPicker | Mode::Search(SearchTarget::Presets) => {
                    "j/k up/down  Enter apply  / search  Esc back"
                }
                _ => "j/k up/down  h back  l/Enter open/play  e presets  / search  space pause  n/p next/prev  q quit",
            };
            lines.push(Line::from(help));
        }
        Mode::PresetPicker => {
            lines.push(Line::from("j/k up/down  Enter apply  / search  Esc back"));
            if app.presets.loading {
                lines.push(Line::from("Loading AutoEq presets..."));
            } else if !app.presets.search.is_empty() {
                lines.push(Line::from(format!("Filter presets: /{}", app.presets.search)));
            }
        }
        Mode::Browser => {
            lines.push(Line::from(
                "j/k up/down  h back  l/Enter open/play  e presets  / search  space pause  n/p next/prev  q quit",
            ));
            if !app.browser.search.is_empty() {
                lines.push(Line::from(format!("Filter files: /{}", app.browser.search)));
            }
        }
    }

    let block = Block::default().borders(Borders::ALL).title("Status");
    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

fn init_mpv() -> Result<Mpv> {
    let mpv = Mpv::new()
        .map_err(|err| anyhow::anyhow!("failed to initialize libmpv: {:?}", err))?;
    let _ = mpv.set_property("vo", "null");
    let _ = mpv.set_property("vid", "no");
    let _ = mpv.set_property("keep-open", true);
    let _ = mpv.set_property("idle", true);
    Ok(mpv)
}

fn list_entries(dir: &Path) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read dir {}", dir.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() {
            entries.push(Entry {
                name,
                path,
                kind: EntryKind::Dir,
            });
        } else if file_type.is_file() && is_audio_file(&path) {
            entries.push(Entry {
                name,
                path,
                kind: EntryKind::File,
            });
        }
    }

    entries.sort_by(|a, b| match (&a.kind, &b.kind) {
        (EntryKind::Dir, EntryKind::File) => Ordering::Less,
        (EntryKind::File, EntryKind::Dir) => Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_EXTS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn default_autoeq_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join("autoeq")
}

fn autoeq_results_dir(root: &Path) -> PathBuf {
    root.join("results")
}

fn autoeq_custom_dir(root: &Path) -> PathBuf {
    root.join("presets")
}

fn start_autoeq_loader(
    tx: Sender<BgMessage>,
    autoeq_root: PathBuf,
    extra_preset_dirs: Vec<PathBuf>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<Preset>> {
            let results_dir = autoeq_results_dir(&autoeq_root);
            if !has_parametric_eq(&results_dir) {
                tx.send(BgMessage::Status("Downloading AutoEq results...".to_string()))
                    .ok();
                download_autoeq(&autoeq_root, &results_dir, &tx)?;
            }

            tx.send(BgMessage::Status("Loading presets...".to_string()))
                .ok();
            let presets = load_presets_multi(&autoeq_root, &results_dir, &extra_preset_dirs)?;
            Ok(presets)
        })();

        match result {
            Ok(presets) => {
                let _ = tx.send(BgMessage::PresetsLoaded(presets));
            }
            Err(err) => {
                let _ = tx.send(BgMessage::Error(format!("AutoEq error: {}", err)));
            }
        }
    });
}

fn download_autoeq(
    autoeq_root: &Path,
    results_dir: &Path,
    tx: &Sender<BgMessage>,
) -> Result<()> {
    fs::create_dir_all(autoeq_root)
        .with_context(|| format!("create autoeq root {}", autoeq_root.display()))?;

    if results_dir.exists() {
        let _ = fs::remove_dir_all(results_dir);
    }

    if try_git_sparse_checkout(autoeq_root, results_dir, tx).is_ok() {
        return Ok(());
    }

    tx.send(BgMessage::Status(
        "AutoEq git download failed, falling back to zip...".to_string(),
    ))
    .ok();

    download_autoeq_zip(autoeq_root, results_dir, tx)
}

fn load_presets_multi(
    autoeq_root: &Path,
    results_dir: &Path,
    extra_preset_dirs: &[PathBuf],
) -> Result<Vec<Preset>> {
    let mut presets = Vec::new();

    presets.extend(load_presets_with_label(
        results_dir,
        "AutoEq Default".to_string(),
    )?);

    let custom_dir = autoeq_custom_dir(autoeq_root);
    if custom_dir.exists() {
        presets.extend(load_presets_with_label(
            &custom_dir,
            "Custom".to_string(),
        )?);
    }

    for dir in extra_preset_dirs {
        if dir.exists() {
            let label = dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Custom")
                .to_string();
            presets.extend(load_presets_with_label(dir, label)?);
        }
    }

    presets.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(presets)
}

fn load_presets_with_label(root: &Path, label: String) -> Result<Vec<Preset>> {
    let mut presets = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with("ParametricEQ.txt") {
            continue;
        }
        if let Ok(mut preset) = parse_preset(path, root) {
            preset.name = format!("{} / {}", label, preset.name);
            presets.push(preset);
        }
    }

    Ok(presets)
}

fn try_git_sparse_checkout(
    autoeq_root: &Path,
    results_dir: &Path,
    tx: &Sender<BgMessage>,
) -> Result<()> {
    let temp_dir = autoeq_root.join("autoeq_git");
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }

    tx.send(BgMessage::Status(
        "AutoEq: cloning results (git sparse checkout)...".to_string(),
    ))
    .ok();

    let clone_status = Command::new("git")
        .args([
            "clone",
            "--depth=1",
            "--filter=blob:none",
            "--sparse",
            AUTOEQ_REPO_URL,
            temp_dir.to_string_lossy().as_ref(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let status = match clone_status {
        Ok(status) if status.success() => status,
        _ => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(anyhow::anyhow!("git clone failed"));
        }
    };

    let _ = status;

    tx.send(BgMessage::Status(
        "AutoEq: checking out results...".to_string(),
    ))
    .ok();

    let sparse_status = Command::new("git")
        .args([
            "-C",
            temp_dir.to_string_lossy().as_ref(),
            "sparse-checkout",
            "set",
            "results",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if sparse_status.map(|s| s.success()).unwrap_or(false) == false {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(anyhow::anyhow!("git sparse-checkout failed"));
    }

    tx.send(BgMessage::Status(
        "AutoEq: copying results...".to_string(),
    ))
    .ok();

    let temp_results = temp_dir.join("results");
    if !temp_results.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(anyhow::anyhow!("git results folder missing"));
    }

    fs::create_dir_all(results_dir).context("create results dir")?;
    for entry in WalkDir::new(&temp_results).follow_links(false) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(&temp_results)?;
        let dest = results_dir.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest)?;
        } else if entry.file_type().is_file() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with("ParametricEQ.txt") {
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::copy(entry.path(), &dest)?;
                }
            }
        }
    }

    let _ = fs::remove_dir_all(&temp_dir);
    Ok(())
}

fn download_autoeq_zip(
    autoeq_root: &Path,
    results_dir: &Path,
    tx: &Sender<BgMessage>,
) -> Result<()> {
    let zip_path = autoeq_root.join("autoeq.zip");
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("{}/{}", APP_NAME, env!("CARGO_PKG_VERSION")))
        .build()
        .context("build http client")?;

    let mut response = client
        .get(AUTOEQ_ZIP_URL)
        .send()
        .context("download AutoEq zip")?;

    let total = response.content_length();
    let mut zip_file = File::create(&zip_path).context("write AutoEq zip")?;
    let mut downloaded: u64 = 0;
    let mut last_report: u64 = 0;
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let bytes = response.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        zip_file.write_all(&buffer[..bytes])?;
        downloaded += bytes as u64;
        if downloaded.saturating_sub(last_report) >= 20 * 1024 * 1024 {
            let msg = if let Some(total) = total {
                format!(
                    "Downloading AutoEq results... {} / {} MB",
                    downloaded / (1024 * 1024),
                    total / (1024 * 1024)
                )
            } else {
                format!(
                    "Downloading AutoEq results... {} MB",
                    downloaded / (1024 * 1024)
                )
            };
            tx.send(BgMessage::Status(msg)).ok();
            last_report = downloaded;
        }
    }

    let zip_file = File::open(&zip_path).context("open AutoEq zip")?;
    let mut archive = zip::ZipArchive::new(zip_file).context("read AutoEq zip")?;

    fs::create_dir_all(results_dir).context("create results dir")?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name();

        let rel = if let Some(stripped) = name.strip_prefix("AutoEq-master/results/") {
            stripped
        } else if let Some(stripped) = name.strip_prefix("AutoEq-main/results/") {
            stripped
        } else {
            continue;
        };

        if rel.is_empty() {
            continue;
        }

        if file.is_dir() {
            fs::create_dir_all(results_dir.join(rel))?;
            continue;
        }

        if !rel.ends_with("ParametricEQ.txt") {
            continue;
        }

        let out_path = results_dir.join(rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out_path)?;
        io::copy(&mut file, &mut outfile)?;
    }

    let _ = fs::remove_file(zip_path);
    Ok(())
}

fn has_parametric_eq(results_dir: &Path) -> bool {
    if !results_dir.exists() {
        return false;
    }
    for entry in WalkDir::new(results_dir).follow_links(false) {
        if let Ok(entry) = entry {
            if entry.file_type().is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with("ParametricEQ.txt") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn parse_preset(path: &Path, results_dir: &Path) -> Result<Preset> {
    let content = fs::read_to_string(path).context("read preset file")?;
    let eq = parse_eq_profile(&content)?;
    let name = preset_name(path, results_dir);
    Ok(Preset { name, eq })
}

fn preset_name(path: &Path, results_dir: &Path) -> String {
    let rel = path
        .strip_prefix(results_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let mut name = rel
        .trim_end_matches("ParametricEQ.txt")
        .trim_end_matches(".txt")
        .trim_end_matches('/')
        .trim_end_matches('\\')
        .to_string();
    name = name.replace(['/', '\\'], " / ");
    if name.is_empty() {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        name
    }
}

fn parse_eq_profile(input: &str) -> Result<EqProfile> {
    let mut preamp_db = 0.0;
    let mut filters = Vec::new();

    for line in input.lines() {
        if let Some(caps) = PREAMP_RE.captures(line) {
            preamp_db = caps[1].parse::<f32>().unwrap_or(0.0);
            continue;
        }
        let Some(caps) = FILTER_RE.captures(line) else {
            continue;
        };
        if &caps[1] != "ON" {
            continue;
        }
        let kind_raw = &caps[2];
        let kind = if kind_raw.starts_with("PK") {
            FilterType::Peak
        } else if kind_raw.starts_with("LS") {
            FilterType::LowShelf
        } else if kind_raw.starts_with("HS") {
            FilterType::HighShelf
        } else {
            continue;
        };
        let freq = caps[3].parse::<f32>().unwrap_or(0.0);
        let gain = caps[4].parse::<f32>().unwrap_or(0.0);
        let q = caps[5].parse::<f32>().unwrap_or(0.0);
        filters.push(EqFilter {
            freq,
            gain,
            q,
            kind,
        });
    }

    Ok(EqProfile { preamp_db, filters })
}

fn fmt_f32(value: f32) -> String {
    if value.abs() < 0.0005 {
        "0".to_string()
    } else {
        format!("{:.3}", value)
    }
}

fn normalized_search(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '_' | '-' | '/' | '\\'))
        .collect()
}
