//! Consultas y manipulación de ventanas Win32, y hooks de eventos del sistema.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, GA_PARENT, GW_OWNER, GWL_EXSTYLE,
    GetAncestor, GetClassNameW, GetDesktopWindow, GetWindow, GetWindowLongW, GetWindowPlacement,
    GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
    OBJID_WINDOW, PostMessageW, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos, WINDOWPLACEMENT, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WS_EX_TOOLWINDOW,
};
use windows::core::PWSTR;

use super::geometry::Rect;

/// La ventana empieza a moverse o a cambiar de tamaño.
pub const EVENT_MOVE_START: u32 = EVENT_SYSTEM_MOVESIZESTART;
/// La ventana termina el movimiento o el cambio de tamaño.
pub const EVENT_MOVE_END: u32 = EVENT_SYSTEM_MOVESIZEEND;
/// La ventana cambió de posición o de tamaño.
pub const EVENT_LOCATION_CHANGE: u32 = EVENT_OBJECT_LOCATIONCHANGE;
/// La ventana se destruye.
pub const EVENT_DESTROY: u32 = EVENT_OBJECT_DESTROY;
/// La ventana deja de ser visible.
pub const EVENT_HIDE: u32 = EVENT_OBJECT_HIDE;

/// Eventos a los que se suscribe Window Memory.
///
/// Cada hook admite un rango de eventos; se instala uno por evento para no
/// recibir el flujo completo de notificaciones de objetos del sistema.
const HOOK_EVENTS: [u32; 5] = [
    EVENT_MOVE_START,
    EVENT_MOVE_END,
    EVENT_LOCATION_CHANGE,
    EVENT_DESTROY,
    EVENT_HIDE,
];

/// Ventana y mensaje a los que se reenvían los eventos.
struct Sink {
    window: AtomicIsize,
    message: AtomicU32,
}

static SINK: Sink = Sink {
    window: AtomicIsize::new(0),
    message: AtomicU32::new(0),
};

/// Hooks de eventos instalados; se retiran al soltar el valor.
///
/// Debe soltarse en el mismo hilo que los instaló, que es también el que tiene
/// el bucle de mensajes donde se entregan los eventos.
pub struct EventHooks {
    hooks: Vec<HWINEVENTHOOK>,
}

impl EventHooks {
    /// Suscribe los eventos de [`HOOK_EVENTS`] y publica cada uno en `window`
    /// con `message`: `wparam` lleva el evento y `lparam` la ventana.
    ///
    /// El procedimiento de callback solo publica el mensaje; nunca ejecuta
    /// trabajo que pueda bloquear al hilo emisor.
    pub fn install(window: HWND, message: u32) -> EventHooks {
        SINK.window.store(window.0 as isize, Ordering::SeqCst);
        SINK.message.store(message, Ordering::SeqCst);

        let mut hooks = Vec::with_capacity(HOOK_EVENTS.len());
        for event in HOOK_EVENTS {
            let hook = unsafe {
                SetWinEventHook(
                    event,
                    event,
                    None,
                    Some(on_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if hook.0.is_null() {
                continue;
            }
            hooks.push(hook);
        }
        EventHooks { hooks }
    }

    /// Número de hooks activos.
    pub fn count(&self) -> usize {
        self.hooks.len()
    }
}

impl Drop for EventHooks {
    fn drop(&mut self) {
        SINK.window.store(0, Ordering::SeqCst);
        for hook in self.hooks.drain(..) {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
    }
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    // Solo interesa el objeto «ventana» completo: los controles y los hijos no.
    if object != OBJID_WINDOW.0 || child as u32 != CHILDID_SELF || hwnd.0.is_null() {
        return;
    }
    let window = SINK.window.load(Ordering::SeqCst);
    if window == 0 {
        return;
    }
    let message = SINK.message.load(Ordering::SeqCst);
    let _ = unsafe {
        PostMessageW(
            HWND(window as *mut c_void),
            message,
            WPARAM(event as usize),
            LPARAM(hwnd.0 as isize),
        )
    };
}

pub fn is_window(hwnd: HWND) -> bool {
    !hwnd.0.is_null() && unsafe { IsWindow(hwnd) }.as_bool()
}

pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

pub fn is_minimized(hwnd: HWND) -> bool {
    unsafe { IsIconic(hwnd) }.as_bool()
}

pub fn is_maximized(hwnd: HWND) -> bool {
    unsafe { IsZoomed(hwnd) }.as_bool()
}

/// Ventana sin propietario cuyo padre es el escritorio.
pub fn is_top_level(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    unsafe { GetAncestor(hwnd, GA_PARENT) == GetDesktopWindow() }
}

pub fn has_owner(hwnd: HWND) -> bool {
    unsafe { GetWindow(hwnd, GW_OWNER) }
        .map(|owner| !owner.0.is_null())
        .unwrap_or(false)
}

pub fn is_tool_window(hwnd: HWND) -> bool {
    let extended = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    extended & WS_EX_TOOLWINDOW.0 != 0
}

/// `true` si DWM oculta la ventana (ventanas fantasma de aplicaciones UWP).
pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    result.is_ok() && cloaked != 0
}

/// Rectángulo de la ventana en píxeles físicos.
pub fn rect(hwnd: HWND) -> Option<Rect> {
    let mut value = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut value) }.ok()?;
    Some(Rect::from_rect(value))
}

/// Coloca la ventana en `rect` sin activarla ni alterar su orden Z.
pub fn place(hwnd: HWND, rect: Rect) -> bool {
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        )
        .is_ok()
    }
}

pub fn class_name(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 256];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    (length > 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

pub fn process_id(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

pub fn current_process_id() -> u32 {
    unsafe { GetCurrentProcessId() }
}

/// Estado de la ventana según `GetWindowPlacement`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub maximized: bool,
    pub minimized: bool,
    /// Posición normal: la que recupera la ventana al restaurarse.
    pub normal: Rect,
}

pub fn placement(hwnd: HWND) -> Option<Placement> {
    let mut raw = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(hwnd, &mut raw) }.ok()?;
    Some(Placement {
        maximized: raw.showCmd == SW_SHOWMAXIMIZED.0 as u32,
        minimized: raw.showCmd == SW_SHOWMINIMIZED.0 as u32,
        normal: Rect::from_rect(raw.rcNormalPosition),
    })
}

/// Nombre del ejecutable (con extensión, en minúsculas) del proceso `pid`.
pub fn executable_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    result.ok()?;

    let path = String::from_utf16_lossy(&buffer[..length as usize]);
    Path::new(&path)
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_constants_match_the_documented_values() {
        assert_eq!(EVENT_MOVE_START, 0x000A);
        assert_eq!(EVENT_MOVE_END, 0x000B);
        assert_eq!(EVENT_LOCATION_CHANGE, 0x800B);
        assert_eq!(EVENT_DESTROY, 0x8001);
        assert_eq!(EVENT_HIDE, 0x8003);
    }
}
