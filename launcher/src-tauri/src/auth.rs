//! Sign-in through the system browser: the launcher listens on a loopback port, opens the site's
//! /launcher/login page, and the page navigates back to http://127.0.0.1:<port>/callback?token=...

use anyhow::{anyhow, bail};
use std::time::{Duration, Instant};
use tiny_http::{Header, Response, Server};

const TIMEOUT: Duration = Duration::from_secs(300);

const DONE_PAGE: &str = "<!doctype html><html><head><meta charset='utf-8'><title>GridLock</title></head>\
<body style='margin:0;background:#0c0e11;color:#d9d5ca;font-family:system-ui,sans-serif;display:grid;place-items:center;height:100vh'>\
<div style='text-align:center'><h2 style='margin:0 0 8px'>Signed in</h2><p style='color:#8a8f99'>You can close this tab and go back to the launcher.</p></div></body></html>";

pub async fn login(site: &str) -> anyhow::Result<String> {
    let server = Server::http("127.0.0.1:0").map_err(|e| anyhow!("loopback listener: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow!("loopback listener has no ip address"))?
        .port();
    let url = format!("{site}/launcher/login?port={port}");
    tauri_plugin_opener::open_url(&url, None::<&str>)?;

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
