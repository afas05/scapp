// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn get_exe_path() -> Result<PathBuf, String> {
    let exe_dir = env::current_exe()
        .map_err(|e| format!("Failed to get exe path: {}", e))?
        .parent()
        .ok_or("Failed to get parent directory")?
        .to_path_buf();

    Ok(exe_dir.join("bin").join("streaming_app.exe"))
}

#[tauri::command]
async fn start_stream() -> Result<String, String> {
    let exe_path = get_exe_path()?;

    let result = tauri::async_runtime::spawn_blocking(|| {
        let output = Command::new(exe_path).arg("some_argument").output();

        output.map_err(|e| format!("Failed to run exe: {}", e))
    })
    .await
    .map_err(|e| format!("Failed to join task: {}", e))?;

    let output = result?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![start_stream])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
