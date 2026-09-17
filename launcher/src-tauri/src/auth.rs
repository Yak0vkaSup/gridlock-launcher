//! Sign-in through the system browser: the launcher listens on a loopback port, opens the site's
//! /launcher/login page, and the page navigates back to http://127.0.0.1:<port>/callback?token=...
//! The window gets the URL as a `login-url` event, so the page can hand it to the user when no
//! browser turned up.

use crate::host;
use anyhow::{anyhow, bail};
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tiny_http::{Header, Response, Server};

const TIMEOUT: Duration = Duration::from_secs(300);

const DONE_PAGE: &str = "<!doctype html><html><head><meta charset='utf-8'><title>GridLock</title></head>\
<body style='margin:0;background:#0c0e11;color:#d9d5ca;font-family:system-ui,sans-serif;display:grid;place-items:center;height:100vh'>\
<div style='text-align:center'><h2 style='margin:0 0 8px'>Signed in</h2><p style='color:#8a8f99'>You can close this tab and go back to the launcher.</p></div></body></html>";

#[derive(Serialize, Clone)]
struct LoginUrl {
    url: String,
    opened: bool,
    error: Option<String>,
}

pub async fn login(app: &AppHandle, site: &str) -> anyhow::Result<String> {
    let server = Server::http("127.0.0.1:0").map_err(|e| anyhow!("loopback listener: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow!("loopback listener has no ip address"))?
        .port();
    let url = format!("{site}/launcher/login?port={port}");
    let opened = {
        let url = url.clone();
        tokio::task::spawn_blocking(move || host::open_url(&url)).await?
    };
    // sign-in goes on either way: the page shows the link for the user to open by hand
    let _ = app.emit(
        "login-url",
        LoginUrl { url: url.clone(), opened: opened.is_ok(), error: opened.err().map(|e| e.to_string()) },
    );

    tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                bail!("sign-in timed out; press Sign in again");
            }
            let Some(req) = server.recv_timeout(left)? else { continue };
            let path = req.url().to_string();
            let token = path
                .strip_prefix("/callback?")
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
                .map(str::to_string);
            match token {
                Some(t) if !t.is_empty() => {
                    let resp = Response::from_string(DONE_PAGE)
                        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
                    let _ = req.respond(resp);
                    return Ok(t);
                }
                _ => {
                    let _ = req.respond(Response::from_string("not found").with_status_code(404));
                }
            }
        }
    })
    .await?
}
