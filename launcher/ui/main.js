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
function setProgress(done, total, file) {
  const bar = $("bar"); bar.hidden = false;
  $("fill").style.width = Math.min(100, (100 * done) / Math.max(1, total)).toFixed(1) + "%";
  $("detail").textContent = `${gb(done)} / ${gb(total)}${file ? "  ·  " + file.split("/").pop() : ""}`;
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
  if (!ui.busy) { $("detail").textContent = `${c.files} files, ${gb(c.bytes)} to download`; $("bar").hidden = true; }
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
  ui.user = await invoke("login");
  ui.state = await invoke("get_state");
  ui.check = await invoke("check");
});

const doInstall = () => run(async () => {
  $("status").textContent = ui.check && ui.check.installed ? "Updating…" : "Installing…";
  setProgress(0, ui.check ? ui.check.bytes : 1, "");
  ui.check = await invoke("install");
  ui.state = await invoke("get_state");
});

const doPlay = () => run(async () => {
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

T.event.listen("progress", (ev) => { const p = ev.payload; setProgress(p.done, p.total, p.file); });

// launcher self-update (signed, from GitHub Releases); the game update is separate
(async () => {
  try {
    const update = await T.updater.check();
    if (update) {
      $("launcher-update").hidden = false;
      $("btn-launcher-update").onclick = async () => {
        $("btn-launcher-update").disabled = true;
        try { await update.downloadAndInstall(); await T.process.relaunch(); }
        catch (e) { setError("Launcher update failed: " + (e && e.message ? e.message : e)); $("btn-launcher-update").disabled = false; }
      };
    }
  } catch (_) { /* offline or no release yet */ }
})();

refresh().catch((e) => setError(String(e)));
