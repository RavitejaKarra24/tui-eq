use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

use crate::app::{App, Mode, SearchTarget};

pub fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => return Ok(true),
            KeyCode::Char('l') => {
                app.clear_search_filter();
                return Ok(false);
            }
            KeyCode::Char('d') => {
                half_page(app, true);
                return Ok(false);
            }
            KeyCode::Char('u') => {
                half_page(app, false);
                return Ok(false);
            }
            _ => {}
        }
    }

    match app.mode {
        Mode::Help => handle_help_keys(app, key),
        Mode::Search(target) => handle_search_input(app, key, target),
        Mode::Browser => handle_browser_keys(app, key),
        Mode::PresetPicker => handle_preset_keys(app, key),
    }
}

fn handle_help_keys(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            app.show_help();
        }
        _ => {}
    }
    Ok(false)
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
                // vim-style "jk" to leave insert/search
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
        KeyCode::Char('?') => app.show_help(),
        KeyCode::Char('j') | KeyCode::Down => {
            app.g_pending = None;
            app.browser.move_by(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.g_pending = None;
            app.browser.move_by(-1);
        }
        KeyCode::PageDown => {
            app.g_pending = None;
            app.browser.page_by(app.list_height.max(1), true);
        }
        KeyCode::PageUp => {
            app.g_pending = None;
            app.browser.page_by(app.list_height.max(1), false);
        }
        KeyCode::Char('h') | KeyCode::Left => {
            app.g_pending = None;
            app.go_parent()?;
        }
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
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
            if app.presets.loading {
                app.status = "Loading AutoEq presets…".to_string();
            }
        }
        KeyCode::Char(' ') => {
            app.g_pending = None;
            app.toggle_pause();
        }
        KeyCode::Char('s') => {
            app.g_pending = None;
            app.stop();
        }
        KeyCode::Char('n') => {
            app.g_pending = None;
            app.step_file(1);
        }
        KeyCode::Char('p') => {
            app.g_pending = None;
            app.step_file(-1);
        }
        KeyCode::Char('r') => {
            app.g_pending = None;
            app.refresh_browser()?;
        }
        KeyCode::Char('x') => {
            app.g_pending = None;
            app.clear_eq();
        }
        KeyCode::Char(',') => {
            app.g_pending = None;
            app.seek(-5.0);
        }
        KeyCode::Char('.') => {
            app.g_pending = None;
            app.seek(5.0);
        }
        KeyCode::Char('[') => {
            app.g_pending = None;
            app.seek(-30.0);
        }
        KeyCode::Char(']') => {
            app.g_pending = None;
            app.seek(30.0);
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            app.g_pending = None;
            app.adjust_volume(5.0);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            app.g_pending = None;
            app.adjust_volume(-5.0);
        }
        KeyCode::Char('g') => handle_gg(app, now, true),
        KeyCode::Char('G') => {
            app.g_pending = None;
            app.browser.select_last();
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
        KeyCode::Char('?') => app.show_help(),
        KeyCode::Char('j') | KeyCode::Down => {
            app.g_pending = None;
            app.presets.move_by(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.g_pending = None;
            app.presets.move_by(-1);
        }
        KeyCode::PageDown => {
            app.g_pending = None;
            app.presets.page_by(app.list_height.max(1), true);
        }
        KeyCode::PageUp => {
            app.g_pending = None;
            app.presets.page_by(app.list_height.max(1), false);
        }
        KeyCode::Char('/') => {
            app.g_pending = None;
            app.enter_search(SearchTarget::Presets);
        }
        KeyCode::Enter => {
            app.g_pending = None;
            app.apply_preset();
        }
        KeyCode::Char('x') => {
            app.g_pending = None;
            app.clear_eq();
        }
        KeyCode::Char('g') => handle_gg(app, now, false),
        KeyCode::Char('G') => {
            app.g_pending = None;
            app.presets.select_last();
        }
        // allow volume/seek while browsing presets
        KeyCode::Char(' ') => app.toggle_pause(),
        KeyCode::Char(',') => app.seek(-5.0),
        KeyCode::Char('.') => app.seek(5.0),
        KeyCode::Char('[') => app.seek(-30.0),
        KeyCode::Char(']') => app.seek(30.0),
        KeyCode::Char('+') | KeyCode::Char('=') => app.adjust_volume(5.0),
        KeyCode::Char('-') | KeyCode::Char('_') => app.adjust_volume(-5.0),
        _ => {}
    }
    Ok(false)
}

fn handle_gg(app: &mut App, now: Instant, browser: bool) {
    if let Some(pending_at) = app.g_pending {
        if now.duration_since(pending_at) <= Duration::from_millis(350) {
            if browser {
                app.browser.select_first();
            } else {
                app.presets.select_first();
            }
            app.g_pending = None;
        } else {
            app.g_pending = Some(now);
        }
    } else {
        app.g_pending = Some(now);
    }
}

fn half_page(app: &mut App, forward: bool) {
    let amount = (app.list_height / 2).max(1);
    match app.mode {
        Mode::Browser => app.browser.page_by(amount, forward),
        Mode::PresetPicker => app.presets.page_by(amount, forward),
        Mode::Search(SearchTarget::Browser) => app.browser.page_by(amount, forward),
        Mode::Search(SearchTarget::Presets) => app.presets.page_by(amount, forward),
        Mode::Help => {}
    }
}
