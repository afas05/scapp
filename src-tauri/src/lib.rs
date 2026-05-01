// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod gstreamer;

#[tauri::command]
async fn start_stream(host: String, rtp_port: u16, rtcp_port: u16) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        gstreamer::start_streaming(host, rtp_port, rtcp_port)
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
        .invoke_handler(tauri::generate_handler![start_stream, stop_stream])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
