// Discord Rich Presence integration.
//
// Publishes a "Playing StreamSnipe" activity to the locally-running Discord client
// while the user is live, including the app logo and a "Watch Stream" button that
// links to their stream on streamsnipe.live.
//
// All operations are best-effort: if Discord isn't running (or anything fails) we log
// and continue so that streaming is never blocked.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

// ---------------------------------------------------------------------------
// Configuration — these are public Discord identifiers, not secrets.
//
// APP_ID: the Application ID from the Discord Developer Portal.
// LOGO_ASSET_KEY: the key of the image uploaded under the application's
//                 "Rich Presence → Art Assets" section.
// ---------------------------------------------------------------------------
const APP_ID: &str = "1371963968804880456";
const LOGO_ASSET_KEY: &str = "scope_monogram";

lazy_static::lazy_static! {
    static ref CLIENT: Arc<Mutex<DiscordState>> = Arc::new(Mutex::new(DiscordState::new()));
}

struct DiscordState {
    client: Option<DiscordIpcClient>,
    connected: bool,
}

impl DiscordState {
    fn new() -> Self {
        DiscordState { client: None, connected: false }
    }

    /// Ensure we have a connected client. Returns false (without erroring) if Discord
    /// isn't reachable — e.g. the desktop app isn't running.
    fn ensure_connected(&mut self) -> bool {
        if self.connected {
            return true;
        }
        let client = self.client.get_or_insert_with(|| DiscordIpcClient::new(APP_ID));
        match client.connect() {
            Ok(_) => {
                self.connected = true;
                true
            }
            Err(e) => {
                println!("[discord] connect failed (is Discord running?): {}", e);
                false
            }
        }
    }
}

/// Publish the live-streaming activity. Non-fatal: always returns Ok, logging failures.
pub fn set_streaming(producer_id: String) -> Result<(), String> {
    let mut state = CLIENT.lock().map_err(|e| format!("discord state lock poisoned: {}", e))?;
    if !state.ensure_connected() {
        return Ok(());
    }

    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let watch_url = format!("https://streamsnipe.live?streamId={}", producer_id);

    let activity = activity::Activity::new()
        .details("Streaming via StreamSnipe")
        .state("Live now")
        .assets(
            activity::Assets::new()
                .large_image(LOGO_ASSET_KEY)
                .large_text("StreamSnipe"),
        )
        .buttons(vec![activity::Button::new("Watch Stream", watch_url.as_str())])
        .timestamps(activity::Timestamps::new().start(start));

    if let Some(client) = state.client.as_mut() {
        if let Err(e) = client.set_activity(activity) {
            println!("[discord] set_activity failed: {}", e);
            // Mark disconnected so the next attempt reconnects.
            state.connected = false;
        }
    }
    Ok(())
}

/// Clear the activity (stream ended). Non-fatal.
pub fn clear() -> Result<(), String> {
    let mut state = CLIENT.lock().map_err(|e| format!("discord state lock poisoned: {}", e))?;
    if !state.connected {
        return Ok(());
    }
    if let Some(client) = state.client.as_mut() {
        if let Err(e) = client.clear_activity() {
            println!("[discord] clear_activity failed: {}", e);
            state.connected = false;
        }
    }
    Ok(())
}
