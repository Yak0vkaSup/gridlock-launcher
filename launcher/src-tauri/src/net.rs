//! Talking to the site: who am I, what is the newest build.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub exec: bool,
    #[serde(default)]
    pub url: Option<String>,
    /// glb1: size of the blob in the store (what a fresh install downloads)
    #[serde(default)]
    pub stored: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest {
    pub version: String,
    pub platform: String,
    pub exec: String,
    #[serde(default)]
    pub total: u64,
    pub files: Vec<ManifestFile>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default, rename = "urlExpiresAt")]
    pub url_expires_at: Option<u64>,
    /// None = raw objects; "glb1" = block blobs (delta.rs)
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub block: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserInfo {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    NoBuild,
    Other(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "unauthorized"),
            ApiError::NoBuild => write!(f, "no build has been published for this platform yet"),
            ApiError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        ApiError::Other(format!("network: {e}"))
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("GridLockLauncher/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

async fn get_json<T: serde::de::DeserializeOwned>(url: &str, token: &str) -> Result<T, ApiError> {
    let resp = client().get(url).bearer_auth(token).send().await?;
    match resp.status().as_u16() {
        200 => Ok(resp.json::<T>().await?),
        401 | 403 => Err(ApiError::Unauthorized),
        404 => Err(ApiError::NoBuild),
        code => {
            let body = resp.text().await.unwrap_or_default();
            Err(ApiError::Other(format!("{code}: {}", body.chars().take(200).collect::<String>())))
        }
    }
}

pub async fn me(site: &str, token: &str) -> Result<UserInfo, ApiError> {
    get_json(&format!("{site}/api/me"), token).await
}

pub async fn manifest(site: &str, token: &str, platform: &str) -> Result<Manifest, ApiError> {
    get_json(&format!("{site}/api/manifest?platform={platform}"), token).await
}
