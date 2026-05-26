use crate::windows_capture::{MonitorInfo, WindowInfo};
use base64::{engine::general_purpose::STANDARD, Engine};
use x11rb::connection::Connection;

fn pixels_to_base64_jpeg(data: &[u8], depth: u8, width: u32, height: u32) -> String {
    if depth != 24 && depth != 32 {
        return String::new();
    }

    let pixel_count = (width as usize) * (height as usize);
    let bpp = 4usize;

    if data.len() < pixel_count * bpp {
        return String::new();
    }

    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for chunk in data[..pixel_count * bpp].chunks_exact(bpp) {
        rgb.push(chunk[2]); // R (from BGRX)
        rgb.push(chunk[1]); // G
        rgb.push(chunk[0]); // B
    }

    if rgb.iter().all(|&b| b == 0) {
        return String::new();
    }

    let img = match image::RgbImage::from_raw(width, height, rgb) {
        Some(i) => i,
        None => return String::new(),
    };

    let resized = image::imageops::resize(&img, 320, 180, image::imageops::FilterType::Triangle);
    let dyn_img = image::DynamicImage::ImageRgb8(resized);

    let mut jpeg_bytes = Vec::new();
    if dyn_img
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg_bytes),
            image::ImageFormat::Jpeg,
        )
        .is_err()
    {
        return String::new();
    }

    STANDARD.encode(&jpeg_bytes)
}

fn capture_window_thumbnail(
    conn: &x11rb::rust_connection::RustConnection,
    win: u32,
) -> String {
    use x11rb::protocol::composite::ConnectionExt as _;
    use x11rb::protocol::xproto::*;

    let geom = match get_geometry(conn, win) {
        Ok(c) => c.reply().ok(),
        Err(_) => None,
    };
    let geom = match geom {
        Some(g) if g.width > 0 && g.height > 0 && g.width <= 16384 && g.height <= 16384 => g,
        _ => return String::new(),
    };

    let (w, h) = (geom.width as u32, geom.height as u32);

    // Try Composite pixmap (works when a compositor is running — virtually all
    // modern Linux desktops). Falls back to direct GetImage if unavailable.
    let composite_thumb = (|| -> Option<String> {
        let pixmap: u32 = conn.generate_id().ok()?;
        conn.composite_name_window_pixmap(win, pixmap).ok()?.check().ok()?;

        let img = get_image(
            conn,
            ImageFormat::Z_PIXMAP,
            pixmap,
            0, 0,
            w as u16, h as u16,
            !0u32,
        )
        .ok()
        .and_then(|c| c.reply().ok());

        let _ = free_pixmap(conn, pixmap);

        let img = img?;
        let t = pixels_to_base64_jpeg(&img.data, img.depth, w, h);
        if t.is_empty() { None } else { Some(t) }
    })();

    if let Some(t) = composite_thumb {
        return t;
    }

    get_image(conn, ImageFormat::Z_PIXMAP, win, 0, 0, w as u16, h as u16, !0u32)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|img| pixels_to_base64_jpeg(&img.data, img.depth, w, h))
        .unwrap_or_default()
}

fn capture_root_region(
    conn: &x11rb::rust_connection::RustConnection,
    root: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> String {
    use x11rb::protocol::xproto::*;

    get_image(
        conn,
        ImageFormat::Z_PIXMAP,
        root,
        x as i16, y as i16,
        width as u16, height as u16,
        !0u32,
    )
    .ok()
    .and_then(|c| c.reply().ok())
    .map(|img| pixels_to_base64_jpeg(&img.data, img.depth, width, height))
    .unwrap_or_default()
}

pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>, String> {
    let (conn, _) = x11rb::connect(None)
        .map_err(|_| "Failed to connect to X11 display. Monitor enumeration requires X11.".to_string())?;

    let screen = &conn.setup().roots[0];
    let root = screen.root;

    use x11rb::protocol::randr::{self, ConnectionExt as _};

    let resources = conn.randr_get_screen_resources_current(root)
        .map_err(|e| format!("Failed to get screen resources: {:?}", e))?
        .reply()
        .map_err(|e| format!("Failed to get screen resources reply: {:?}", e))?;

    let mut monitors = Vec::new();

    for &output in &resources.outputs {
        let output_info = conn.randr_get_output_info(output, 0)
            .map_err(|e| format!("Failed to get output info: {:?}", e))?
            .reply()
            .map_err(|e| format!("Failed to get output info reply: {:?}", e))?;

        if output_info.connection != randr::Connection::CONNECTED {
            continue;
        }
        if output_info.crtc == 0 {
            continue;
        }

        let crtc_info = conn.randr_get_crtc_info(output_info.crtc, 0)
            .map_err(|e| format!("Failed to get CRTC info: {:?}", e))?
            .reply()
            .map_err(|e| format!("Failed to get CRTC info reply: {:?}", e))?;

        let name = String::from_utf8_lossy(&output_info.name).to_string();

        monitors.push(MonitorInfo {
            name,
            x: crtc_info.x as i32,
            y: crtc_info.y as i32,
            width: crtc_info.width as u32,
            height: crtc_info.height as u32,
            thumbnail: String::new(),
        });
    }

    monitors.sort_by_key(|m| (m.x, m.y));

    for mon in &mut monitors {
        mon.thumbnail = capture_root_region(&conn, root, mon.x, mon.y, mon.width, mon.height);
    }

    Ok(monitors)
}

pub fn enumerate_windows() -> Result<Vec<WindowInfo>, String> {
    let conn = match x11rb::connect(None) {
        Ok((conn, _)) => conn,
        Err(_) => return Err("Failed to connect to X11 display. Window capture requires X11.".to_string()),
    };

    let screen = &conn.setup().roots[0];
    let root = screen.root;

    use x11rb::protocol::xproto::*;

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
                get_property(&conn, false, win, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 256)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .and_then(|reply| String::from_utf8(reply.value).ok())
            })
            .unwrap_or_default();

        if title.is_empty() {
            continue;
        }

        let pid = get_property(&conn, false, win, net_wm_pid, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut iter| iter.next()))
            .unwrap_or(0);

        let process_name = if pid > 0 {
            std::fs::read_to_string(format!("/proc/{}/comm", pid))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let thumbnail = capture_window_thumbnail(&conn, win);

        result.push(WindowInfo {
            id: win as u64,
            pid,
            title,
            process_name,
            thumbnail,
        });
    }

    Ok(result)
}
