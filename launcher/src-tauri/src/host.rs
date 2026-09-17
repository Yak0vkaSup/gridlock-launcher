//! Child processes get the host's environment back, and URLs open with it.
//!
//! Inside the AppImage the AppRun prepends `$APPDIR` to PATH, LD_LIBRARY_PATH, XDG_DATA_DIRS and a
//! few more, and linuxdeploy's GTK hook points GTK_PATH, GDK_PIXBUF_MODULE_FILE, GIO_EXTRA_MODULES
//! and friends into the bundle. A host program started with that environment loads the bundled
//! Ubuntu 22.04 libraries instead of its own and dies with a symbol lookup error: that is how
//! `xdg-open` -> `gio open` failed and Sign in never showed a browser (tauri-apps/tauri#10617).
//! The game would pull the bundled libzstd/libelf/libffi into its Vulkan driver the same way.
//! Outside an AppImage (.deb, Windows, `cargo run`) nothing here changes anything.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// A command that runs with the host's environment, not the bundle's.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "linux")]
    if let Some(appdir) = std::env::var_os("APPDIR") {
        cmd.env_clear();
        cmd.envs(scrub(&appdir, std::env::vars_os()));
    }
    cmd
}

/// Opens a URL in the default browser.
#[cfg(target_os = "linux")]
pub fn open_url(url: &str) -> anyhow::Result<()> {
    xdg_open(OsStr::new(url))
}

/// Shows a folder in the file manager.
#[cfg(target_os = "linux")]
pub fn open_path(path: &Path) -> anyhow::Result<()> {
    xdg_open(path.as_os_str())
}

#[cfg(not(target_os = "linux"))]
pub fn open_url(url: &str) -> anyhow::Result<()> {
    Ok(tauri_plugin_opener::open_url(url, None::<&str>)?)
}

#[cfg(not(target_os = "linux"))]
pub fn open_path(path: &Path) -> anyhow::Result<()> {
    Ok(tauri_plugin_opener::open_path(path, None::<&str>)?)
}

/// The host's environment for a child: the variables the AppImage runtime and the GTK hook set for
/// the bundle are dropped, and the bundle's directories are cut out of the search paths the AppRun
/// prepended them to, which leaves whatever the user had there before.
#[cfg(target_os = "linux")]
pub fn scrub(
    appdir: &OsStr,
    env: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    // set for the bundle alone; the host never had them (the GTK hook overwrites the last five)
    const DROP: &[&str] = &[
        "APPDIR", "APPIMAGE", "OWD", "ARGV0",
        "GTK_THEME", "GDK_BACKEND", "GTK_DATA_PREFIX", "GTK_EXE_PREFIX", "GTK_IM_MODULE_FILE",
        "GTK_PATH", "GDK_PIXBUF_MODULE_FILE", "GIO_EXTRA_MODULES", "GSETTINGS_SCHEMA_DIR",
    ];
    // the AppRun prepends bundle directories to these; the host's own entries follow
    const PREFIXED: &[&str] = &[
        "PATH", "LD_LIBRARY_PATH", "XDG_DATA_DIRS", "PYTHONPATH", "PERLLIB", "QT_PLUGIN_PATH",
        "GST_PLUGIN_SYSTEM_PATH", "GST_PLUGIN_SYSTEM_PATH_1_0",
    ];

    let appdir = appdir.as_bytes();
    if appdir.is_empty() || appdir == b"/" {
        return env.into_iter().collect();
    }
    let in_bundle = |entry: &[u8]| {
        entry.strip_prefix(appdir).is_some_and(|rest| rest.is_empty() || rest[0] == b'/')
    };
    let mut out = Vec::new();
    for (k, v) in env {
        if DROP.iter().any(|d| k.as_os_str() == OsStr::new(d)) {
            continue;
        }
        if PREFIXED.iter().any(|p| k.as_os_str() == OsStr::new(p)) {
            let kept: Vec<&[u8]> = v
                .as_bytes()
                .split(|&b| b == b':')
                .filter(|e| !e.is_empty() && !in_bundle(e))
                .collect();
            if kept.is_empty() {
                continue;
            }
            out.push((k, std::ffi::OsString::from_vec(kept.join(&b':'))));
        } else {
            out.push((k, v));
        }
    }
    out
}

/// What the `open` crate does, with the host's environment and an answer: `xdg-open` and its
/// fallbacks, in its order. The first that spawns and exits 0 wins.
#[cfg(target_os = "linux")]
fn xdg_open(target: &OsStr) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let launchers: [(&str, &[&str]); 4] =
        [("xdg-open", &[]), ("gio", &["open"]), ("kde-open", &["--"]), ("gnome-open", &[])];
    let mut why = Vec::new();
    for (prog, args) in launchers {
        let mut cmd = command(prog);
        cmd.args(args)
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                why.push(format!("{prog}: {e}"));
                continue;
            }
        };
        // xdg-open comes back as soon as the desktop took the URL; on a bare window manager it
        // waits for the browser itself, so a launcher still alive after a moment counts as fine
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    why.push(format!("{prog}: {status}"));
                    break;
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                Ok(None) => {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return Ok(());
                }
                Err(e) => {
                    why.push(format!("{prog}: {e}"));
                    break;
                }
            }
        }
    }
    if why.is_empty() {
        anyhow::bail!("no xdg-open on this system");
    }
    anyhow::bail!("{}", why.join("; "))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::time::Duration;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))).collect()
    }

    fn get<'a>(out: &'a [(OsString, OsString)], k: &str) -> Option<&'a str> {
        out.iter().find(|(n, _)| n == k).map(|(_, v)| v.to_str().unwrap())
    }

    #[test]
    fn scrub_gives_the_host_environment_back() {
        let a = "/tmp/.mount_glXYZ";
        let out = scrub(
            OsStr::new(a),
            env(&[
                ("APPDIR", a),
                ("APPIMAGE", "/home/u/Downloads/GridLock.AppImage"),
                ("OWD", "/home/u"),
                ("HOME", "/home/u"),
                ("PATH", &format!("{a}/usr/bin/:{a}/usr/sbin/:{a}/usr/games/:{a}/bin/:{a}/sbin/:/usr/local/bin:/usr/bin:/bin")),
                // AppRun.c formats "<bundle dirs>:%s" with the old value, empty here: only bundle entries
                ("LD_LIBRARY_PATH", &format!("{a}/usr/lib/:{a}/usr/lib/x86_64-linux-gnu/:{a}/lib64/:")),
                ("XDG_DATA_DIRS", &format!("{a}/usr/share:/usr/share:{a}/usr/share/:/home/u/.local/share/flatpak/exports/share:/usr/local/share:/usr/share")),
                ("GTK_PATH", &format!("{a}//usr/lib/x86_64-linux-gnu/gtk-3.0:/usr/lib64/gtk-3.0")),
                ("GDK_PIXBUF_MODULE_FILE", &format!("{a}//usr/lib/x86_64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders.cache")),
                ("GDK_BACKEND", "x11"),
                ("GTK_THEME", "Adwaita:dark"),
                ("PYTHONPATH", &format!("{a}/usr/share/pyshared/:/opt/mine")),
                ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
            ]),
        );
        assert_eq!(get(&out, "PATH"), Some("/usr/local/bin:/usr/bin:/bin"));
        assert_eq!(get(&out, "LD_LIBRARY_PATH"), None, "nothing of the host's was there");
        assert_eq!(get(&out, "XDG_DATA_DIRS"), Some("/usr/share:/home/u/.local/share/flatpak/exports/share:/usr/local/share:/usr/share"));
        assert_eq!(get(&out, "PYTHONPATH"), Some("/opt/mine"), "the user's own entry stays");
        for gone in ["APPDIR", "APPIMAGE", "OWD", "GTK_PATH", "GDK_PIXBUF_MODULE_FILE", "GDK_BACKEND", "GTK_THEME"] {
            assert_eq!(get(&out, gone), None, "{gone} should be gone");
        }
        assert_eq!(get(&out, "HOME"), Some("/home/u"));
        assert_eq!(get(&out, "WEBKIT_DISABLE_DMABUF_RENDERER"), Some("1"));
    }

    #[test]
    fn scrub_only_cuts_the_bundle_itself() {
        let a = "/tmp/.mount_glXYZ";
        let out = scrub(OsStr::new(a), env(&[("PATH", &format!("{a}/usr/bin/:/tmp/.mount_glXYZ2/bin:/usr/bin"))]));
        assert_eq!(get(&out, "PATH"), Some("/tmp/.mount_glXYZ2/bin:/usr/bin"));
        let out = scrub(OsStr::new("/"), env(&[("PATH", "/usr/bin"), ("APPDIR", "/")]));
        assert_eq!(get(&out, "PATH"), Some("/usr/bin"));
        assert_eq!(get(&out, "APPDIR"), Some("/"), "a nonsense APPDIR changes nothing");
    }

    /// The real thing: GL_APPDIR_TEST=<extracted AppImage dir> (`./x.AppImage --appimage-extract`,
    /// squashfs-root). Sets the environment the AppRun would, then shows that the host's `gio`
    /// dies under it and lives with `command()`. Run with --test-threads=1: it edits the process env.
    #[test]
    #[ignore]
    fn appimage_env_kills_host_gio_and_scrub_saves_it() {
        let Ok(a) = std::env::var("GL_APPDIR_TEST") else { return };
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("APPDIR", &a);
        std::env::set_var("PATH", format!("{a}/usr/bin/:{a}/usr/sbin/:{a}/usr/games/:{a}/bin/:{a}/sbin/:{old_path}"));
        std::env::set_var("LD_LIBRARY_PATH", format!("{a}/usr/lib/:{a}/usr/lib/x86_64-linux-gnu/:{a}/lib/x86_64-linux-gnu/:"));
        std::env::set_var("XDG_DATA_DIRS", format!("{a}/usr/share/:/usr/share"));
        std::env::set_var("GIO_EXTRA_MODULES", format!("{a}/usr/lib/x86_64-linux-gnu/gio/modules"));

        let polluted = Command::new("/usr/bin/gio").arg("version").output().expect("gio exists");
        eprintln!("polluted: {} {}", polluted.status, String::from_utf8_lossy(&polluted.stderr).trim());
        let scrubbed = command("gio").arg("version").output().expect("gio exists");
        eprintln!("scrubbed: {} {}", scrubbed.status, String::from_utf8_lossy(&scrubbed.stdout).trim());
        assert!(!polluted.status.success(), "the bundled libraries should break the host's gio");
        assert!(scrubbed.status.success(), "with the host environment gio runs");

        let which = command("sh").arg("-c").arg("command -v xdg-open").output().unwrap();
        let which = String::from_utf8_lossy(&which.stdout);
        assert!(!which.contains(&a), "xdg-open must be the host's, got {which}");

        std::env::set_var("PATH", old_path);
        std::env::remove_var("APPDIR");
        std::env::remove_var("LD_LIBRARY_PATH");
    }

    /// End to end under the same fake AppRun environment: `open_url` must bring a browser to a
    /// page served here. Opens a tab on the desktop. GL_APPDIR_TEST as above, --test-threads=1.
    #[test]
    #[ignore]
    fn appimage_open_url_reaches_the_browser() {
        let Ok(a) = std::env::var("GL_APPDIR_TEST") else { return };
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("APPDIR", &a);
        std::env::set_var("PATH", format!("{a}/usr/bin/:{a}/usr/sbin/:{old_path}"));
        std::env::set_var("LD_LIBRARY_PATH", format!("{a}/usr/lib/:{a}/usr/lib/x86_64-linux-gnu/:"));
        std::env::set_var("GIO_EXTRA_MODULES", format!("{a}/usr/lib/x86_64-linux-gnu/gio/modules"));

        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/launcher-test");
        let t0 = std::time::Instant::now();
        open_url(&url).expect("open_url");
        eprintln!("open_url returned after {:?}", t0.elapsed());
        let req = server.recv_timeout(Duration::from_secs(20)).unwrap().expect("the browser never asked for the page");
        assert_eq!(req.url(), "/launcher-test");
        let _ = req.respond(tiny_http::Response::from_string("<h2>GridLock launcher test: the browser opened. Close this tab.</h2>").with_header(
            tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap(),
        ));
        eprintln!("browser fetched the page after {:?}", t0.elapsed());

        std::env::set_var("PATH", old_path);
        std::env::remove_var("APPDIR");
        std::env::remove_var("LD_LIBRARY_PATH");
        std::env::remove_var("GIO_EXTRA_MODULES");
    }
}
