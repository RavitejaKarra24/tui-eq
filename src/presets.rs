use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use walkdir::WalkDir;

use crate::eq::{parse_eq_profile, Preset};
use crate::util::normalized_search;
use crate::{APP_NAME, AUTOEQ_REPO_URL, AUTOEQ_ZIP_URL};

#[derive(Debug)]
pub enum BgMessage {
    Status(String),
    PresetsLoaded(Vec<Preset>),
    Error(String),
}

#[derive(Debug)]
pub struct PresetState {
    pub presets: Vec<Preset>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub search: String,
    pub loading: bool,
}

impl PresetState {
    pub fn new() -> Self {
        Self {
            presets: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            search: String::new(),
            loading: true,
        }
    }

    pub fn set_presets(&mut self, presets: Vec<Preset>) {
        self.presets = presets;
        self.loading = false;
        self.apply_filter();
    }

    pub fn apply_filter(&mut self) {
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

    pub fn set_search(&mut self, value: String) {
        self.search = value;
        self.apply_filter();
    }

    pub fn selected_preset(&self) -> Option<&Preset> {
        self.filtered
            .get(self.selected)
            .and_then(|idx| self.presets.get(*idx))
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

pub fn default_autoeq_root() -> PathBuf {
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

pub fn start_autoeq_loader(
    tx: Sender<BgMessage>,
    autoeq_root: PathBuf,
    extra_preset_dirs: Vec<PathBuf>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<Preset>> {
            let results_dir = autoeq_results_dir(&autoeq_root);
            if !has_parametric_eq(&results_dir) {
                tx.send(BgMessage::Status(
                    "Downloading AutoEq results (first run)…".to_string(),
                ))
                .ok();
                download_autoeq(&autoeq_root, &results_dir, &tx)?;
            }

            tx.send(BgMessage::Status("Loading presets…".to_string()))
                .ok();
            let presets = load_presets_multi(&autoeq_root, &results_dir, &extra_preset_dirs)?;
            Ok(presets)
        })();

        match result {
            Ok(presets) => {
                let _ = tx.send(BgMessage::PresetsLoaded(presets));
            }
            Err(err) => {
                let _ = tx.send(BgMessage::Error(format!("AutoEq error: {err}")));
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
        "AutoEq git download failed, falling back to zip…".to_string(),
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
        "AutoEq".to_string(),
    )?);

    let custom_dir = autoeq_custom_dir(autoeq_root);
    if custom_dir.exists() {
        presets.extend(load_presets_with_label(&custom_dir, "Custom".to_string())?);
    }

    for dir in extra_preset_dirs {
        if dir.exists() {
            let label = dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Extra")
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
            preset.name = format!("{label} / {}", preset.name);
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
        "AutoEq: cloning results (git sparse checkout)…".to_string(),
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

    if !matches!(clone_status, Ok(status) if status.success()) {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(anyhow::anyhow!("git clone failed"));
    }

    tx.send(BgMessage::Status(
        "AutoEq: checking out results…".to_string(),
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

    if !sparse_status.map(|s| s.success()).unwrap_or(false) {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(anyhow::anyhow!("git sparse-checkout failed"));
    }

    tx.send(BgMessage::Status("AutoEq: copying results…".to_string()))
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
        .user_agent(format!("{APP_NAME}/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .context("build http client")?;

    let mut response = client
        .get(AUTOEQ_ZIP_URL)
        .send()
        .context("download AutoEq zip")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "download failed with status {}",
            response.status()
        ));
    }

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
        if downloaded.saturating_sub(last_report) >= 10 * 1024 * 1024 {
            let msg = if let Some(total) = total {
                format!(
                    "Downloading AutoEq results… {} / {} MB",
                    downloaded / (1024 * 1024),
                    total / (1024 * 1024)
                )
            } else {
                format!(
                    "Downloading AutoEq results… {} MB",
                    downloaded / (1024 * 1024)
                )
            };
            tx.send(BgMessage::Status(msg)).ok();
            last_report = downloaded;
        }
    }
    zip_file.flush()?;
    drop(zip_file);

    let zip_file = File::open(&zip_path).context("open AutoEq zip")?;
    let mut archive = zip::ZipArchive::new(zip_file).context("read AutoEq zip")?;

    fs::create_dir_all(results_dir).context("create results dir")?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();

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
    WalkDir::new(results_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .map(|n| n.ends_with("ParametricEQ.txt"))
                    .unwrap_or(false)
        })
}

fn parse_preset(path: &Path, results_dir: &Path) -> Result<Preset> {
    let content = fs::read_to_string(path).context("read preset file")?;
    let eq = parse_eq_profile(&content);
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
    // Drop trailing " / " if the file was named ParametricEQ.txt alone.
    name = name.trim().trim_end_matches('/').trim().to_string();
    if name.is_empty() {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        name
    }
}
