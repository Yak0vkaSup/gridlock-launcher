// Vanilla JS over Tauri's global API (withGlobalTauri). One button that is Sign in, Install/Update or Play.
const T = window.__TAURI__;
const invoke = T.core.invoke;
const $ = (id) => document.getElementById(id);

const ui = {
  state: null,      // from get_state
  user: null,       // from whoami / login
  check: null,      // from check / install
  busy: false,
};

const mb = (b) => (b / 1048576).toFixed(b >= 1048576 * 100 ? 0 : 1) + " MB";
const gb = (b) => (b >= 1073741824 ? (b / 1073741824).toFixed(2) + " GB" : mb(b));

function setError(msg) { const e = $("error"); e.hidden = !msg; e.textContent = msg || ""; }
function setProgress(done, total, file, downloaded) {
  const bar = $("bar"); bar.hidden = false;
  $("fill").style.width = Math.min(100, (100 * done) / Math.max(1, total)).toFixed(1) + "%";
  // done counts what is settled (reused from the old files or fetched); downloaded is the wire
  const net = downloaded === undefined ? done : downloaded;
  $("detail").textContent = `${gb(net)} downloaded${file ? "  ·  " + file.split("/").pop() : ""}`;
}

function render() {
  const s = ui.state, c = ui.check;
  $("launcher-version").textContent = s ? "v" + s.launcher_version : "";
  $("install-dir").textContent = s ? s.install_dir : "";
  $("user-name").textContent = ui.user ? (ui.user.name || ui.user.email || "signed in") : "";
  $("btn-logout").hidden = !(s && s.logged_in);
  const btn = $("btn-main");
  btn.disabled = ui.busy;
  $("btn-verify").disabled = ui.busy || !(s && s.installed_version);
  $("btn-dir").disabled = ui.busy;
  if (!s) return;
  if (!s.logged_in) {
    $("status").textContent = "Sign in to get the demo";
    $("detail").textContent = "Your browser opens once; the launcher remembers you.";
    $("bar").hidden = true;
    btn.textContent = "Sign in"; btn.onclick = doLogin; return;
  }
  if (!c) {
    $("status").textContent = ui.busy ? "Checking for updates…" : (s.installed_version ? `Installed ${s.installed_version}` : "Ready");
    if (!ui.busy) $("detail").textContent = "";
    btn.textContent = s.installed_version ? "Play" : "Install";
    btn.onclick = s.installed_version ? doPlay : doInstall; return;
  }
  if (c.up_to_date) {
    $("status").textContent = `Up to date · ${c.latest}`;
    if (!ui.busy) { $("detail").textContent = ""; $("bar").hidden = true; }
    btn.textContent = "Play"; btn.onclick = doPlay; return;
  }
  const fresh = !c.installed;
  $("status").textContent = fresh ? `Install ${c.latest}` : `Update ${c.installed} → ${c.latest}`;
  // an update reuses whatever the old files still hold, so the size is only an upper bound
  if (!ui.busy) { $("detail").textContent = fresh ? `${c.files} files, ${gb(c.bytes)}` : `${c.files} files, up to ${gb(c.bytes)}`; $("bar").hidden = true; }
  btn.textContent = fresh ? "Install" : "Update"; btn.onclick = doInstall;
}

async function run(fn) {
  if (ui.busy) return;
  ui.busy = true; setError(""); render();
  try { await fn(); }
  catch (e) {
    const msg = typeof e === "string" ? e : (e && e.message) || String(e);
    if (msg === "unauthorized") { ui.state = await invoke("get_state"); ui.user = null; ui.check = null; setError("Signed out. Please sign in again."); }
    else setError(msg);
  }
  finally { ui.busy = false; render(); }
}

async function refresh() {
  ui.state = await invoke("get_state");
  render();
  if (ui.state.logged_in) {
    await run(async () => {
      ui.user = await invoke("whoami");
      ui.check = await invoke("check");
    });
  }
}

const doLogin = () => run(async () => {
  $("status").textContent = "Waiting for the browser…";
  $("detail").textContent = "Finish signing in on the page that opened.";
  try { ui.user = await invoke("login"); }
  finally { hideLoginLink(); }
  ui.state = await invoke("get_state");
  ui.check = await invoke("check");
});

// The sign-in link, for when no browser shows up (a broken xdg-open, an odd desktop): the Rust side
// sends it as soon as it tried to open the browser, with whether that worked.
let loginLink = null;
function showLoginLink(text) {
  if (!loginLink) return;
  $("login-link-text").textContent = text;
  $("login-url").value = loginLink.url;
  $("login-link").hidden = false;
}
function hideLoginLink() { loginLink = null; $("login-link").hidden = true; }
T.event.listen("login-url", (ev) => {
  loginLink = ev.payload;
  if (!loginLink.opened) {
    $("status").textContent = "Sign in in your browser";
    $("detail").textContent = loginLink.error ? "No browser opened: " + loginLink.error : "No browser opened.";
    showLoginLink("Open this link yourself:");
  } else {
    setTimeout(() => showLoginLink("Browser didn't open? Use this link:"), 5000);
  }
});
$("login-url").onclick = (e) => e.target.select();
$("btn-copy").onclick = async () => {
  const el = $("login-url"); el.focus(); el.select();
  let ok = false;
  try { await navigator.clipboard.writeText(el.value); ok = true; } catch (_) {}
  if (!ok) { try { ok = document.execCommand("copy"); } catch (_) {} }
  $("btn-copy").textContent = ok ? "Copied" : "Select all and copy";
  setTimeout(() => { $("btn-copy").textContent = "Copy"; }, 1500);
};

const doInstall = () => run(async () => {
  $("status").textContent = ui.check && ui.check.installed ? "Updating…" : "Installing…";
  setProgress(0, ui.check ? ui.check.bytes : 1, "");
  ui.check = await invoke("install");
  ui.state = await invoke("get_state");
});

const doPlay = () => run(async () => {
  // never start a stale build: look at the manifest once more right before launching
  $("status").textContent = "Checking version…";
  ui.check = await invoke("check");
  if (!ui.check.up_to_date) {
    $("detail").textContent = "A new build is out. Update first.";
    return;
  }
  await invoke("play");
  $("detail").textContent = "Game started.";
});

$("btn-logout").onclick = () => run(async () => { await invoke("logout"); ui.user = null; ui.check = null; ui.state = await invoke("get_state"); });
$("btn-open").onclick = () => invoke("open_install_dir").catch((e) => setError(String(e)));
$("btn-dir").onclick = () => run(async () => {
  const picked = await T.dialog.open({ directory: true, multiple: false, title: "Choose where to keep GridLock" });
  if (!picked) return;
  await invoke("set_install_dir", { path: picked });
  ui.state = await invoke("get_state");
  ui.check = ui.state.logged_in ? await invoke("check") : null;
});
$("btn-verify").onclick = () => run(async () => {
  $("status").textContent = "Verifying files…";
  const bad = await invoke("verify");
  ui.state = await invoke("get_state");
  ui.check = await invoke("check");
  $("detail").textContent = bad ? `${bad} file(s) need to be downloaded again` : "All files OK";
  $("bar").hidden = true;
});

T.event.listen("progress", (ev) => { const p = ev.payload; setProgress(p.done, p.total, p.file, p.downloaded); });

// launcher self-update (signed, from GitHub Releases): installs itself and restarts before
// anything else happens, so nobody keeps an old launcher around. Offline: carry on as we are.
async function selfUpdate() {
  let update = null;
  try { update = await T.updater.check(); } catch (_) { return false; }
  if (!update) return false;
  ui.busy = true; render();
  $("status").textContent = `Updating launcher to ${update.version}…`;
  $("detail").textContent = "It restarts by itself.";
  try {
    await update.downloadAndInstall((ev) => {
      if (ev.event === "Progress" && ev.data && ev.data.chunkLength) selfUpdate.done = (selfUpdate.done || 0) + ev.data.chunkLength;
      if (ev.event === "Started" && ev.data && ev.data.contentLength) selfUpdate.total = ev.data.contentLength;
      if (selfUpdate.total) setProgress(selfUpdate.done || 0, selfUpdate.total, "launcher");
    });
    await T.process.relaunch();
    return true;
  } catch (e) {
    ui.busy = false;
    setError("Launcher update failed: " + (e && e.message ? e.message : e));
    $("launcher-update").hidden = false;
    $("btn-launcher-update").onclick = () => selfUpdate();
    return false;
  }
}

(async () => {
  if (await selfUpdate()) return;
  await refresh().catch((e) => setError(String(e)));
})();
