use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    tauri_build::build();

    // Windows resolves DLL imports at process startup against the EXE's
    // directory — not subfolders. Runtime GStreamer DLLs (gstreamer-1.0-0.dll
    // and its transitive glib/gobject/... deps) must sit next to scapp.exe or
    // the loader refuses to start the process. Plugins are loaded later via
    // GST_PLUGIN_PATH, so they live in a separate `gstreamer-1.0/` subfolder.
    //
    // The Tauri bundler handles this for installers (see tauri.conf.json
    // `resources` map). For `cargo build` / `tauri dev`, this build script
    // mirrors the same layout into target/<profile>/.

    let manifest_dir = PathBuf::from(env_var("CARGO_MANIFEST_DIR"));
    let target_dir = locate_target_dir();

    let gst_src_bin = manifest_dir.join("gstreamer").join("bin");
    let gst_src_plugins = manifest_dir.join("gstreamer").join("plugins");

    if gst_src_bin.is_dir() {
        copy_dlls(&gst_src_bin, &target_dir);
        println!("cargo:rerun-if-changed={}", gst_src_bin.display());
    }

    if gst_src_plugins.is_dir() {
        let dest_plugins = target_dir.join("gstreamer-1.0");
        let _ = fs::create_dir_all(&dest_plugins);
        copy_dlls(&gst_src_plugins, &dest_plugins);
        println!("cargo:rerun-if-changed={}", gst_src_plugins.display());
    }
}

fn copy_dlls(src: &Path, dest: &Path) {
    let entries = match fs::read_dir(src) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("dll")) != Some(true) {
            continue;
        }
        let file_name = match path.file_name() {
            Some(n) => n,
            None => continue,
        };
        let dest_path = dest.join(file_name);
        if let (Ok(src_meta), Ok(dest_meta)) = (path.metadata(), dest_path.metadata()) {
            if src_meta.len() == dest_meta.len() {
                if let (Ok(s), Ok(d)) = (src_meta.modified(), dest_meta.modified()) {
                    if d >= s {
                        continue;
                    }
                }
            }
        }
        let _ = fs::copy(&path, &dest_path);
    }
}

fn locate_target_dir() -> PathBuf {
    let out_dir = PathBuf::from(env_var("OUT_DIR"));
    out_dir
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf())
        .unwrap_or(out_dir)
}

fn env_var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing env var {}", name))
}
