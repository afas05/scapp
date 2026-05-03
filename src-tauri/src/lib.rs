// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod gstreamer;

#[tauri::command]
async fn start_stream(
    video_host: String,
    video_rtp_port: u16,
    video_rtcp_port: u16,
    audio_host: String,
    audio_rtp_port: u16,
    audio_rtcp_port: u16,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        gstreamer::start_streaming(
            video_host,
            video_rtp_port,
            video_rtcp_port,
            audio_host,
            audio_rtp_port,
            audio_rtcp_port,
        )
    })
    .await
    .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn stop_stream() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        gstreamer::stop_streaming()
    })
    .await
    .map_err(|e| format!("Failed to join task: {}", e))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            use tauri::Manager;

            // Runtime DLLs (gstreamer-1.0-0.dll & deps) are bundled directly
            // next to the EXE — that's the only place Windows' loader looks
            // at process startup. Plugins go in `gstreamer-1.0/` and we point
            // GST_PLUGIN_PATH at that folder. See build.rs and tauri.conf.json
            // `resources` map for the wiring.
            let resource_dir = app.path().resource_dir()
                .expect("Failed to resolve resource directory");
            let gst_plugin_path = resource_dir.join("gstreamer-1.0");
            std::env::set_var("GST_PLUGIN_PATH", &gst_plugin_path);

            // Prevent GStreamer from scanning the host machine's system plugin dirs
            std::env::set_var("GST_PLUGIN_SYSTEM_PATH_1_0", "");
            std::env::set_var("GST_PLUGIN_SYSTEM_PATH", "");

            println!("[setup] GST_PLUGIN_PATH = {}", gst_plugin_path.display());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![start_stream, stop_stream])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
