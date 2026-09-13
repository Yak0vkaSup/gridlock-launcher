//! Launcher settings on disk: the site to talk to, the sign-in token, where the game lives.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_SITE: &str = "https://gridlock-umber.vercel.app";

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub install_dir: Option<PathBuf>,
    #[serde(default)]
    pub site: Option<String>,
}

pub fn dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("GridLockLauncher")
}

fn file() -> PathBuf {
    dir().join("config.json")
}

pub fn load() -> Config {
    std::fs::read(file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir())?;
    let tmp = file().with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(cfg)?)?;
    std::fs::rename(tmp, file())?;
    Ok(())
}

impl Config {
    /// GRIDLOCK_SITE in the environment wins (for testing against a preview deployment).
    pub fn site(&self) -> String {
        std::env::var("GRIDLOCK_SITE")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| self.site.clone())
            .unwrap_or_else(|| DEFAULT_SITE.to_string())
            .trim_end_matches('/')
            .to_string()
    }

    pub fn install_dir(&self) -> PathBuf {
        self.install_dir.clone().unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(dir)
                .join("GridLock")
        })
    }
}
