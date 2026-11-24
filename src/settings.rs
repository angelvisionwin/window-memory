//! Diálogo modal para editar la configuración.

use std::ffi::c_void;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, DeleteObject, GetMonitorInfoW, HBRUSH, HFONT, HGDIOBJ, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateWindowExW,
    DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL, ES_NUMBER, ES_RIGHT,
    GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW, IDC_ARROW, IsDialogMessageW,
    IsWindow, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WNDCLASSW, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_EX_TOOLWINDOW, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};
use windows::core::{HSTRING, w};

use crate::config::{Config, General};
use crate::meta;
use crate::platform::dialog::{self, SS_CENTERIMAGE};
use crate::strings;

const ID_REFRESH: i32 = 101;
const ID_FOLDER: i32 = 102;
const ID_MEMORY: i32 = 103;
const ID_AUTO_RESTORE: i32 = 104;
// IDOK e IDCANCEL: `IsDialogMessageW` envía estos identificadores con Intro y Escape.
const ID_SAVE: i32 = 1;
const ID_CANCEL: i32 = 2;

// `BST_UNCHECKED` y `BST_CHECKED`: valores de un botón de verificación.
const BUTTON_UNCHECKED: usize = 0;
const BUTTON_CHECKED: usize = 1;

const WIDTH: f32 = 380.0;
const MARGIN: f32 = 16.0;
const ROW: f32 = 24.0;
const NUMBER: f32 = 80.0;
const BUTTON: f32 = 28.0;

#[derive(Clone, Copy)]
struct Controls {
    memory: HWND,
    auto_restore: HWND,
    refresh: HWND,
    font: HFONT,
}

struct State {
    parent: HWND,
    controls: Controls,
    current: Config,
    saved: Option<Config>,
    out: *mut Option<Config>,
}

static OPENED: std::sync::Mutex<isize> = std::sync::Mutex::new(0);
static REGISTERED: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

pub fn is_open() -> bool {
    *OPENED.lock().unwrap() != 0
}

pub fn focus() {
    let address = *OPENED.lock().unwrap();
    if address != 0 {
        unsafe {
            let _ = SetForegroundWindow(HWND(address as *mut c_void));
        }
    }
}

/// Abre el diálogo y devuelve la configuración guardada, si la hay.
pub unsafe fn open(parent: HWND, current: Config) -> Option<Config> {
    if is_open() {
        focus();
        return None;
    }

    let instance = HINSTANCE(GetModuleHandleW(None).ok()?.0);
    let class = meta::WINDOW_CLASS_SETTINGS;
    let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

    let mut registered = REGISTERED.lock().unwrap();
    if !*registered {
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: cursor,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 as usize + 1) as *mut c_void),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            meta::log("settings: no se pudo registrar la clase");
            return None;
        }
        *registered = true;
    }
    drop(registered);

    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let _ = GetMonitorInfoW(monitor, &mut monitor_info);
    let work = monitor_info.rcWork;

    // El diálogo aparece en el monitor del cursor, así que su escala puede no
    // coincidir con la de la ventana principal.
    let dpi = {
        let mut dpi_x = 0u32;
        let mut dpi_y = 0u32;
        match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
            Ok(()) if dpi_x > 0 => dpi_x,
            _ => {
                let dpi = GetDpiForWindow(parent);
                if dpi == 0 { 96 } else { dpi }
            }
        }
    };
    let scale = dpi as f32 / 96.0;
    let px = |value: f32| (value * scale).round() as i32;

    let m = px(MARGIN);
    let row = px(ROW);
    let client_width = px(WIDTH);
    let right = client_width - m;
    let label_width = right - m - px(NUMBER) - px(12.0);

    let y_memory = m;
    let y_auto = y_memory + row + px(6.0);
    let y_field = y_auto + row + px(14.0);
    let y_buttons = y_field + row + px(28.0);
    let client_height = y_buttons + px(BUTTON) + m;

    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
    let ex_style = WS_EX_TOOLWINDOW;
    let (width, height) = dialog::window_size(style, ex_style, client_width, client_height, dpi);

    let x = work.left + ((work.right - work.left) - width).max(0) / 2;
    let y = work.top + ((work.bottom - work.top) - height).max(0) / 2;

    let title = HSTRING::from(strings::SETTINGS_TITLE);
    let Ok(dialog_hwnd) = CreateWindowExW(
        ex_style,
        class,
        &title,
        style,
        x,
        y,
        width,
        height,
        HWND(std::ptr::null_mut()),
        None,
        instance,
        None,
    ) else {
        meta::log("settings: no se pudo crear la ventana");
        return None;
    };

    let font = dialog::font(px(12.0), 400);

    let controls = (|| -> Option<Controls> {
        let check = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32);
        let memory = dialog::child(
            dialog_hwnd,
            w!("BUTTON"),
            strings::SETTINGS_MEMORY,
            check,
            WINDOW_EX_STYLE(0),
            dialog::rect(m, y_memory, right - m, row),
            ID_MEMORY,
            font,
        )?;
        let auto_restore = dialog::child(
            dialog_hwnd,
            w!("BUTTON"),
            strings::SETTINGS_AUTO_RESTORE,
            check,
            WINDOW_EX_STYLE(0),
            dialog::rect(m, y_auto, right - m, row),
            ID_AUTO_RESTORE,
            font,
        )?;

        dialog::child(
            dialog_hwnd,
            w!("STATIC"),
            strings::SETTINGS_REFRESH,
            WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE,
            WINDOW_EX_STYLE(0),
            dialog::rect(m, y_field, label_width, row),
            0,
            font,
        )?;

        let number_style = WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WINDOW_STYLE((ES_AUTOHSCROLL | ES_NUMBER | ES_RIGHT) as u32);
        let refresh = dialog::child(
            dialog_hwnd,
            w!("EDIT"),
            "",
            number_style,
            WS_EX_CLIENTEDGE,
            dialog::rect(right - px(NUMBER), y_field, px(NUMBER), row),
            ID_REFRESH,
            font,
        )?;

        let button = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
        dialog::child(
            dialog_hwnd,
            w!("BUTTON"),
            strings::SETTINGS_OPEN_FOLDER,
            button | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            WINDOW_EX_STYLE(0),
            dialog::rect(
                right - px(96.0) - px(8.0) - px(96.0) - px(8.0) - px(130.0),
                y_buttons,
                px(130.0),
                px(BUTTON),
            ),
            ID_FOLDER,
            font,
        )?;
        dialog::child(
            dialog_hwnd,
            w!("BUTTON"),
            strings::SETTINGS_CANCEL,
            button | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            WINDOW_EX_STYLE(0),
            dialog::rect(
                right - px(96.0) - px(8.0) - px(96.0),
                y_buttons,
                px(96.0),
                px(BUTTON),
            ),
            ID_CANCEL,
            font,
        )?;
        dialog::child(
            dialog_hwnd,
            w!("BUTTON"),
            strings::SETTINGS_SAVE,
            button | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            WINDOW_EX_STYLE(0),
            dialog::rect(right - px(96.0), y_buttons, px(96.0), px(BUTTON)),
            ID_SAVE,
            font,
        )?;

        Some(Controls {
            memory,
            auto_restore,
            refresh,
            font,
        })
    })();

    let Some(controls) = controls else {
        meta::log("settings: no se pudieron crear los controles");
        let _ = DestroyWindow(dialog_hwnd);
        let _ = DeleteObject(HGDIOBJ(font.0));
        return None;
    };

    let mut result: Option<Config> = None;
    let state = Box::into_raw(Box::new(State {
        parent,
        controls,
        current: current.clone(),
        saved: None,
        out: &mut result,
    }));
    SetWindowLongPtrW(dialog_hwnd, GWLP_USERDATA, state as isize);
    *OPENED.lock().unwrap() = dialog_hwnd.0 as isize;

    dialog::set_text(
        controls.refresh,
        &current.refresh_interval_secs().to_string(),
    );
    set_checked(controls.memory, current.enabled());
    set_checked(controls.auto_restore, current.auto_restore());

    let _ = EnableWindow(parent, false);
    let _ = ShowWindow(dialog_hwnd, SW_SHOW);
    let _ = SetForegroundWindow(dialog_hwnd);

    let mut message = MSG::default();
    while IsWindow(dialog_hwnd).as_bool() {
        let got = GetMessageW(&mut message, HWND(std::ptr::null_mut()), 0, 0);
        if got.0 <= 0 {
            if got.0 == 0 {
                let _ = DestroyWindow(dialog_hwnd);
                PostQuitMessage(message.wParam.0 as i32);
            }
            break;
        }
        if !IsDialogMessageW(dialog_hwnd, &message).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    result
}

extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                // BN_CLICKED
                if ((wparam.0 >> 16) & 0xFFFF) == 0 {
                    match id {
                        ID_SAVE => save(hwnd),
                        ID_CANCEL => {
                            let _ = DestroyWindow(hwnd);
                        }
                        ID_FOLDER => open_folder(),
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                if !pointer.is_null() {
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let state = Box::from_raw(pointer);
                    if let Some(config) = state.saved.clone() {
                        *state.out = Some(config);
                    }
                    let _ = DeleteObject(HGDIOBJ(state.controls.font.0));
                    let _ = EnableWindow(state.parent, true);
                }
                *OPENED.lock().unwrap() = 0;
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe fn save(hwnd: HWND) {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if pointer.is_null() {
        return;
    }
    let state = &mut *pointer;

    let refresh = match parse_number(
        &dialog::get_text(state.controls.refresh),
        state.current.refresh_interval_secs(),
    ) {
        Some(value) if (10..=3600).contains(&value) => value,
        _ => {
            dialog::warn(
                hwnd,
                strings::SETTINGS_TITLE,
                strings::SETTINGS_INVALID_REFRESH,
            );
            let _ = SetFocus(state.controls.refresh);
            return;
        }
    };

    let config = Config {
        general: General {
            enabled: is_checked(state.controls.memory),
            auto_restore: is_checked(state.controls.auto_restore),
        },
        refresh_seconds: refresh,
        ..state.current.clone()
    };

    if let Err(error) = config.save() {
        dialog::warn(
            hwnd,
            strings::SETTINGS_TITLE,
            &strings::save_failed(&error.to_string()),
        );
        return;
    }
    state.saved = Some(config);
    let _ = DestroyWindow(hwnd);
}

unsafe fn open_folder() {
    let dir = crate::config::dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Err(error) = std::process::Command::new("explorer.exe").arg(&dir).spawn() {
        meta::log(&format!("settings: no se pudo abrir la carpeta: {error}"));
    }
}

fn parse_number(text: &str, fallback: u64) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return Some(fallback);
    }
    text.parse().ok()
}

fn set_checked(hwnd: HWND, checked: bool) {
    let state = if checked {
        BUTTON_CHECKED
    } else {
        BUTTON_UNCHECKED
    };
    unsafe { SendMessageW(hwnd, BM_SETCHECK, WPARAM(state), LPARAM(0)) };
}

fn is_checked(hwnd: HWND) -> bool {
    let state = unsafe { SendMessageW(hwnd, BM_GETCHECK, WPARAM(0), LPARAM(0)) };
    state.0 as usize == BUTTON_CHECKED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_fall_back_when_empty() {
        assert_eq!(parse_number("", 60), Some(60));
        assert_eq!(parse_number("  45 ", 60), Some(45));
        assert_eq!(parse_number("0", 60), Some(0));
        assert_eq!(parse_number("abc", 60), None);
        assert_eq!(parse_number("99999999999999999999999", 60), None);
    }
}
