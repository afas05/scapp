use serde::Serialize;

#[derive(Serialize)]
pub struct WindowInfo {
    pub id: u64,
    pub pid: u32,
    pub title: String,
    pub process_name: String,
    pub thumbnail: String, // base64 JPEG, empty if capture fails
}

pub fn enumerate_windows() -> Result<Vec<WindowInfo>, String> {
    #[cfg(windows)]
    return win::enumerate();
    #[cfg(not(windows))]
    Err("Window enumeration is only supported on Windows".to_string())
}

#[cfg(windows)]
mod win {
    use super::WindowInfo;
    use rayon::prelude::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible,
        GWL_EXSTYLE, GWL_STYLE,
        WS_CHILD, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_VISIBLE,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        GetDC, GetDIBits, ReleaseDC, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::core::PWSTR;

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> windows::Win32::Foundation::BOOL {
        if is_capturable(hwnd) {
            let list = &mut *(lparam.0 as *mut Vec<HWND>);
            list.push(hwnd);
        }
        windows::Win32::Foundation::BOOL(1)
    }

    unsafe fn is_capturable(hwnd: HWND) -> bool {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }

        let mut title = [0u16; 512];
        if GetWindowTextW(hwnd, &mut title) == 0 {
            return false;
        }

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;

        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        if style & WS_VISIBLE.0 == 0 || style & WS_CHILD.0 != 0 {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        if rect.right - rect.left <= 0 || rect.bottom - rect.top <= 0 {
            return false;
        }

        let mut class = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class);
        if class_len > 0 {
            let class_str = String::from_utf16_lossy(&class[..class_len as usize]);
            if matches!(class_str.as_str(), "Shell_TrayWnd" | "Progman" | "WorkerW") {
                return false;
            }
        }

        true
    }

    unsafe fn get_window_title(hwnd: HWND) -> String {
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..len as usize]).to_string()
    }

    unsafe fn get_process_info(hwnd: HWND) -> (u32, String) {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        let name = if pid > 0 {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let mut buf = [0u16; 1024];
                    let mut size = buf.len() as u32;
                    let n = if QueryFullProcessImageNameW(
                        handle,
                        PROCESS_NAME_WIN32,
                        PWSTR(buf.as_mut_ptr()),
                        &mut size,
                    ).is_ok() {
                        let path = String::from_utf16_lossy(&buf[..size as usize]);
                        std::path::Path::new(&path)
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let _ = CloseHandle(handle);
                    n
                }
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        (pid, name)
    }

    unsafe fn capture_thumbnail(hwnd: HWND) -> String {
        // UWP / XAML apps redirect via composition; PrintWindow returns black for them.
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_NOREDIRECTIONBITMAP.0 != 0 {
            return String::new();
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return String::new();
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 || width > 16384 || height > 16384 {
            return String::new();
        }

        let screen_dc = GetDC(HWND(std::ptr::null_mut()));
        if screen_dc.is_invalid() {
            return String::new();
        }

        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
            return String::new();
        }

        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);
            return String::new();
        }

        let old_obj = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

        // PW_RENDERFULLCONTENT (0x2): captures the window even when occluded/minimized
        let print_ok = PrintWindow(hwnd, mem_dc, PRINT_WINDOW_FLAGS(2)).as_bool();

        let result = if print_ok {
            extract_jpeg(mem_dc, bitmap, width, height)
        } else {
            String::new()
        };

        SelectObject(mem_dc, old_obj);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(HWND(std::ptr::null_mut()), screen_dc);

        result
    }

    unsafe fn extract_jpeg(
        mem_dc: windows::Win32::Graphics::Gdi::HDC,
        bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
        width: i32,
        height: i32,
    ) -> String {
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative = top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let pixel_count = (width * height) as usize;
        let mut pixels = vec![0u8; pixel_count * 4];

        let got = GetDIBits(
            mem_dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        if got == 0 {
            return String::new();
        }

        // Windows DIB is BGRA; convert to RGB
        let mut rgb = Vec::with_capacity(pixel_count * 3);
        for p in pixels.chunks_exact(4) {
            rgb.push(p[2]); // R
            rgb.push(p[1]); // G
            rgb.push(p[0]); // B
        }

        // Black frame → sandboxed/failed capture
        if rgb.iter().all(|&b| b == 0) {
            return String::new();
        }

        let img = match image::RgbImage::from_raw(width as u32, height as u32, rgb) {
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

    pub fn enumerate() -> Result<Vec<WindowInfo>, String> {
        let mut hwnds: Vec<HWND> = Vec::new();
        unsafe {
            EnumWindows(
                Some(enum_cb),
                LPARAM(&mut hwnds as *mut Vec<HWND> as isize),
            )
            .map_err(|e| format!("EnumWindows failed: {e}"))?;
        }

        // HWND wraps a raw pointer (!Send/!Sync). Carry it across the rayon
        // boundary as an isize handle and rebuild HWND inside the worker.
        let infos: Vec<(isize, String, u32, String)> = hwnds
            .iter()
            .map(|&hwnd| {
                let (title, pid, proc_name) = unsafe {
                    let t = get_window_title(hwnd);
                    let (p, n) = get_process_info(hwnd);
                    (t, p, n)
                };
                (hwnd.0 as isize, title, pid, proc_name)
            })
            .collect();

        let results: Vec<WindowInfo> = infos
            .par_iter()
            .map(|(hwnd_handle, title, pid, proc_name)| {
                let hwnd = HWND(*hwnd_handle as *mut _);
                let thumbnail = unsafe { capture_thumbnail(hwnd) };
                WindowInfo {
                    id: *hwnd_handle as u64,
                    pid: *pid,
                    title: title.clone(),
                    process_name: proc_name.clone(),
                    thumbnail,
                }
            })
            .collect();

        Ok(results)
    }
}
