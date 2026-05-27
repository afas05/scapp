// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod gstreamer;
mod windows_capture;
#[cfg(target_os = "linux")]
mod linux_capture;

#[tauri::command]
async fn list_windows() -> Result<Vec<windows_capture::WindowInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| windows_capture::enumerate_windows())
        .await
        .map_err(|e| format!("Task join error: {}", e))?
}

#[tauri::command]
async fn list_monitors() -> Result<Vec<windows_capture::MonitorInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| windows_capture::enumerate_monitors())
        .await
        .map_err(|e| format!("Task join error: {}", e))?
}

#[tauri::command]
async fn list_audio_inputs() -> Result<Vec<gstreamer::MicDevice>, String> {
    tauri::async_runtime::spawn_blocking(|| gstreamer::list_audio_input_devices())
        .await
        .map_err(|e| format!("Task join error: {}", e))?
}

#[tauri::command]
async fn get_preview_frame() -> Result<String, String> {
    gstreamer::get_preview_frame()
}

#[tauri::command]
async fn start_stream(
    video_host: String,
    video_rtp_port: u16,
    video_rtcp_port: u16,
    audio_host: String,
    audio_rtp_port: u16,
    audio_rtcp_port: u16,
    window_handle: Option<u64>,
    process_pid: Option<u32>,
    monitor_index: Option<u32>,
    mic_enabled: Option<bool>,
    mic_device_id: Option<String>,
    mic_initially_muted: Option<bool>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        gstreamer::start_streaming(
            video_host,
            video_rtp_port,
            video_rtcp_port,
            audio_host,
            audio_rtp_port,
            audio_rtcp_port,
            window_handle,
            process_pid,
            monitor_index,
            mic_enabled.unwrap_or(false),
            mic_device_id,
            mic_initially_muted.unwrap_or(false),
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

#[tauri::command]
async fn set_mic_muted(muted: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || gstreamer::set_mic_muted(muted))
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn start_recording(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || gstreamer::start_recording(path))
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn stop_recording() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| gstreamer::stop_recording())
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
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| {
            #[cfg(windows)]
            {
                use tauri::Manager;

                let resource_dir = _app.path().resource_dir()
                    .expect("Failed to resolve resource directory");
                let gst_plugin_path = resource_dir.join("gstreamer-1.0");
                std::env::set_var("GST_PLUGIN_PATH", &gst_plugin_path);
                std::env::set_var("GST_PLUGIN_SYSTEM_PATH_1_0", "");
                std::env::set_var("GST_PLUGIN_SYSTEM_PATH", "");
                println!("[setup] GST_PLUGIN_PATH = {}", gst_plugin_path.display());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![start_stream, stop_stream, list_windows, list_monitors, list_audio_inputs, set_mic_muted, get_preview_frame, start_recording, stop_recording])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
