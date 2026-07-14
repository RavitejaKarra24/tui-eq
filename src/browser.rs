use anyhow::{Context, Result};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use crate::util::normalized_search;

const SUPPORTED_EXTS: &[&str] = &[
    "mp3", "flac", "wav", "aac", "m4a", "ogg", "opus", "alac", "aiff", "wma", "mka",
];

#[derive(Clone, Debug)]
pub enum EntryKind {
    Dir,
    File,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
}

#[derive(Debug)]
pub struct BrowserState {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub search: String,
}

impl BrowserState {
    pub fn new(cwd: PathBuf, entries: Vec<Entry>) -> Self {
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

    pub fn refresh(&mut self) -> Result<()> {
        self.entries = list_entries(&self.cwd)?;
        self.apply_filter();
        Ok(())
    }

    pub fn apply_filter(&mut self) {
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

    pub fn set_search(&mut self, value: String) {
        self.search = value;
        self.apply_filter();
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.filtered
            .get(self.selected)
            .and_then(|idx| self.entries.get(*idx))
    }

    pub fn move_by(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as i32;
        let next = (self.selected as i32 + delta).clamp(0, len - 1);
        self.selected = next as usize;
    }

    pub fn page_by(&mut self, page_size: usize, forward: bool) {
        if self.filtered.is_empty() || page_size == 0 {
            return;
        }
        let delta = page_size as i32;
        self.move_by(if forward { delta } else { -delta });
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = self.filtered.len() - 1;
        }
    }
}

pub fn list_entries(dir: &Path) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read dir {}", dir.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
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

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_EXTS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}
