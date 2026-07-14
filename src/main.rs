mod app;
mod browser;
mod eq;
mod keys;
mod presets;
mod ui;
mod util;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use app::App;
use keys::handle_key_event;
use presets::start_autoeq_loader;

pub const APP_NAME: &str = "eqterm";
pub const AUTOEQ_ZIP_URL: &str =
    "https://github.com/jaakkopasanen/AutoEq/archive/refs/heads/master.zip";
pub const AUTOEQ_REPO_URL: &str = "https://github.com/jaakkopasanen/AutoEq.git";

#[derive(Parser, Debug)]
#[command(
    name = APP_NAME,
    author,
    version,
    about = "Vim-first terminal music player with AutoEq headphone/IEM presets"
)]
struct Args {
    /// Path to music folder
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Additional preset roots to scan (can be repeated)
    #[arg(long = "presets-dir")]
    presets_dir: Vec<PathBuf>,
}

/// Restores the terminal even if the app panics or returns early.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        terminal.hide_cursor()?;
        Ok(Self { terminal })
    }

    fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = self.terminal.show_cursor();
        let _ = self.terminal.backend_mut().execute(LeaveAlternateScreen);
    }
}

fn main() -> Result<()> {
    // Consistent decimal formatting for filter values passed to lavfi.
    // SAFETY: called before any threads are spawned.
    unsafe {
        std::env::set_var("LC_NUMERIC", "C");
    }

    let args = Args::parse();
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(args.path, args.presets_dir, rx)?;

    start_autoeq_loader(
        tx,
        app.autoeq_root.clone(),
        app.extra_preset_dirs.clone(),
    );

    run_app(&mut app)
}

fn run_app(app: &mut App) -> Result<()> {
    let mut guard = TerminalGuard::new()?;
    let tick_rate = Duration::from_millis(33);
    let mut last_tick = Instant::now();

    loop {
        guard.terminal().draw(|f| ui::draw(f, app))?;
        app.handle_bg_messages();

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::from_millis(0));

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if handle_key_event(app, key)? {
                        break;
                    }
                }
                Event::Resize(_, _) => {
                    // Redraw on next loop iteration.
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick_playback();
            last_tick = Instant::now();
        }
    }

    Ok(())
}
