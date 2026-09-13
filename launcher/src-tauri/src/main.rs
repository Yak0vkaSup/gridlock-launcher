// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's DMABUF renderer aborts on a number of NVIDIA / Wayland setups
    // ("Could not create default EGL display: EGL_BAD_PARAMETER"); the plain
    // path is more than enough for a launcher window. The user's own value wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    gridlock_launcher_lib::run()
}
