use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Mode, SearchTarget};
use crate::browser::EntryKind;
use crate::util::{format_time, format_volume};
use crate::APP_NAME;

// ── Zack Snyder black grade (night / Dolby Vision safe) ──────────────
// Crushed blacks, low-luminance steel, no pure white, no crimson.
// Soft metal hierarchy so HDR panels don't bloom on text.

/// Absolute void — near-black canvas
const VOID: Color = Color::Rgb(4, 4, 6);
/// Panel fill slightly lifted from void
const PANEL: Color = Color::Rgb(10, 10, 12);
/// Steel border rails
const STEEL: Color = Color::Rgb(40, 42, 48);
/// Dim labels / chrome
const ASH: Color = Color::Rgb(72, 74, 82);
/// Secondary body text — mid graphite
const SMOKE: Color = Color::Rgb(112, 114, 122);
/// Primary text — soft pewter (not white)
const BONE: Color = Color::Rgb(148, 150, 156);
/// Accent / selection — dim steel, still readable
const SILVER: Color = Color::Rgb(132, 136, 146);
/// Playing / active — cool gunmetal (no red)
const ACTIVE: Color = Color::Rgb(118, 128, 142);
/// Playing marker — slightly cooler lift
const ACTIVE_DIM: Color = Color::Rgb(96, 104, 118);
/// EQ / epic metal — muted bronze, restrained
const BRONZE: Color = Color::Rgb(118, 100, 72);
/// Paused / caution — desaturated ochre
const AMBER: Color = Color::Rgb(120, 100, 64);
/// Folders — cold steel blue-gray
const SLATE: Color = Color::Rgb(88, 96, 110);
/// Gauge empty track
const TRACK: Color = Color::Rgb(18, 18, 22);
/// Errors — deep rust, not neon red
const RUST: Color = Color::Rgb(110, 56, 48);

fn base() -> Style {
    Style::default().bg(VOID).fg(BONE)
}

fn panel_block(title: impl Into<String>, title_fg: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(STEEL).bg(VOID))
        .style(Style::default().bg(PANEL).fg(BONE))
        .title(Span::styled(
            title.into(),
            Style::default()
                .fg(title_fg)
                .bg(VOID)
                .add_modifier(Modifier::BOLD),
        ))
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let size = f.size();

    // Crush the whole canvas to pure black before drawing panels.
    f.render_widget(
        Block::default().style(Style::default().bg(VOID).fg(BONE)),
        size,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // header
            Constraint::Length(3), // progress
            Constraint::Min(5),    // list
            Constraint::Length(4), // footer
        ])
        .split(size);

    app.list_height = chunks[2].height.saturating_sub(2) as usize;

    draw_header(f, app, chunks[0]);
    draw_progress(f, app, chunks[1]);
    draw_list(f, app, chunks[2]);
    draw_footer(f, app, chunks[3]);

    if matches!(app.mode, Mode::Help) {
        draw_help_overlay(f, size);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let path = truncate_middle(
        &app.browser.cwd.to_string_lossy(),
        (area.width as usize).saturating_sub(10),
    );

    let (track_name, state_label) = if let Some(path) = &app.now_playing {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let label = if app.paused { "PAUSED" } else { "PLAYING" };
        (name, label)
    } else {
        ("—".to_string(), "IDLE")
    };

    let preset = app
        .active_preset
        .as_deref()
        .map(|p| truncate_middle(p, 48))
        .unwrap_or_else(|| "flat".to_string());

    let state_fg = if app.paused {
        AMBER
    } else if app.now_playing.is_some() {
        ACTIVE
    } else {
        ASH
    };

    let title = format!(" {APP_NAME} ");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(STEEL).bg(VOID))
        .style(Style::default().bg(PANEL).fg(BONE))
        .title(Span::styled(
            title,
            Style::default()
                .fg(VOID)
                .bg(SILVER)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Left);

    let lines = vec![
        Line::from(vec![
            Span::styled(" PATH ", Style::default().fg(ASH).bg(PANEL)),
            Span::styled(path, Style::default().fg(SMOKE).bg(PANEL)),
        ]),
        Line::from(vec![
            Span::styled(" NOW  ", Style::default().fg(ASH).bg(PANEL)),
            Span::styled(
                format!("{state_label}  "),
                Style::default()
                    .fg(state_fg)
                    .bg(PANEL)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_middle(&track_name, 50),
                Style::default().fg(BONE).bg(PANEL),
            ),
            Span::styled("   ", Style::default().bg(PANEL)),
            Span::styled("VOL ", Style::default().fg(ASH).bg(PANEL)),
            Span::styled(
                format_volume(app.volume),
                Style::default().fg(SILVER).bg(PANEL),
            ),
            Span::styled("   ", Style::default().bg(PANEL)),
            Span::styled("EQ ", Style::default().fg(ASH).bg(PANEL)),
            Span::styled(
                preset,
                Style::default()
                    .fg(if app.active_preset.is_some() {
                        BRONZE
                    } else {
                        ASH
                    })
                    .bg(PANEL)
                    .add_modifier(if app.active_preset.is_some() {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).style(base()).block(block), area);
}

fn draw_progress(f: &mut Frame, app: &App, area: Rect) {
    let (pos, dur) = app.playback_progress();
    let ratio = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let label = if app.now_playing.is_some() {
        format!("{} / {}", format_time(pos), format_time(dur))
    } else {
        "0:00 / 0:00".to_string()
    };

    let bar_fg = if app.paused {
        AMBER
    } else if app.now_playing.is_some() {
        ACTIVE
    } else {
        STEEL
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(STEEL).bg(VOID))
                .style(Style::default().bg(PANEL))
                .title(Span::styled(
                    " PROGRESS ",
                    Style::default().fg(ASH).bg(VOID).add_modifier(Modifier::BOLD),
                )),
        )
        .gauge_style(Style::default().fg(bar_fg).bg(TRACK))
        .ratio(ratio)
        .label(Span::styled(
            label,
            Style::default()
                .fg(BONE)
                .add_modifier(Modifier::BOLD),
        ));

    f.render_widget(gauge, area);
}

fn showing_presets(app: &App) -> bool {
    match app.mode {
        Mode::PresetPicker | Mode::Search(SearchTarget::Presets) => true,
        Mode::Help => matches!(app.return_mode, Mode::PresetPicker),
        _ => false,
    }
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    let (items, title, selected) = if showing_presets(app) {
        let items: Vec<ListItem> = app
            .presets
            .filtered
            .iter()
            .filter_map(|idx| app.presets.presets.get(*idx))
            .map(|preset| {
                let active = app
                    .active_preset
                    .as_ref()
                    .map(|a| a == &preset.name)
                    .unwrap_or(false);
                let marker = if active { "● " } else { "  " };
                let style = if active {
                    Style::default()
                        .fg(BRONZE)
                        .bg(PANEL)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(SMOKE).bg(PANEL)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, Style::default().fg(BRONZE).bg(PANEL)),
                    Span::styled(preset.name.clone(), style),
                ]))
                .style(Style::default().bg(PANEL))
            })
            .collect();
        let shown = app.presets.filtered.len();
        let total = app.presets.presets.len();
        let title = if app.presets.loading {
            " EQ PRESETS  ·  LOADING… ".to_string()
        } else {
            format!(" EQ PRESETS  ·  {shown}/{total} ")
        };
        (items, title, app.presets.selected)
    } else {
        let items: Vec<ListItem> = app
            .browser
            .filtered
            .iter()
            .filter_map(|idx| app.browser.entries.get(*idx))
            .map(|entry| {
                let playing = app
                    .now_playing
                    .as_ref()
                    .map(|p| p == &entry.path)
                    .unwrap_or(false);
                match entry.kind {
                    EntryKind::Dir => {
                        let label = format!("▸ {}/", entry.name);
                        ListItem::new(Line::from(Span::styled(
                            label,
                            Style::default()
                                .fg(SLATE)
                                .bg(PANEL)
                                .add_modifier(Modifier::BOLD),
                        )))
                        .style(Style::default().bg(PANEL))
                    }
                    EntryKind::File => {
                        let marker = if playing { "▶ " } else { "  " };
                        let style = if playing {
                            Style::default()
                                .fg(ACTIVE)
                                .bg(PANEL)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(SMOKE).bg(PANEL)
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(marker, Style::default().fg(ACTIVE_DIM).bg(PANEL)),
                            Span::styled(entry.name.clone(), style),
                        ]))
                        .style(Style::default().bg(PANEL))
                    }
                }
            })
            .collect();
        let shown = app.browser.filtered.len();
        let total = app.browser.entries.len();
        let title = format!(" LIBRARY  ·  {shown}/{total} ");
        (items, title, app.browser.selected)
    };

    if items.is_empty() {
        let empty_msg = if showing_presets(app) {
            if app.presets.loading {
                "Loading AutoEq presets in the background…"
            } else if !app.presets.search.is_empty() {
                "No presets match this filter. Press Ctrl-l to clear."
            } else {
                "No presets found. Check AutoEq download or --presets-dir."
            }
        } else if !app.browser.search.is_empty() {
            "No files match this filter. Press Ctrl-l to clear."
        } else {
            "No audio files here. Press h for parent, or open another folder."
        };
        let block = panel_block(title, SILVER);
        f.render_widget(
            Paragraph::new(Span::styled(
                empty_msg,
                Style::default().fg(ASH).bg(PANEL),
            ))
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().bg(PANEL)),
            area,
        );
        return;
    }

    let list = List::new(items)
        .block(panel_block(title, SILVER))
        .highlight_style(
            Style::default()
                .fg(VOID)
                .bg(SILVER)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    let status_style = if app.status.to_lowercase().contains("fail")
        || app.status.to_lowercase().contains("error")
    {
        Style::default().fg(RUST).bg(PANEL).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BONE).bg(PANEL)
    };
    lines.push(Line::from(Span::styled(app.status.clone(), status_style)));

    match &app.mode {
        Mode::Search(target) => {
            let label = match target {
                SearchTarget::Browser => "files",
                SearchTarget::Presets => "presets",
            };
            lines.push(Line::from(vec![
                Span::styled(
                    " /",
                    Style::default()
                        .fg(SILVER)
                        .bg(PANEL)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{label}: "), Style::default().fg(ASH).bg(PANEL)),
                Span::styled(
                    app.search_input.clone(),
                    Style::default()
                        .fg(BRONZE)
                        .bg(PANEL)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(SILVER).bg(PANEL)),
            ]));
            lines.push(Line::from(Span::styled(
                "Enter confirm  Esc cancel  jk cancel (vim)  type to filter live",
                Style::default().fg(ASH).bg(PANEL),
            )));
        }
        Mode::PresetPicker => {
            lines.push(Line::from(Span::styled(
                "j/k move  Enter apply  / search  x clear EQ  gg/G top/bottom  PgUp/Dn page  Esc back  ? help",
                Style::default().fg(ASH).bg(PANEL),
            )));
            if !app.presets.search.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("filter: /{}", app.presets.search),
                    Style::default().fg(BRONZE).bg(PANEL),
                )));
            } else if app.presets.loading {
                lines.push(Line::from(Span::styled(
                    "Loading AutoEq presets in the background…",
                    Style::default().fg(AMBER).bg(PANEL),
                )));
            }
        }
        Mode::Help => {
            lines.push(Line::from(Span::styled(
                "Press ? or Esc to close help",
                Style::default().fg(ASH).bg(PANEL),
            )));
        }
        Mode::Browser => {
            lines.push(Line::from(Span::styled(
                "j/k move  h/l browse  Enter play  e EQ  space pause  n/p next/prev  ,/. seek  +/- vol  ? help  q quit",
                Style::default().fg(ASH).bg(PANEL),
            )));
            if !app.browser.search.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("filter: /{}", app.browser.search),
                    Style::default().fg(BRONZE).bg(PANEL),
                )));
            }
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(STEEL).bg(VOID))
        .style(Style::default().bg(PANEL).fg(BONE))
        .title(Span::styled(
            " STATUS ",
            Style::default().fg(ASH).bg(VOID).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(PANEL)),
        area,
    );
}

fn draw_help_overlay(f: &mut Frame, area: Rect) {
    let popup = centered_rect(72, 80, area);
    // Dim the world behind the modal.
    f.render_widget(Clear, popup);

    let text = vec![
        Line::from(Span::styled(
            "EQTERM — KEYBOARD REFERENCE",
            Style::default()
                .fg(SILVER)
                .bg(PANEL)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled("", Style::default().bg(PANEL))),
        section("NAVIGATION"),
        help_line("j / ↓", "move down"),
        help_line("k / ↑", "move up"),
        help_line("h / ←", "parent folder"),
        help_line("l / → / Enter", "open folder or play track"),
        help_line("gg / G", "jump top / bottom"),
        help_line("PgUp / PgDn", "page up / down"),
        help_line("Ctrl-d / Ctrl-u", "half-page down / up"),
        help_line("r", "refresh current folder"),
        Line::from(Span::styled("", Style::default().bg(PANEL))),
        section("PLAYBACK"),
        help_line("space", "pause / resume"),
        help_line("n / p", "next / previous track"),
        help_line("s", "stop"),
        help_line(", / .", "seek −5s / +5s"),
        help_line("[ / ]", "seek −30s / +30s"),
        help_line("+ / −", "volume up / down"),
        Line::from(Span::styled("", Style::default().bg(PANEL))),
        section("EQ PRESETS"),
        help_line("e", "open preset picker"),
        help_line("Enter", "apply selected preset"),
        help_line("x", "clear EQ (flat response)"),
        help_line("/", "filter list (live)"),
        help_line("Esc", "back to library"),
        Line::from(Span::styled("", Style::default().bg(PANEL))),
        section("GENERAL"),
        help_line("?", "toggle this help"),
        help_line("Ctrl-l", "clear active filter"),
        help_line("q / Ctrl-c", "quit"),
        Line::from(Span::styled("", Style::default().bg(PANEL))),
        Line::from(Span::styled(
            "Presets load from AutoEq on first run and cache locally.",
            Style::default().fg(ASH).bg(PANEL),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(SILVER).bg(VOID))
        .style(Style::default().bg(PANEL).fg(BONE))
        .title(Span::styled(
            " HELP ",
            Style::default()
                .fg(VOID)
                .bg(SILVER)
                .add_modifier(Modifier::BOLD),
        ));

    f.render_widget(
        Paragraph::new(text)
            .block(block)
            .style(Style::default().bg(PANEL).fg(BONE))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("▸ {title}"),
        Style::default()
            .fg(BRONZE)
            .bg(PANEL)
            .add_modifier(Modifier::BOLD),
    ))
}

fn help_line(keys: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {keys:<18}"),
            Style::default()
                .fg(SILVER)
                .bg(PANEL)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(SMOKE).bg(PANEL)),
    ])
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn truncate_middle(s: &str, max: usize) -> String {
    if max < 4 || s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1) / 2;
    let chars: Vec<char> = s.chars().collect();
    let head: String = chars.iter().take(keep).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}
