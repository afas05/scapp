use crate::windows_capture::WindowInfo;

pub fn enumerate_windows() -> Result<Vec<WindowInfo>, String> {
    let conn = match x11rb::connect(None) {
        Ok((conn, _)) => conn,
        Err(_) => return Err("Failed to connect to X11 display. Window capture requires X11.".to_string()),
    };

    let screen = &conn.setup().roots[0];
    let root = screen.root;

    use x11rb::protocol::xproto::*;
    use x11rb::connection::Connection;

    let net_client_list = intern_atom(&conn, false, b"_NET_CLIENT_LIST")
        .map_err(|e| format!("Failed to intern atom: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get atom reply: {:?}", e))?
        .atom;

    let net_wm_name = intern_atom(&conn, false, b"_NET_WM_NAME")
        .map_err(|e| format!("Failed to intern atom: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get atom reply: {:?}", e))?
        .atom;

    let utf8_string = intern_atom(&conn, false, b"UTF8_STRING")
        .map_err(|e| format!("Failed to intern atom: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get atom reply: {:?}", e))?
        .atom;

    let net_wm_pid = intern_atom(&conn, false, b"_NET_WM_PID")
        .map_err(|e| format!("Failed to intern atom: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get atom reply: {:?}", e))?
        .atom;

    let net_wm_state = intern_atom(&conn, false, b"_NET_WM_STATE")
        .map_err(|e| format!("Failed to intern atom: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get atom reply: {:?}", e))?
        .atom;

    let net_wm_state_hidden = intern_atom(&conn, false, b"_NET_WM_STATE_HIDDEN")
        .map_err(|e| format!("Failed to intern atom: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get atom reply: {:?}", e))?
        .atom;

    let client_list = get_property(&conn, false, root, net_client_list, AtomEnum::WINDOW, 0, 1024)
        .map_err(|e| format!("Failed to get client list: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get client list reply: {:?}", e))?;

    let windows: Vec<u32> = client_list.value32()
        .map(|iter| iter.collect())
        .unwrap_or_default();

    let mut result = Vec::new();

    for win in windows {
        // Check if hidden
        let state_reply = get_property(&conn, false, win, net_wm_state, AtomEnum::ATOM, 0, 64)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        let is_hidden = state_reply
            .and_then(|reply| reply.value32().map(|atoms| atoms.collect::<Vec<_>>()))
            .map(|atoms| atoms.contains(&net_wm_state_hidden))
            .unwrap_or(false);
        if is_hidden {
            continue;
        }

        // Get window title (_NET_WM_NAME with UTF8, fallback to WM_NAME)
        let title = get_property(&conn, false, win, net_wm_name, utf8_string, 0, 256)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| {
                if reply.value.is_empty() {
                    None
                } else {
                    String::from_utf8(reply.value).ok()
                }
            })
            .or_else(|| {
                get_property(&conn, false, win, AtomEnum::WM_NAME.into(), AtomEnum::STRING, 0, 256)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .and_then(|reply| String::from_utf8(reply.value).ok())
            })
            .unwrap_or_default();

        if title.is_empty() {
            continue;
        }

        // Get PID
        let pid = get_property(&conn, false, win, net_wm_pid, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut iter| iter.next()))
            .unwrap_or(0);

        // Get process name from /proc/<pid>/comm
        let process_name = if pid > 0 {
            std::fs::read_to_string(format!("/proc/{}/comm", pid))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };

        result.push(WindowInfo {
            id: win as u64,
            pid,
            title,
            process_name,
            thumbnail: String::new(),
        });
    }

    Ok(result)
}
