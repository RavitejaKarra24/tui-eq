use anyhow::Result;
use libmpv2::Mpv;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::browser::{list_entries, BrowserState, EntryKind};
use crate::presets::{default_autoeq_root, BgMessage, PresetState};

#[derive(Debug, Copy, Clone)]
pub enum SearchTarget {
    Browser,
    Presets,
}

#[derive(Debug)]
pub enum Mode {
    Browser,
    PresetPicker,
    Search(SearchTarget),
    Help,
}

pub struct App {
    pub mode: Mode,
    pub return_mode: Mode,
    pub browser: BrowserState,
    pub presets: PresetState,
    pub mpv: Mpv,
    pub status: String,
    pub now_playing: Option<PathBuf>,
    pub paused: bool,
    pub active_preset: Option<String>,
    pub volume: f64,
    pub rx: Receiver<BgMessage>,
    pub autoeq_root: PathBuf,
    pub search_input: String,
    pub search_backup: String,
    pub g_pending: Option<Instant>,
    pub search_j_pending: Option<Instant>,
    pub extra_preset_dirs: Vec<PathBuf>,
    pub list_height: usize,
    /// Prevents auto-advance from firing repeatedly at EOF.
    auto_advanced: bool,
}

impl App {
    pub fn new(
        path: PathBuf,
        extra_preset_dirs: Vec<PathBuf>,
        rx: Receiver<BgMessage>,
    ) -> Result<Self> {
        let mpv = init_mpv()?;
        let cwd = fs::canonicalize(&path).unwrap_or(path);
        let entries = list_entries(&cwd)?;
        let browser = BrowserState::new(cwd, entries);
        let presets = PresetState::new();
        let autoeq_root = default_autoeq_root();
        let volume = mpv.get_property::<f64>("volume").unwrap_or(100.0);
        Ok(Self {
            mode: Mode::Browser,
            return_mode: Mode::Browser,
            browser,
            presets,
            mpv,
            status: "Ready — press ? for help".to_string(),
            now_playing: None,
            paused: false,
            active_preset: None,
            volume,
            rx,
            autoeq_root,
            search_input: String::new(),
            search_backup: String::new(),
            g_pending: None,
            search_j_pending: None,
            extra_preset_dirs,
            list_height: 20,
            auto_advanced: false,
        })
    }

    pub fn handle_bg_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMessage::Status(text) => self.status = text,
                BgMessage::PresetsLoaded(presets) => {
                    let count = presets.len();
                    self.presets.set_presets(presets);
                    self.status = format!("Loaded {count} EQ presets");
                }
                BgMessage::Error(err) => self.status = err,
            }
        }
    }

    pub fn tick_playback(&mut self) {
        // Keep pause state in sync if changed externally.
        if let Ok(paused) = self.mpv.get_property::<bool>("pause") {
            self.paused = paused;
        }
        if let Ok(vol) = self.mpv.get_property::<f64>("volume") {
            self.volume = vol;
        }

        if self.now_playing.is_none() {
            return;
        }

        let eof = self
            .mpv
            .get_property::<bool>("eof-reached")
            .unwrap_or(false);
        if eof {
            if !self.auto_advanced && !self.paused {
                self.auto_advanced = true;
                self.step_file(1);
            }
        } else {
            self.auto_advanced = false;
        }
    }

    pub fn playback_progress(&self) -> (f64, f64) {
        let pos = self.mpv.get_property::<f64>("time-pos").unwrap_or(0.0);
        let dur = self.mpv.get_property::<f64>("duration").unwrap_or(0.0);
        let pos = if pos.is_finite() && pos >= 0.0 { pos } else { 0.0 };
        let dur = if dur.is_finite() && dur >= 0.0 { dur } else { 0.0 };
        (pos, dur)
    }

    pub fn enter_search(&mut self, target: SearchTarget) {
        self.search_backup = match target {
            SearchTarget::Browser => self.browser.search.clone(),
            SearchTarget::Presets => self.presets.search.clone(),
        };
        self.search_input = self.search_backup.clone();
        self.search_j_pending = None;
        self.mode = Mode::Search(target);
    }

    pub fn update_search(&mut self, target: SearchTarget) {
        match target {
            SearchTarget::Browser => self.browser.set_search(self.search_input.clone()),
            SearchTarget::Presets => self.presets.set_search(self.search_input.clone()),
        }
    }

    pub fn cancel_search(&mut self, target: SearchTarget) {
        match target {
            SearchTarget::Browser => self.browser.set_search(self.search_backup.clone()),
            SearchTarget::Presets => self.presets.set_search(self.search_backup.clone()),
        }
        self.search_j_pending = None;
    }

    pub fn clear_search_filter(&mut self) {
        match self.mode {
            Mode::Browser | Mode::Search(SearchTarget::Browser) => {
                self.browser.set_search(String::new());
                self.search_input.clear();
                self.status = "Cleared file filter".to_string();
            }
            Mode::PresetPicker | Mode::Search(SearchTarget::Presets) => {
                self.presets.set_search(String::new());
                self.search_input.clear();
                self.status = "Cleared preset filter".to_string();
            }
            Mode::Help => {}
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.now_playing.is_none() {
            self.status = "Nothing playing".to_string();
            return;
        }
        self.paused = !self.paused;
        let _ = self.mpv.set_property("pause", self.paused);
        self.status = if self.paused {
            "Paused".to_string()
        } else {
            "Resumed".to_string()
        };
    }

    pub fn stop(&mut self) {
        let _ = self.mpv.command("stop", &[]);
        self.now_playing = None;
        self.paused = false;
        self.auto_advanced = false;
        self.status = "Stopped".to_string();
    }

    pub fn play_selected(&mut self) {
        let Some(entry) = self.browser.selected_entry() else {
            return;
        };
        if matches!(entry.kind, EntryKind::Dir) {
            return;
        }
        let path_str = entry.path.to_string_lossy().to_string();
        if self
            .mpv
            .command("loadfile", &[path_str.as_str(), "replace"])
            .is_ok()
        {
            self.now_playing = Some(entry.path.clone());
            self.paused = false;
            self.auto_advanced = false;
            let _ = self.mpv.set_property("pause", false);
            // Re-apply active EQ so filters stick across track changes.
            if let Some(name) = self.active_preset.clone() {
                if let Some(preset) = self.presets.presets.iter().find(|p| p.name == name) {
                    let af = preset.eq.to_mpv_af();
                    let _ = if af.is_empty() {
                        self.mpv.set_property("af", "")
                    } else {
                        self.mpv.set_property("af", af.as_str())
                    };
                }
            }
            self.status = format!("Playing: {}", entry.name);
        } else {
            self.status = format!("Failed to play: {}", entry.name);
        }
    }

    pub fn open_selected(&mut self) -> Result<()> {
        let Some(entry) = self.browser.selected_entry() else {
            return Ok(());
        };
        match entry.kind {
            EntryKind::Dir => {
                self.browser.cwd = entry.path.clone();
                self.browser.search.clear();
                self.browser.refresh()?;
                self.status = format!("Opened {}", self.browser.cwd.display());
            }
            EntryKind::File => self.play_selected(),
        }
        Ok(())
    }

    pub fn go_parent(&mut self) -> Result<()> {
        if let Some(parent) = self.browser.cwd.parent() {
            let parent = parent.to_path_buf();
            if parent == self.browser.cwd {
                return Ok(());
            }
            self.browser.cwd = parent;
            self.browser.search.clear();
            self.browser.refresh()?;
            self.status = format!("← {}", self.browser.cwd.display());
        }
        Ok(())
    }

    pub fn refresh_browser(&mut self) -> Result<()> {
        self.browser.refresh()?;
        self.status = format!(
            "Refreshed — {} items",
            self.browser.filtered.len()
        );
        Ok(())
    }

    pub fn step_file(&mut self, direction: i32) {
        if self.browser.filtered.is_empty() {
            return;
        }
        let mut idx = self.browser.selected as i32;
        while idx >= 0 && (idx as usize) < self.browser.filtered.len() {
            idx += direction;
            if idx < 0 || (idx as usize) >= self.browser.filtered.len() {
                if direction > 0 {
                    self.status = "End of list".to_string();
                } else {
                    self.status = "Start of list".to_string();
                }
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

    pub fn apply_preset(&mut self) {
        let Some(preset) = self.presets.selected_preset() else {
            return;
        };
        let name = preset.name.clone();
        let af = preset.eq.to_mpv_af();
        let result = if af.is_empty() {
            self.mpv.set_property("af", "")
        } else {
            self.mpv.set_property("af", af.as_str())
        };
        match result {
            Ok(()) => {
                self.active_preset = Some(name.clone());
                self.status = format!("EQ: {name}");
                self.mode = Mode::Browser;
            }
            Err(err) => {
                self.status = format!("Failed to apply preset: {err}");
            }
        }
    }

    pub fn clear_eq(&mut self) {
        match self.mpv.set_property("af", "") {
            Ok(()) => {
                self.active_preset = None;
                self.status = "EQ cleared (flat)".to_string();
            }
            Err(err) => {
                self.status = format!("Failed to clear EQ: {err}");
            }
        }
    }

    pub fn adjust_volume(&mut self, delta: f64) {
        let next = (self.volume + delta).clamp(0.0, 150.0);
        if self.mpv.set_property("volume", next).is_ok() {
            self.volume = next;
            self.status = format!("Volume: {:.0}%", self.volume);
        }
    }

    pub fn seek(&mut self, seconds: f64) {
        if self.now_playing.is_none() {
            self.status = "Nothing playing".to_string();
            return;
        }
        let arg = format!("{seconds}");
        if self
            .mpv
            .command("seek", &[arg.as_str(), "relative"])
            .is_ok()
        {
            let (pos, dur) = self.playback_progress();
            self.status = format!(
                "Seek {}{:.0}s  ({}/{})",
                if seconds >= 0.0 { "+" } else { "" },
                seconds,
                crate::util::format_time(pos),
                crate::util::format_time(dur)
            );
        }
    }

    pub fn show_help(&mut self) {
        if matches!(self.mode, Mode::Help) {
            self.mode = std::mem::replace(&mut self.return_mode, Mode::Browser);
        } else if !matches!(self.mode, Mode::Search(_)) {
            self.return_mode = match self.mode {
                Mode::Browser => Mode::Browser,
                Mode::PresetPicker => Mode::PresetPicker,
                Mode::Search(_) | Mode::Help => Mode::Browser,
            };
            self.mode = Mode::Help;
        }
    }
}

fn init_mpv() -> Result<Mpv> {
    let mpv = Mpv::new()
        .map_err(|err| anyhow::anyhow!("failed to initialize libmpv: {err:?}"))?;
    let _ = mpv.set_property("vo", "null");
    let _ = mpv.set_property("vid", "no");
    let _ = mpv.set_property("keep-open", "yes");
    let _ = mpv.set_property("idle", "yes");
    let _ = mpv.set_property("volume", 100.0f64);
    let _ = mpv.set_property("audio-display", "no");
    let _ = mpv.set_property("terminal", "no");
    Ok(mpv)
}
