//! The game folder: what is installed, what the newest manifest wants, and the download that gets
//! from one to the other (content-addressed files, resumable, hash-checked, four at a time).

use crate::net::{Manifest, ManifestFile};
use anyhow::{anyhow, bail, Context};
use futures_util::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const INSTALLED_FILE: &str = "gridlock-installed.json";
const PARALLEL: usize = 4;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Installed {
    pub version: String,
    #[serde(default)]
    pub exec: String,
    #[serde(default)]
    pub files: Vec<InstalledFile>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstalledFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub exec: bool,
}

pub fn load_installed(dir: &Path) -> Option<Installed> {
    let bytes = std::fs::read(dir.join(INSTALLED_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_installed(dir: &Path, installed: &Installed) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{INSTALLED_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(installed)?)?;
    std::fs::rename(tmp, dir.join(INSTALLED_FILE))?;
    Ok(())
}

/// Files the manifest wants that are not on disk in the right version, and files to remove.
pub struct Plan {
    pub to_download: Vec<ManifestFile>,
    pub bytes: u64,
    pub to_delete: Vec<String>,
}

fn safe_relative(path: &str) -> anyhow::Result<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute()
        || p.components().any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        bail!("manifest path is not a plain relative path: {path}");
    }
    Ok(p.to_path_buf())
}

pub fn plan(dir: &Path, manifest: &Manifest, installed: Option<&Installed>) -> anyhow::Result<Plan> {
    let have: HashMap<&str, &InstalledFile> = installed
        .map(|i| i.files.iter().map(|f| (f.path.as_str(), f)).collect())
        .unwrap_or_default();
    let mut to_download = Vec::new();
    let mut bytes = 0u64;
    for f in &manifest.files {
        let rel = safe_relative(&f.path)?;
        let on_disk = std::fs::metadata(dir.join(&rel)).map(|m| m.len()).ok();
        let same = have.get(f.path.as_str()).is_some_and(|h| h.sha256 == f.sha256)
            && on_disk == Some(f.size);
        if !same {
            bytes += f.size;
            to_download.push(f.clone());
        }
    }
    let wanted: HashMap<&str, ()> = manifest.files.iter().map(|f| (f.path.as_str(), ())).collect();
    let to_delete = have
        .keys()
        .filter(|p| !wanted.contains_key(*p))
        .map(|p| p.to_string())
        .collect();
    Ok(Plan { to_download, bytes, to_delete })
}

#[derive(Serialize, Clone)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub file: String,
}

struct Reporter {
    app: AppHandle,
    done: AtomicU64,
    total: u64,
    last: Mutex<Instant>,
    current: Mutex<String>,
}

impl Reporter {
    fn add(&self, n: u64, file: &str) {
        let done = self.done.fetch_add(n, Ordering::Relaxed) + n;
        let mut last = self.last.lock().unwrap();
        if last.elapsed() >= Duration::from_millis(150) || done >= self.total {
            *last = Instant::now();
            *self.current.lock().unwrap() = file.to_string();
            let _ = self.app.emit("progress", Progress { done, total: self.total, file: file.to_string() });
        }
    }
}

async fn hash_existing(part: &Path, hasher: &mut Sha256) -> anyhow::Result<u64> {
    let mut fh = tokio::fs::File::open(part).await?;
    let mut buf = vec![0u8; 1 << 20];
    let mut n_total = 0u64;
    loop {
        let n = fh.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        n_total += n as u64;
    }
    Ok(n_total)
}

async fn fetch_one(
    client: reqwest::Client,
    f: ManifestFile,
    dir: PathBuf,
    rep: Arc<Reporter>,
) -> anyhow::Result<()> {
    let url = f.url.as_deref().ok_or_else(|| anyhow!("no download url for {}", f.path))?;
    let dest = dir.join(safe_relative(&f.path)?);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));

    // resume a half-downloaded .part if there is one and it is not already too big
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(meta) = tokio::fs::metadata(&part).await {
        if meta.len() < f.size {
            have = hash_existing(&part, &mut hasher).await.unwrap_or(0);
        } else {
            let _ = tokio::fs::remove_file(&part).await;
        }
    }
    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await.with_context(|| format!("request for {}", f.path))?;
    let status = resp.status();
    let mut file = if have > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
        rep.add(have, &f.path);
        tokio::fs::OpenOptions::new().append(true).open(&part).await?
    } else if status.is_success() {
        hasher = Sha256::new();
        tokio::fs::File::create(&part).await?
    } else {
        bail!("{} while downloading {}", status, f.path);
    };

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("stream for {}", f.path))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        rep.add(chunk.len() as u64, &f.path);
    }
    file.flush().await?;
    drop(file);

    let got = hex::encode(hasher.finalize());
    if got != f.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        bail!("hash mismatch for {} (got {}, want {})", f.path, &got[..12], &f.sha256[..12]);
    }
    if tokio::fs::metadata(&dest).await.is_ok() {
        let _ = tokio::fs::remove_file(&dest).await;
    }
    tokio::fs::rename(&part, &dest).await?;
    set_exec(&dest, f.exec)?;
    Ok(())
}

#[cfg(unix)]
fn set_exec(path: &Path, exec: bool) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if exec {
        let mut perm = std::fs::metadata(path)?.permissions();
        perm.set_mode(perm.mode() | 0o755);
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_exec(_path: &Path, _exec: bool) -> anyhow::Result<()> {
    Ok(())
}

/// Downloads everything in the plan, removes leftovers, records the new version.
pub async fn apply(app: AppHandle, dir: PathBuf, manifest: &Manifest, plan: Plan) -> anyhow::Result<()> {
    std::fs::create_dir_all(&dir)?;
    let rep = Arc::new(Reporter {
        app: app.clone(),
        done: AtomicU64::new(0),
        total: plan.bytes.max(1),
        last: Mutex::new(Instant::now() - Duration::from_secs(1)),
        current: Mutex::new(String::new()),
    });
    let client = crate::net::client();
    futures_util::stream::iter(plan.to_download.into_iter().map(|f| {
        let (client, dir, rep) = (client.clone(), dir.clone(), rep.clone());
        async move { fetch_one(client, f, dir, rep).await }
    }))
    .buffer_unordered(PARALLEL)
    .try_collect::<Vec<()>>()
    .await?;

    for old in plan.to_delete {
        if let Ok(rel) = safe_relative(&old) {
            let _ = std::fs::remove_file(dir.join(rel));
        }
    }
    // executable bits also for files that were already in place
    for f in manifest.files.iter().filter(|f| f.exec) {
        if let Ok(rel) = safe_relative(&f.path) {
            let _ = set_exec(&dir.join(rel), true);
        }
    }
    save_installed(
        &dir,
        &Installed {
            version: manifest.version.clone(),
            exec: manifest.exec.clone(),
            files: manifest
                .files
                .iter()
                .map(|f| InstalledFile { path: f.path.clone(), sha256: f.sha256.clone(), size: f.size, exec: f.exec })
                .collect(),
        },
    )?;
    let _ = app.emit("progress", Progress { done: rep.total, total: rep.total, file: String::new() });
    Ok(())
}

/// Re-hashes every recorded file; entries that do not match are dropped so the next check re-downloads them.
pub fn verify(dir: &Path, app: &AppHandle) -> anyhow::Result<usize> {
    let Some(mut installed) = load_installed(dir) else { return Ok(0) };
    let total: u64 = installed.files.iter().map(|f| f.size).sum();
    let mut done = 0u64;
    let mut bad = 0usize;
    let mut kept = Vec::with_capacity(installed.files.len());
    for f in installed.files.drain(..) {
        let ok = safe_relative(&f.path)
            .ok()
            .and_then(|rel| std::fs::File::open(dir.join(rel)).ok())
            .map(|mut fh| {
                let mut h = Sha256::new();
                std::io::copy(&mut fh, &mut h).map(|_| hex::encode(h.finalize()) == f.sha256).unwrap_or(false)
            })
            .unwrap_or(false);
        done += f.size;
        let _ = app.emit("progress", Progress { done, total: total.max(1), file: f.path.clone() });
        if ok {
            kept.push(f);
        } else {
            bad += 1;
        }
    }
    installed.files = kept;
    save_installed(dir, &installed)?;
    Ok(bad)
}
