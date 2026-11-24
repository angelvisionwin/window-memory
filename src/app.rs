use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use crate::config::Config;
use crate::meta;
use crate::panel::{self, PanelView, RowView};
use crate::platform::{self, time};
use crate::window_manager::{Manager, Stats, WindowFacts};
use crate::{cli, settings, strings, tray};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESHED: u32 = WM_APP + 2;
const WM_WINDOW_EVENT: u32 = WM_APP + 3;
const TIMER_REFRESH: usize = 1;
const ID_PANEL: usize = 1;
const ID_REFRESH: usize = 2;
const ID_TEST_NOTIFICATION: usize = 3;
const ID_AUTOSTART: usize = 4;
const ID_CONFIG: usize = 5;
const ID_EXIT: usize = 6;
const ID_MEMORY: usize = 7;
const ID_AUTO_RESTORE: usize = 8;

/// Estado de la aplicación.
///
/// Vive en el hilo de la interfaz y su propietario es [`run`]: las ventanas solo
/// guardan un puntero prestado en `GWLP_USERDATA`. La mutabilidad interior es
/// necesaria porque los procedimientos de ventana se reentran (menús modales y
/// el diálogo de configuración), de modo que no puede existir más de una
/// referencia `&mut App` viva.
struct App {
    config: RefCell<Config>,
    view: RefCell<PanelView>,
    /// Memoria de geometrías y seguimiento de ventanas.
    memory: RefCell<Manager>,
    /// Hooks de eventos; presentes solo mientras Window Memory está activo.
    hooks: RefCell<Option<platform::window::EventHooks>>,

    refresh_tx: mpsc::Sender<Vec<RowView>>,
    refresh_rx: mpsc::Receiver<Vec<RowView>>,
    busy: Arc<AtomicBool>,
    pending_refresh: Cell<bool>,
    show_on_first: Cell<bool>,

    notified: RefCell<HashSet<String>>,
    notify_seq: Cell<u64>,

    panel: Cell<Option<HWND>>,
    hidden_at: Cell<Option<Instant>>,
    tray_icon: Cell<isize>,
}

impl App {
    fn new() -> App {
        let (refresh_tx, refresh_rx) = mpsc::channel();
        let config = Config::load();
        App {
            memory: RefCell::new(Manager::new(config.clone())),
            config: RefCell::new(config),
            view: RefCell::new(PanelView {
                title: meta::APP_NAME.to_string(),
                subtitle: None,
                footer: strings::FOOTER.to_string(),
                empty: (
                    strings::EMPTY_TITLE.to_string(),
                    strings::EMPTY_HINT.to_string(),
                ),
                rows: Vec::new(),
            }),
            hooks: RefCell::new(None),
            refresh_tx,
            refresh_rx,
            busy: Arc::new(AtomicBool::new(false)),
            pending_refresh: Cell::new(false),
            show_on_first: Cell::new(false),
            notified: RefCell::new(HashSet::new()),
            notify_seq: Cell::new(0),
            panel: Cell::new(None),
            hidden_at: Cell::new(None),
            tray_icon: Cell::new(0),
        }
    }

    fn config_snapshot(&self) -> Config {
        self.config.borrow().clone()
    }

    /// Texto del tooltip a partir de las filas visibles.
    fn tooltip(&self) -> String {
        let view = self.view.borrow();
        if view.rows.is_empty() {
            return format!("{} — {}", meta::APP_NAME, strings::EMPTY_TITLE);
        }
        let parts: Vec<String> = view
            .rows
            .iter()
            .map(|row| format!("{} {}", row.title, row.value))
            .collect();
        let mut tip = format!("{} — {}", meta::APP_NAME, parts.join(" · "));
        if tip.chars().count() > 120 {
            tip = tip.chars().take(117).collect::<String>() + "…";
        }
        tip
    }

    /// Notifica una sola vez por clave.
    fn notify_once(&self, hwnd: HWND, key: &str, title: &str, body: &str) {
        let announce = match self.notified.try_borrow_mut() {
            Ok(mut keys) => keys.insert(key.to_string()),
            Err(_) => false,
        };
        if announce {
            tray::notify(hwnd, title, body, true);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Red de seguridad: `WM_DESTROY` ya deja el contador a cero, así que esto
        // solo actúa si la ventana no llegó a destruirse.
        let handle = self.tray_icon.replace(0);
        if handle != 0 {
            unsafe {
                let _ = DestroyIcon(HICON(handle as *mut c_void));
            }
        }
    }
}

/// Puntero al estado asociado a una ventana.
///
/// No promete ninguna duración de vida: el llamante debe garantizar que `App`
/// sigue vivo mientras use la referencia obtenida.
fn app_from(hwnd: HWND) -> Option<NonNull<App>> {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut App;
    NonNull::new(raw)
}

/// Guarda el puntero recibido en `lpCreateParams` (se usa en `WM_NCCREATE`).
///
/// Devuelve `false` si la ventana no trae estado asociado.
unsafe fn attach_app(hwnd: HWND, lparam: LPARAM) -> bool {
    let create = lparam.0 as *const CREATESTRUCTW;
    if create.is_null() {
        return false;
    }
    let params = unsafe { (*create).lpCreateParams };
    if params.is_null() {
        return false;
    }
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, params as isize) };
    true
}

/// Retira el puntero de la ventana (se usa en `WM_NCDESTROY`). No libera nada.
unsafe fn detach_app(hwnd: HWND) -> Option<NonNull<App>> {
    let raw = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut App;
    NonNull::new(raw)
}

/// Destruye la ventana si sigue existiendo y confirma que ya no queda ninguna.
fn destroy_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return true;
        }
        if DestroyWindow(hwnd).is_err() {
            meta::log("no se pudo destruir una ventana");
        }
        !IsWindow(hwnd).as_bool()
    }
}

fn tray_icon(app: &App) -> Option<HICON> {
    let handle = app.tray_icon.get();
    (handle != 0).then_some(HICON(handle as *mut c_void))
}

pub(crate) fn run(args: cli::Args) {
    unsafe {
        let _ = platform::dpi::enable_per_monitor_v2();
        std::panic::set_hook(Box::new(|info| meta::log(&format!("panic: {info}"))));

        if args.dump {
            dump();
            return;
        }

        if !platform::instance::acquire(meta::MUTEX_NAME) {
            return;
        }

        // Propietario único del estado: se libera al final del bucle de mensajes.
        let app = Box::new(App::new());
        let app_ptr = NonNull::from(&*app);
        let config = app.config_snapshot();
        app.show_on_first.set(args.show);
        meta::log(&format!(
            "inicio v{} — refresco {}s",
            meta::VERSION,
            config.refresh_interval_secs()
        ));

        let Some(instance) = GetModuleHandleW(None)
            .ok()
            .map(|module| HINSTANCE(module.0))
        else {
            meta::log("no se pudo obtener el módulo");
            return;
        };

        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

        let main_class = meta::WINDOW_CLASS_MAIN;
        let main_wc = WNDCLASSW {
            lpfnWndProc: Some(main_wndproc),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: main_class,
            ..Default::default()
        };
        if RegisterClassW(&main_wc) == 0 {
            meta::log("no se pudo registrar la ventana principal");
            return;
        }

        let panel_class = meta::WINDOW_CLASS_PANEL;
        let panel_wc = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(panel_wndproc),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: panel_class,
            ..Default::default()
        };
        if RegisterClassW(&panel_wc) == 0 {
            meta::log("no se pudo registrar el panel");
            return;
        }

        let window_title = windows::core::HSTRING::from(meta::APP_NAME);
        let params = Some(app_ptr.as_ptr() as *const c_void);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            main_class,
            &window_title,
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            instance,
            params,
        ) else {
            meta::log("no se pudo crear la ventana principal");
            return;
        };

        let Ok(panel) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            panel_class,
            &window_title,
            WS_POPUP,
            0,
            0,
            0,
            0,
            hwnd,
            None,
            instance,
            params,
        ) else {
            meta::log("no se pudo crear el panel");
            if !destroy_window(hwnd) {
                meta::log("la ventana principal sigue viva; se conserva el estado");
                std::mem::forget(app);
            }
            return;
        };
        app.panel.set(Some(panel));

        let corner = 2u32; // DWMWCP_ROUND
        let _ = DwmSetWindowAttribute(
            panel,
            DWMWINDOWATTRIBUTE(33),
            &corner as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );

        let icon = match tray::load_app_icon() {
            Some(icon) => {
                app.tray_icon.set(icon.0 as isize);
                icon
            }
            None => {
                meta::log("no se pudo cargar el icono de la aplicación");
                HICON::default()
            }
        };
        if !tray::add(hwnd, WM_TRAY, icon, &app.tooltip()) {
            meta::log("no se pudo crear el icono de la bandeja");
        }

        sync_hooks(&app, hwnd);

        let _ = SetTimer(
            hwnd,
            TIMER_REFRESH,
            (config.refresh_interval_secs() * 1000) as u32,
            None,
        );

        request_refresh(&app, hwnd, "inicio");

        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND(std::ptr::null_mut()), 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        meta::log("fin");

        // Ninguna ventana puede sobrevivir al estado: se destruyen antes de liberarlo.
        let panel_gone = destroy_window(panel);
        let main_gone = destroy_window(hwnd);
        if panel_gone && main_gone {
            drop(app);
        } else {
            meta::log(
                "queda una ventana sin destruir; se conserva el estado hasta el fin del proceso",
            );
            std::mem::forget(app);
        }
    }
}

/// Datos que muestra la aplicación.
///
/// El panel no sondea el escritorio: solo refleja el estado que mantienen los
/// hooks de ventana.
fn snapshot(stats: &Stats) -> Vec<RowView> {
    let active = stats.enabled;
    vec![
        RowView {
            title: strings::ROW_MEMORY.to_string(),
            detail: strings::ROW_MEMORY_DETAIL.to_string(),
            value: if active {
                strings::STATE_ACTIVE
            } else {
                strings::STATE_PAUSED
            }
            .to_string(),
            percent: None,
            accent: if active {
                panel::Accent::Ok
            } else {
                panel::Accent::Neutral
            },
        },
        RowView {
            title: strings::ROW_AUTO_RESTORE.to_string(),
            detail: strings::ROW_AUTO_RESTORE_DETAIL.to_string(),
            value: if stats.auto_restore {
                strings::VALUE_YES
            } else {
                strings::VALUE_NO
            }
            .to_string(),
            percent: None,
            accent: match (active, stats.auto_restore) {
                (false, _) => panel::Accent::Neutral,
                (true, true) => panel::Accent::Ok,
                (true, false) => panel::Accent::Warn,
            },
        },
        RowView {
            title: strings::ROW_WINDOWS.to_string(),
            detail: strings::ROW_WINDOWS_DETAIL.to_string(),
            value: stats.tracked.to_string(),
            percent: None,
            accent: panel::Accent::Neutral,
        },
        RowView {
            title: strings::ROW_GEOMETRIES.to_string(),
            detail: strings::ROW_GEOMETRIES_DETAIL.to_string(),
            value: stats.geometries.to_string(),
            percent: None,
            accent: panel::Accent::Neutral,
        },
        RowView {
            title: strings::ROW_MONITORS.to_string(),
            detail: strings::ROW_MONITORS_DETAIL.to_string(),
            value: stats.monitors.to_string(),
            percent: None,
            accent: if stats.monitors == 0 {
                panel::Accent::Bad
            } else {
                panel::Accent::Neutral
            },
        },
    ]
}

fn request_refresh(app: &App, hwnd: HWND, reason: &str) {
    if app.busy.swap(true, Ordering::SeqCst) {
        // El trabajador en curso repetirá el refresco al terminar.
        app.pending_refresh.set(true);
        return;
    }
    meta::log(&format!("refresh ({reason})"));
    let sender = app.refresh_tx.clone();
    let busy = Arc::clone(&app.busy);
    let address = hwnd.0 as isize;
    let stats = match app.memory.try_borrow() {
        Ok(memory) => memory.stats(),
        Err(_) => Stats::default(),
    };
    std::thread::spawn(move || {
        let rows = match std::panic::catch_unwind(|| snapshot(&stats)) {
            Ok(rows) => rows,
            Err(_) => {
                meta::log("refresh: error inesperado");
                Vec::new()
            }
        };
        let _ = sender.send(rows);
        busy.store(false, Ordering::SeqCst);
        unsafe {
            let _ = PostMessageW(
                HWND(address as *mut c_void),
                WM_REFRESHED,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
}

unsafe fn on_refreshed(app: &App, hwnd: HWND) {
    let mut received = Vec::new();
    while let Ok(rows) = app.refresh_rx.try_recv() {
        received = rows;
    }

    let changed = match app.view.try_borrow() {
        Ok(view) => view.rows != received,
        Err(_) => true,
    };
    if let Ok(mut view) = app.view.try_borrow_mut() {
        view.rows = received.clone();
        let (hours, minutes, _) = time::local_time();
        view.subtitle = Some(format!("{} {hours:02}:{minutes:02}", strings::UPDATED));
    }

    if changed {
        for row in &received {
            meta::log(&format!("{} = {} ({})", row.title, row.value, row.detail));
        }
        meta::log(&format!("refresh: {} fila(s)", received.len()));
    }

    if let Some(icon) = tray_icon(app)
        && !tray::update(hwnd, icon, &app.tooltip())
    {
        meta::log("no se pudo actualizar la bandeja");
    }

    if app.show_on_first.replace(false) {
        show_panel(app);
    } else if let Some(panel) = app.panel.get()
        && IsWindowVisible(panel).as_bool()
    {
        panel::resize(panel, received.len());
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(panel, None, true);
    }
}

fn show_panel(app: &App) {
    let Some(panel) = app.panel.get() else {
        return;
    };
    let count = app.view.borrow().rows.len();
    unsafe {
        panel::show(panel, count);
    }
}

unsafe fn show_menu(app: &App, hwnd: HWND) {
    let autostart = platform::registry::autostart_enabled(meta::RUN_VALUE);
    let (enabled, auto_restore) = match app.memory.try_borrow() {
        Ok(memory) => (memory.enabled(), memory.auto_restore()),
        Err(_) => (false, false),
    };
    let items = [
        tray::MenuItem::Item {
            id: ID_PANEL,
            label: strings::MENU_SHOW,
            checked: false,
        },
        tray::MenuItem::Item {
            id: ID_REFRESH,
            label: strings::MENU_REFRESH,
            checked: false,
        },
        tray::MenuItem::Item {
            id: ID_TEST_NOTIFICATION,
            label: strings::MENU_TEST_NOTIFICATION,
            checked: false,
        },
        tray::MenuItem::Separator,
        tray::MenuItem::Item {
            id: ID_MEMORY,
            label: strings::MENU_MEMORY,
            checked: enabled,
        },
        tray::MenuItem::Item {
            id: ID_AUTO_RESTORE,
            label: strings::MENU_AUTO_RESTORE,
            checked: auto_restore,
        },
        tray::MenuItem::Separator,
        tray::MenuItem::Item {
            id: ID_AUTOSTART,
            label: strings::MENU_AUTOSTART,
            checked: autostart,
        },
        tray::MenuItem::Item {
            id: ID_CONFIG,
            label: strings::MENU_CONFIG,
            checked: false,
        },
        tray::MenuItem::Separator,
        tray::MenuItem::Item {
            id: ID_EXIT,
            label: strings::MENU_EXIT,
            checked: false,
        },
    ];

    match tray::menu(hwnd, &items) {
        Some(ID_REFRESH) => request_refresh(app, hwnd, "menú"),
        Some(ID_PANEL) => show_panel(app),
        Some(ID_TEST_NOTIFICATION) => {
            let seq = app.notify_seq.get();
            app.notify_seq.set(seq + 1);
            let key = format!("test:{seq}");
            app.notify_once(hwnd, &key, strings::NOTIFY_TITLE, strings::NOTIFY_TEST_BODY);
        }
        Some(ID_MEMORY) => {
            update_config(app, hwnd, |config| config.general.enabled = !enabled);
            meta::log(&format!("Window Memory activo -> {}", !enabled));
        }
        Some(ID_AUTO_RESTORE) => {
            update_config(app, hwnd, |config| {
                config.general.auto_restore = !auto_restore;
            });
            meta::log(&format!("restauración automática -> {}", !auto_restore));
        }
        Some(ID_AUTOSTART) => {
            let ok = platform::registry::set_autostart(meta::RUN_VALUE, !autostart);
            meta::log(&format!("inicio con Windows -> {ok}"));
        }
        Some(ID_CONFIG) => {
            if let Some(updated) = settings::open(hwnd, app.config_snapshot()) {
                apply_config(app, hwnd, updated);
            }
        }
        Some(ID_EXIT) => {
            let closed = DestroyWindow(hwnd);
            if closed.is_err() {
                meta::log("no se pudo cerrar la ventana principal");
            }
        }
        _ => {}
    }
}

/// Cambia, guarda y aplica la configuración desde el menú de la bandeja.
unsafe fn update_config(app: &App, hwnd: HWND, change: impl FnOnce(&mut Config)) {
    let mut config = app.config_snapshot();
    change(&mut config);
    if let Err(error) = config.save() {
        meta::log(&format!("config: no se pudo guardar: {error}"));
    }
    unsafe {
        apply_config(app, hwnd, config);
    }
}

/// Aplica en caliente la configuración guardada desde el diálogo.
unsafe fn apply_config(app: &App, hwnd: HWND, updated: Config) {
    if let Ok(mut config) = app.config.try_borrow_mut() {
        *config = updated.clone();
    }
    if let Ok(mut memory) = app.memory.try_borrow_mut() {
        memory.apply_config(updated.clone());
    }
    sync_hooks(app, hwnd);

    let title = windows::core::HSTRING::from(meta::APP_NAME);
    let _ = SetWindowTextW(hwnd, &title);
    if let Some(panel) = app.panel.get() {
        let _ = SetWindowTextW(panel, &title);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(panel, None, true);
    }

    let _ = SetTimer(
        hwnd,
        TIMER_REFRESH,
        (updated.refresh_interval_secs() * 1000) as u32,
        None,
    );
    meta::log(&format!(
        "configuración aplicada — Window Memory {}, restauración {}, refresco {}s",
        if updated.enabled() {
            "activo"
        } else {
            "pausado"
        },
        if updated.auto_restore() { "sí" } else { "no" },
        updated.refresh_interval_secs()
    ));
    request_refresh(app, hwnd, "configuración");
}

/// Instala o retira los hooks de ventana según el estado de Window Memory.
fn sync_hooks(app: &App, hwnd: HWND) {
    let enabled = app.memory.borrow().enabled();
    let mut hooks = app.hooks.borrow_mut();
    match (enabled, hooks.is_some()) {
        (true, false) => {
            let installed = platform::window::EventHooks::install(hwnd, WM_WINDOW_EVENT);
            meta::log(&format!("hooks de ventana: {}", installed.count()));
            *hooks = Some(installed);
        }
        (false, true) => {
            *hooks = None;
            meta::log("hooks de ventana retirados");
        }
        _ => {}
    }
}

/// Procesa un evento publicado por los hooks de ventana.
fn handle_window_event(app: &App, owner: HWND, target: HWND, event: u32) {
    let facts = WindowFacts::observe(target);
    let outcome = match app.memory.try_borrow_mut() {
        Ok(mut memory) => memory.on_event(event, facts.as_ref(), Instant::now()),
        Err(_) => return,
    };
    if let Some(rect) = outcome.place {
        if platform::window::place(target, rect) {
            if let Some(detail) = &outcome.detail {
                meta::log(&format!("restaurada {detail}"));
            }
        } else {
            meta::log("no se pudo colocar la ventana en la geometría recordada");
        }
    } else if let Some(detail) = &outcome.detail {
        meta::log(detail);
    }
    if outcome.changed {
        request_refresh(app, owner, "ventanas");
    }
}

extern "system" fn main_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            // Sin estado asociado no se admite la ventana.
            if !attach_app(hwnd, lparam) {
                meta::log("no se pudo asociar el estado a la ventana principal");
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if msg == WM_NCDESTROY {
            let _ = detach_app(hwnd);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let Some(app_ptr) = app_from(hwnd) else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        // SAFETY: `run` mantiene el Box mientras esta ventana exista.
        let app = app_ptr.as_ref();

        match msg {
            WM_TRAY => {
                let event = lparam.0 as u32;
                if settings::is_open() {
                    settings::focus();
                    return LRESULT(0);
                }
                if event == WM_LBUTTONUP || event == WM_LBUTTONDBLCLK {
                    let recently_hidden = app
                        .hidden_at
                        .get()
                        .map(|when| when.elapsed().as_millis() < 350)
                        .unwrap_or(false);
                    if recently_hidden {
                        app.hidden_at.set(None);
                    } else {
                        show_panel(app);
                    }
                } else if event == WM_RBUTTONUP {
                    show_menu(app, hwnd);
                }
                LRESULT(0)
            }
            WM_REFRESHED => {
                on_refreshed(app, hwnd);
                if app.pending_refresh.replace(false) {
                    request_refresh(app, hwnd, "pendiente");
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == TIMER_REFRESH {
                    request_refresh(app, hwnd, "temporizador");
                }
                LRESULT(0)
            }
            WM_WINDOW_EVENT => {
                handle_window_event(app, hwnd, HWND(lparam.0 as *mut c_void), wparam.0 as u32);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                let changed = match app.memory.try_borrow_mut() {
                    Ok(mut memory) => memory.refresh_monitors(),
                    Err(_) => false,
                };
                if changed {
                    meta::log("disposición de monitores actualizada");
                    request_refresh(app, hwnd, "monitores");
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // Los hooks deben retirarse en el hilo que los instaló.
                *app.hooks.borrow_mut() = None;
                if !tray::remove(hwnd) {
                    meta::log("no se pudo quitar el icono de la bandeja");
                }
                let handle = app.tray_icon.replace(0);
                if handle != 0 {
                    let _ = DestroyIcon(HICON(handle as *mut c_void));
                }
                let _ = KillTimer(hwnd, TIMER_REFRESH);
                if let Some(panel) = app.panel.get() {
                    let _ = destroy_window(panel);
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

extern "system" fn panel_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            if !attach_app(hwnd, lparam) {
                meta::log("no se pudo asociar el estado al panel");
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if msg == WM_NCDESTROY {
            let _ = detach_app(hwnd);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let Some(app_ptr) = app_from(hwnd) else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        // SAFETY: `run` mantiene el Box mientras esta ventana exista.
        let app = app_ptr.as_ref();

        match msg {
            WM_PAINT => {
                let view = app.view.borrow().clone();
                panel::paint(hwnd, &view);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PRINTCLIENT => {
                let view = app.view.borrow().clone();
                let dc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                panel::paint_to(hwnd, dc, &view);
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if (wparam.0 as u32 & 0xFFFF) == WA_INACTIVE {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    app.hidden_at.set(Some(Instant::now()));
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wparam.0 as u32 == VK_ESCAPE.0 as u32 {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    app.hidden_at.set(Some(Instant::now()));
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// --------------------------- modo consola ---------------------------

fn dump() {
    let manager = Manager::new(Config::load());
    let rows = snapshot(&manager.stats());
    platform::console::print_line(&to_json(&rows));
}

fn to_json(rows: &[RowView]) -> String {
    if rows.is_empty() {
        return "[\n]".to_string();
    }
    let mut out = String::from("[\n");
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        out.push_str("  {");
        out.push_str(&format!("\"title\":{},", quote(&row.title)));
        out.push_str(&format!("\"detail\":{},", quote(&row.detail)));
        out.push_str(&format!("\"value\":{},", quote(&row.value)));
        let percent = row
            .percent
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string());
        out.push_str(&format!("\"percent\":{percent}"));
        out.push('}');
    }
    out.push_str("\n]");
    out
}

fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RowView {
        RowView {
            title: "Hora \"local\"\tX".to_string(),
            detail: "Detalle".to_string(),
            value: "18:05".to_string(),
            percent: Some(62),
            accent: panel::Accent::Warn,
        }
    }

    #[test]
    fn json_escapes_special_characters() {
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
        assert_eq!(quote("a\tb"), "\"a\\tb\"");
        assert_eq!(quote("ñ"), "\"ñ\"");
    }

    #[test]
    fn json_contains_expected_fields() {
        let json = to_json(&[sample()]);
        assert!(json.starts_with("[\n  {"));
        assert!(json.contains("\"title\":\"Hora \\\"local\\\"\\tX\""));
        assert!(json.contains("\"value\":\"18:05\""));
        assert!(json.contains("\"percent\":62"));
        assert!(json.ends_with("\n]"));
    }

    #[test]
    fn json_serializes_missing_percent_as_null() {
        let mut row = sample();
        row.percent = None;
        assert!(to_json(&[row]).contains("\"percent\":null"));
    }

    #[test]
    fn empty_json_is_an_empty_array() {
        assert_eq!(to_json(&[]), "[\n]");
    }
}
