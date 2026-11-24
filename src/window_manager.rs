//! Enumeración, identificación y seguimiento de ventanas.
//!
//! [`Manager`] combina lo observado por los hooks de Win32 con la memoria de
//! geometrías. La lógica de decisión es pura: recibe [`WindowFacts`] ya
//! resueltos y devuelve el [`Outcome`] que la aplicación debe ejecutar, de modo
//! que se puede probar sin tocar el escritorio real.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;

use crate::config::{self, Config};
use crate::meta;
use crate::platform::geometry::Rect;
use crate::platform::monitor::{self, Monitor, MonitorSet};
use crate::platform::window;
use crate::window_memory::{self, Geometry, GeometryStore};

/// Tiempo durante el que se descartan los eventos provocados por una restauración.
const QUIET_AFTER_RESTORE: Duration = Duration::from_millis(400);

/// Clase de los cuadros de diálogo estándar de Win32.
const DIALOG_CLASS: &str = "#32770";

/// Número máximo de nombres de ejecutable que se conservan en la caché.
const EXECUTABLE_CACHE_LIMIT: usize = 256;

/// Observación de una ventana, ya resuelta por la capa Win32.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowFacts {
    pub hwnd: isize,
    pub process_id: u32,
    pub visible: bool,
    pub top_level: bool,
    pub owned: bool,
    pub tool_window: bool,
    pub cloaked: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub class: String,
    pub rect: Rect,
    /// Identificador de sesión del monitor que contiene la ventana.
    pub monitor: Option<isize>,
}

impl WindowFacts {
    /// Observa `hwnd`; `None` si ya no existe o no tiene geometría.
    pub fn observe(hwnd: HWND) -> Option<WindowFacts> {
        if !window::is_window(hwnd) {
            return None;
        }
        let rect = window::rect(hwnd)?;
        let placement = window::placement(hwnd);
        let minimized =
            placement.map(|value| value.minimized).unwrap_or(false) || window::is_minimized(hwnd);
        let maximized =
            placement.map(|value| value.maximized).unwrap_or(false) || window::is_maximized(hwnd);
        Some(WindowFacts {
            hwnd: hwnd.0 as isize,
            process_id: window::process_id(hwnd),
            visible: window::is_visible(hwnd),
            top_level: window::is_top_level(hwnd),
            owned: window::has_owner(hwnd),
            tool_window: window::is_tool_window(hwnd),
            cloaked: window::is_cloaked(hwnd),
            minimized,
            maximized,
            class: window::class_name(hwnd).unwrap_or_default(),
            rect,
            monitor: monitor::from_window(hwnd).map(|value| value.handle),
        })
    }
}

/// Motivo por el que una ventana no se sigue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    OwnProcess,
    Invisible,
    NotTopLevel,
    Owned,
    ToolWindow,
    Cloaked,
    Dialog,
    Minimized,
    Maximized,
    InvalidGeometry,
    UnknownExecutable,
    NotSupervised,
}

impl Rejection {
    /// Texto breve para el registro.
    pub fn reason(self) -> &'static str {
        match self {
            Rejection::OwnProcess => "proceso propio",
            Rejection::Invisible => "ventana invisible",
            Rejection::NotTopLevel => "ventana hija",
            Rejection::Owned => "ventana con propietario",
            Rejection::ToolWindow => "ventana de herramienta",
            Rejection::Cloaked => "ventana oculta por DWM",
            Rejection::Dialog => "cuadro de diálogo",
            Rejection::Minimized => "ventana minimizada",
            Rejection::Maximized => "ventana maximizada",
            Rejection::InvalidGeometry => "geometría no válida",
            Rejection::UnknownExecutable => "ejecutable desconocido",
            Rejection::NotSupervised => "aplicación no supervisada",
        }
    }
}

/// Filtros que no necesitan conocer el ejecutable de la ventana.
pub fn basic_rejection(facts: &WindowFacts, own_process: u32) -> Option<Rejection> {
    if facts.process_id == 0 || facts.process_id == own_process {
        return Some(Rejection::OwnProcess);
    }
    if !facts.visible {
        return Some(Rejection::Invisible);
    }
    if !facts.top_level {
        return Some(Rejection::NotTopLevel);
    }
    if facts.owned {
        return Some(Rejection::Owned);
    }
    if facts.tool_window {
        return Some(Rejection::ToolWindow);
    }
    if facts.cloaked {
        return Some(Rejection::Cloaked);
    }
    if facts.class.eq_ignore_ascii_case(DIALOG_CLASS) {
        return Some(Rejection::Dialog);
    }
    if facts.minimized {
        return Some(Rejection::Minimized);
    }
    if facts.maximized {
        return Some(Rejection::Maximized);
    }
    if !facts.rect.is_usable() {
        return Some(Rejection::InvalidGeometry);
    }
    None
}

/// Decide si una ventana debe supervisarse.
pub fn rejection(
    facts: &WindowFacts,
    own_process: u32,
    executable: Option<&str>,
    config: &Config,
) -> Option<Rejection> {
    if let Some(rejection) = basic_rejection(facts, own_process) {
        return Some(rejection);
    }
    match executable {
        Some(name) if config.supervises(name) => None,
        Some(_) => Some(Rejection::NotSupervised),
        None => Some(Rejection::UnknownExecutable),
    }
}

/// Contadores que publica la interfaz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub enabled: bool,
    pub auto_restore: bool,
    pub tracked: usize,
    pub geometries: usize,
    pub monitors: usize,
}

/// Resultado de aplicar un evento a la memoria de ventanas.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Rectángulo absoluto al que debe moverse la ventana.
    pub place: Option<Rect>,
    /// Texto descriptivo para el registro.
    pub detail: Option<String>,
    /// Cambió algo relevante para la interfaz.
    pub changed: bool,
}

/// Estado de una ventana seguida.
#[derive(Clone, Debug)]
struct Tracked {
    app: String,
    /// Clave del monitor donde se observó por última vez en reposo.
    monitor: String,
    geometry: Geometry,
    moving: bool,
    /// Fin del periodo en el que se descartan eventos propios.
    quiet_until: Option<Instant>,
}

/// Memoria de geometrías y seguimiento de ventanas.
pub struct Manager {
    config: Config,
    monitor_set: MonitorSet,
    store: GeometryStore,
    store_path: Option<PathBuf>,
    windows: HashMap<isize, Tracked>,
    executables: HashMap<u32, Option<String>>,
    own_process: u32,
}

impl Manager {
    /// Crea el gestor con la configuración dada, los monitores conectados y el
    /// archivo de geometrías del directorio de datos.
    pub fn new(config: Config) -> Manager {
        Manager::with_store(
            config,
            MonitorSet::enumerate(),
            Some(config::geometry_path()),
        )
    }

    fn with_store(config: Config, monitor_set: MonitorSet, store_path: Option<PathBuf>) -> Manager {
        let store = match &store_path {
            Some(path) => match GeometryStore::load(path) {
                Ok(store) => store,
                Err(error) => {
                    meta::log(&format!("geometrías: no se pudieron leer: {error}"));
                    GeometryStore::new()
                }
            },
            None => GeometryStore::new(),
        };
        Manager {
            config,
            monitor_set,
            store,
            store_path,
            windows: HashMap::new(),
            executables: HashMap::new(),
            own_process: window::current_process_id(),
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            enabled: self.config.enabled(),
            auto_restore: self.config.auto_restore(),
            tracked: self.windows.len(),
            geometries: self.store.len(),
            monitors: self.monitor_set.len(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled()
    }

    pub fn auto_restore(&self) -> bool {
        self.config.auto_restore()
    }

    /// Aplica una configuración nueva; al desactivarse deja de seguir ventanas.
    pub fn apply_config(&mut self, config: Config) {
        self.config = config;
        if !self.config.enabled() {
            self.windows.clear();
        }
    }

    /// Reenumera los monitores y devuelve `true` si la disposición cambió.
    pub fn refresh_monitors(&mut self) -> bool {
        self.set_monitors(MonitorSet::enumerate())
    }

    /// Sustituye el conjunto de monitores conocidos.
    pub fn set_monitors(&mut self, monitors: MonitorSet) -> bool {
        if self.monitor_set.same_as(&monitors) {
            return false;
        }
        self.monitor_set = monitors;
        let known = &self.monitor_set;
        self.windows
            .retain(|_, tracked| known.by_key(&tracked.monitor).is_some());
        true
    }

    /// Procesa un evento del sistema para la ventana `hwnd`.
    ///
    /// `facts` es `None` cuando la ventana ya no existe; aun así hay que olvidarla.
    pub fn on_event(
        &mut self,
        event: u32,
        hwnd: isize,
        facts: Option<&WindowFacts>,
        now: Instant,
    ) -> Outcome {
        let mut outcome = Outcome::default();
        if !self.config.enabled() {
            if !self.windows.is_empty() {
                self.windows.clear();
                outcome.changed = true;
            }
            return outcome;
        }
        if matches!(event, window::EVENT_DESTROY | window::EVENT_HIDE) {
            outcome.changed = self.windows.remove(&hwnd).is_some();
            return outcome;
        }
        let Some(facts) = facts else {
            return outcome;
        };

        match event {
            window::EVENT_MOVE_START => self.begin_move(facts, &mut outcome),
            window::EVENT_MOVE_END => self.end_move(facts, now, &mut outcome),
            window::EVENT_LOCATION_CHANGE => self.location_changed(facts, now, &mut outcome),
            _ => {}
        }
        outcome
    }

    fn begin_move(&mut self, facts: &WindowFacts, outcome: &mut Outcome) {
        if self.rejection(facts).is_some() {
            outcome.changed |= self.windows.remove(&facts.hwnd).is_some();
            return;
        }
        let Some(monitor) = self.monitor_of(facts).cloned() else {
            return;
        };
        let Some(app) = self.app_key(facts) else {
            return;
        };

        // En `MOVESIZESTART` la ventana todavía está en el monitor de origen: esta
        // es la geometría que se recordará si termina el movimiento en otro.
        let tracked = Tracked {
            app,
            monitor: monitor.key(),
            geometry: window_memory::relative(facts.rect, monitor.work),
            moving: true,
            quiet_until: None,
        };
        if self.windows.insert(facts.hwnd, tracked).is_none() {
            outcome.changed = true;
        }
    }

    fn end_move(&mut self, facts: &WindowFacts, now: Instant, outcome: &mut Outcome) {
        let Some(mut tracked) = self.windows.remove(&facts.hwnd) else {
            return;
        };
        tracked.moving = false;

        if let Some(rejection) = self.rejection(facts) {
            // Al maximizar, minimizar o cerrar el cuadro de arrastre la ventana
            // deja de interesarnos: se olvida sin guardar nada.
            outcome.changed = true;
            outcome.detail = Some(format!(
                "fin de movimiento descartado: {}",
                rejection.reason()
            ));
            return;
        }
        let Some(target) = self.monitor_of(facts).cloned() else {
            return;
        };
        let target_key = target.key();
        let current = window_memory::relative(facts.rect, target.work);
        let mut dirty = false;

        if target_key == tracked.monitor {
            // Mismo monitor: se recuerda el tamaño final sin mover la ventana.
            dirty |= self.store.insert(&tracked.app, &tracked.monitor, current);
            tracked.geometry = current;
        } else {
            // Se guarda la geometría que la ventana tenía en el monitor de origen,
            // antes de sustituirla por la del destino.
            dirty |= self
                .store
                .insert(&tracked.app, &tracked.monitor, tracked.geometry);

            let desired = if self.config.auto_restore() {
                match self.store.get(&tracked.app, &target_key) {
                    Some(geometry) => Some(window_memory::clamp(geometry, target.work)),
                    None => Some(self.adapt_to(&tracked, &target)),
                }
            } else {
                None
            };

            match desired {
                Some(desired) => {
                    dirty |= self.store.insert(&tracked.app, &target_key, desired);
                    outcome.place = Some(desired.absolute(target.work));
                    outcome.detail = Some(format!(
                        "{}: {} → {} ({}×{} en {}, {})",
                        tracked.app,
                        tracked.monitor,
                        target_key,
                        desired.width,
                        desired.height,
                        desired.x,
                        desired.y
                    ));
                    tracked.monitor = target_key;
                    tracked.geometry = desired;
                }
                None => {
                    dirty |= self.store.insert(&tracked.app, &target_key, current);
                    tracked.monitor = target_key;
                    tracked.geometry = current;
                }
            }
            outcome.changed = true;
        }

        tracked.quiet_until = Some(now + QUIET_AFTER_RESTORE);
        self.windows.insert(facts.hwnd, tracked);
        if dirty {
            self.persist();
        }
    }

    fn location_changed(&mut self, facts: &WindowFacts, now: Instant, outcome: &mut Outcome) {
        if !self.windows.contains_key(&facts.hwnd) {
            return;
        }
        let Some(monitor) = self.monitor_of(facts).cloned() else {
            return;
        };
        let key = monitor.key();
        let geometry = window_memory::relative(facts.rect, monitor.work);
        let Some(tracked) = self.windows.get_mut(&facts.hwnd) else {
            return;
        };
        if tracked.moving || tracked.quiet_until.is_some_and(|until| now < until) {
            return;
        }
        if tracked.monitor == key {
            tracked.geometry = geometry;
            return;
        }
        // Movimiento ajeno a un arrastre (la aplicación se recoloca sola): se toma
        // nota del nuevo monitor, pero no se restaura para no pelearse con ella.
        tracked.monitor = key;
        tracked.geometry = geometry;
        outcome.changed = true;
    }

    /// Geometría inicial para un monitor en el que la aplicación no tiene memoria.
    fn adapt_to(&self, tracked: &Tracked, target: &Monitor) -> Geometry {
        match self.monitor_set.by_key(&tracked.monitor) {
            Some(source) => window_memory::adapt(
                tracked.geometry,
                source.work,
                source.dpi,
                target.work,
                target.dpi,
            ),
            None => window_memory::clamp(tracked.geometry, target.work),
        }
    }

    fn persist(&self) {
        let Some(path) = &self.store_path else {
            return;
        };
        if let Err(error) = self.store.save(path) {
            meta::log(&format!("geometrías: no se pudieron guardar: {error}"));
        }
    }

    fn monitor_of(&self, facts: &WindowFacts) -> Option<&Monitor> {
        self.monitor_set.by_handle(facts.monitor?)
    }

    fn app_key(&mut self, facts: &WindowFacts) -> Option<String> {
        let executable = self.executable(facts.process_id)?;
        Some(config::normalize_app(&executable))
    }

    fn executable(&mut self, pid: u32) -> Option<String> {
        if let Some(cached) = self.executables.get(&pid) {
            return cached.clone();
        }
        if self.executables.len() >= EXECUTABLE_CACHE_LIMIT {
            self.executables.clear();
        }
        let name = window::executable_name(pid);
        self.executables.insert(pid, name.clone());
        name
    }

    fn rejection(&mut self, facts: &WindowFacts) -> Option<Rejection> {
        if let Some(rejection) = basic_rejection(facts, self.own_process) {
            return Some(rejection);
        }
        let executable = self.executable(facts.process_id);
        rejection(facts, self.own_process, executable.as_deref(), &self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: isize = 1;
    const RIGHT: isize = 2;
    const CHROME_PID: u32 = 4242;

    fn monitors() -> MonitorSet {
        MonitorSet::from_list(vec![
            Monitor {
                handle: LEFT,
                device: Some("\\\\.\\DISPLAY1".to_string()),
                bounds: Rect::new(0, 0, 3840, 2160),
                work: Rect::new(0, 0, 3840, 2120),
                primary: true,
                dpi: 96,
            },
            Monitor {
                handle: RIGHT,
                device: Some("\\\\.\\DISPLAY2".to_string()),
                bounds: Rect::new(3840, 0, 6000, 3840),
                work: Rect::new(3840, 0, 6000, 3800),
                primary: false,
                dpi: 96,
            },
        ])
    }

    fn manager() -> Manager {
        let mut manager = Manager::with_store(Config::default(), monitors(), None);
        manager
            .executables
            .insert(CHROME_PID, Some("chrome.exe".to_string()));
        manager
    }

    fn facts(hwnd: isize, monitor: isize, rect: Rect) -> WindowFacts {
        WindowFacts {
            hwnd,
            process_id: CHROME_PID,
            visible: true,
            top_level: true,
            owned: false,
            tool_window: false,
            cloaked: false,
            minimized: false,
            maximized: false,
            class: "Chrome_WidgetWin_1".to_string(),
            rect,
            monitor: Some(monitor),
        }
    }

    fn left_rect() -> Rect {
        Rect::new(100, 100, 1700, 1100)
    }

    fn left_geometry() -> Geometry {
        Geometry::new(100, 100, 1600, 1000)
    }

    /// Simula un arrastre completo y devuelve el resultado del final.
    fn drag(
        manager: &mut Manager,
        start: &WindowFacts,
        end: &WindowFacts,
        now: Instant,
    ) -> Outcome {
        on_facts(manager, window::EVENT_MOVE_START, Some(start), now);
        on_facts(
            manager,
            window::EVENT_MOVE_END,
            Some(end),
            now + Duration::from_secs(1),
        )
    }

    /// Variante de `Manager::on_event` para las pruebas con hechos opcionales.
    fn on_facts(
        manager: &mut Manager,
        event: u32,
        facts: Option<&WindowFacts>,
        now: Instant,
    ) -> Outcome {
        let hwnd = facts.map(|value| value.hwnd).unwrap_or_default();
        manager.on_event(event, hwnd, facts, now)
    }
    #[test]
    fn moving_to_another_monitor_adapts_and_remembers() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        // El rectángulo de llegada no coincide con el de origen: la geometría que
        // debe recordarse es la del monitor de partida, adaptada al de destino.
        let end = facts(10, RIGHT, Rect::new(4100, 300, 5300, 1000));

        let outcome = drag(&mut manager, &start, &end, now);
        let placed = outcome.place.expect("debe colocarse en el monitor destino");
        assert_eq!(
            manager.store.get("chrome", "display1"),
            Some(left_geometry())
        );
        let remembered = manager
            .store
            .get("chrome", "display2")
            .expect("debe recordarse el monitor destino");
        assert_eq!(remembered.absolute(Rect::new(3840, 0, 6000, 3800)), placed);
        assert_eq!((remembered.width, remembered.height), (1600, 1000));
    }

    #[test]
    fn returning_to_the_original_monitor_restores_its_geometry() {
        let now = Instant::now();
        let mut manager = manager();
        let from_left = facts(10, LEFT, left_rect());
        let on_right = facts(10, RIGHT, Rect::new(4100, 300, 5700, 1300));
        let first = drag(&mut manager, &from_left, &on_right, now);
        let placed = first.place.unwrap();

        // La ventana ya colocada en el monitor vertical vuelve al principal.
        let back_start = facts(10, RIGHT, placed);
        let back_end = facts(10, LEFT, Rect::new(0, 0, 900, 700));
        let outcome = drag(
            &mut manager,
            &back_start,
            &back_end,
            now + Duration::from_secs(30),
        );
        assert_eq!(outcome.place, Some(left_rect()));
    }

    #[test]
    fn resizing_on_the_same_monitor_is_remembered_without_moving() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        let end = facts(10, LEFT, Rect::new(200, 150, 2100, 1400));

        let outcome = drag(&mut manager, &start, &end, now);
        assert_eq!(outcome.place, None);
        assert_eq!(
            manager.store.get("chrome", "display1"),
            Some(Geometry::new(200, 150, 1900, 1250))
        );
    }

    #[test]
    fn events_during_the_drag_do_not_trigger_a_restore() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        on_facts(&mut manager, window::EVENT_MOVE_START, Some(&start), now);

        let halfway = facts(10, RIGHT, Rect::new(4000, 200, 5600, 1200));
        let outcome = on_facts(
            &mut manager,
            window::EVENT_LOCATION_CHANGE,
            Some(&halfway),
            now + Duration::from_millis(100),
        );
        assert_eq!(outcome.place, None);
        assert!(!outcome.changed);

        // Al soltar, se restaura una sola vez.
        let outcome = on_facts(
            &mut manager,
            window::EVENT_MOVE_END,
            Some(&halfway),
            now + Duration::from_millis(200),
        );
        assert!(outcome.place.is_some());
    }

    #[test]
    fn restored_events_are_ignored_until_the_quiet_period_ends() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        let end = facts(10, RIGHT, Rect::new(4100, 300, 5700, 1300));
        drag(&mut manager, &start, &end, now);

        let moved = facts(10, RIGHT, Rect::new(3900, 0, 5500, 1000));
        let outcome = on_facts(
            &mut manager,
            window::EVENT_LOCATION_CHANGE,
            Some(&moved),
            now + Duration::from_secs(1) + Duration::from_millis(10),
        );
        assert_eq!(outcome.place, None);

        // Pasado el margen, un cambio de monitor se acepta sin restaurar nada.
        let elsewhere = facts(10, LEFT, Rect::new(0, 0, 900, 700));
        let outcome = on_facts(
            &mut manager,
            window::EVENT_LOCATION_CHANGE,
            Some(&elsewhere),
            now + Duration::from_secs(5),
        );
        assert_eq!(outcome.place, None);
        assert!(outcome.changed);
    }

    #[test]
    fn maximized_and_minimized_windows_are_skipped() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        on_facts(&mut manager, window::EVENT_MOVE_START, Some(&start), now);

        let mut maximized = facts(10, RIGHT, Rect::new(3840, 0, 6000, 2120));
        maximized.maximized = true;
        let outcome = on_facts(
            &mut manager,
            window::EVENT_MOVE_END,
            Some(&maximized),
            now + Duration::from_secs(1),
        );
        assert_eq!(outcome.place, None);
        assert!(manager.store.is_empty());
    }

    #[test]
    fn several_windows_of_the_same_application_share_the_remembered_geometry() {
        let now = Instant::now();
        let mut manager = manager();
        let first_start = facts(10, LEFT, left_rect());
        let first_end = facts(10, RIGHT, Rect::new(4100, 300, 5700, 1300));
        let first = drag(&mut manager, &first_start, &first_end, now);
        let expected = first.place.unwrap();

        let second_start = facts(20, LEFT, Rect::new(300, 200, 1300, 900));
        let second_end = facts(20, RIGHT, Rect::new(4000, 100, 5000, 800));
        let outcome = drag(
            &mut manager,
            &second_start,
            &second_end,
            now + Duration::from_secs(10),
        );
        assert_eq!(outcome.place, Some(expected));
    }

    #[test]
    fn dpi_differences_scale_the_adapted_geometry() {
        let now = Instant::now();
        let list = monitors().list().to_vec();
        let scaled = MonitorSet::from_list(vec![
            list[0].clone(),
            Monitor {
                dpi: 192,
                ..list[1].clone()
            },
        ]);
        let mut manager = Manager::with_store(Config::default(), scaled, None);
        manager
            .executables
            .insert(CHROME_PID, Some("chrome.exe".to_string()));

        let start = facts(10, LEFT, Rect::new(0, 0, 900, 700));
        let end = facts(10, RIGHT, Rect::new(3840, 0, 4740, 700));
        let outcome = drag(&mut manager, &start, &end, now);
        let placed = outcome.place.expect("debe adaptarse");
        assert_eq!(placed.width(), 1800);
        assert_eq!(placed.height(), 1400);
    }

    #[test]
    fn automatic_restore_can_be_disabled() {
        let now = Instant::now();
        let config = Config {
            general: config::General {
                auto_restore: false,
                ..Default::default()
            },
            ..Config::default()
        };
        let mut manager = Manager::with_store(config, monitors(), None);
        manager
            .executables
            .insert(CHROME_PID, Some("chrome.exe".to_string()));

        let start = facts(10, LEFT, left_rect());
        let end = facts(10, RIGHT, Rect::new(4100, 300, 5700, 1300));
        let outcome = drag(&mut manager, &start, &end, now);
        assert_eq!(outcome.place, None);
        assert_eq!(
            manager.store.get("chrome", "display2"),
            Some(Geometry::new(260, 300, 1600, 1000))
        );
    }

    #[test]
    fn disabled_memory_ignores_every_event() {
        let now = Instant::now();
        let config = Config {
            general: config::General {
                enabled: false,
                ..Default::default()
            },
            ..Config::default()
        };
        let mut manager = Manager::with_store(config, monitors(), None);
        manager
            .executables
            .insert(CHROME_PID, Some("chrome.exe".to_string()));

        let start = facts(10, LEFT, left_rect());
        let end = facts(10, RIGHT, Rect::new(4100, 300, 5700, 1300));
        let outcome = drag(&mut manager, &start, &end, now);
        assert_eq!(outcome, Outcome::default());
        assert!(manager.store.is_empty());
    }

    #[test]
    fn excluded_applications_are_not_tracked() {
        let now = Instant::now();
        let config = Config {
            excluded: vec!["chrome".to_string()],
            ..Config::default()
        };
        let mut manager = Manager::with_store(config, monitors(), None);
        manager
            .executables
            .insert(CHROME_PID, Some("chrome.exe".to_string()));

        let start = facts(10, LEFT, left_rect());
        on_facts(&mut manager, window::EVENT_MOVE_START, Some(&start), now);
        assert_eq!(manager.stats().tracked, 0);
    }

    #[test]
    fn filters_describe_why_a_window_is_ignored() {
        let config = Config::default();
        let base = facts(10, LEFT, left_rect());
        assert_eq!(rejection(&base, 0, Some("chrome.exe"), &config), None);

        let own = rejection(&base, CHROME_PID, Some("chrome.exe"), &config);
        assert_eq!(own, Some(Rejection::OwnProcess));

        let mut dialog = base.clone();
        dialog.class = "#32770".to_string();
        assert_eq!(
            rejection(&dialog, 0, Some("chrome.exe"), &config),
            Some(Rejection::Dialog)
        );

        let mut tool = base.clone();
        tool.tool_window = true;
        assert_eq!(
            rejection(&tool, 0, Some("chrome.exe"), &config),
            Some(Rejection::ToolWindow)
        );

        let mut child = base.clone();
        child.top_level = false;
        assert_eq!(
            rejection(&child, 0, Some("chrome.exe"), &config),
            Some(Rejection::NotTopLevel)
        );

        let mut tiny = base.clone();
        tiny.rect = Rect::new(0, 0, 20, 20);
        assert_eq!(
            rejection(&tiny, 0, Some("chrome.exe"), &config),
            Some(Rejection::InvalidGeometry)
        );

        assert_eq!(
            rejection(&base, 0, None, &config),
            Some(Rejection::UnknownExecutable)
        );

        let excluded = Config {
            excluded: vec!["chrome".to_string()],
            ..Config::default()
        };
        assert_eq!(
            rejection(&base, 0, Some("chrome.exe"), &excluded),
            Some(Rejection::NotSupervised)
        );
    }

    #[test]
    fn monitor_changes_drop_windows_on_missing_monitors() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        on_facts(&mut manager, window::EVENT_MOVE_START, Some(&start), now);
        assert_eq!(manager.stats().tracked, 1);

        let only_left = MonitorSet::from_list(vec![monitors().list()[0].clone()]);
        assert!(manager.set_monitors(only_left));
        assert_eq!(manager.stats().tracked, 1);

        // Al desaparecer el monitor donde estaba la ventana, deja de seguirse.
        let right = monitors().list()[1].clone();
        on_facts(
            &mut manager,
            window::EVENT_LOCATION_CHANGE,
            Some(&facts(11, RIGHT, Rect::new(3900, 0, 5500, 1000))),
            now,
        );
        assert_eq!(manager.stats().tracked, 1);
        assert!(manager.set_monitors(MonitorSet::from_list(vec![right])));
        assert_eq!(manager.stats().tracked, 0);
    }

    #[test]
    fn destroyed_windows_are_forgotten_even_without_facts() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        on_facts(&mut manager, window::EVENT_MOVE_START, Some(&start), now);
        assert_eq!(manager.stats().tracked, 1);

        // Al destruirse, la ventana ya no existe: el evento llega sin hechos.
        let outcome = manager.on_event(window::EVENT_DESTROY, 10, None, now);
        assert!(outcome.changed);
        assert_eq!(manager.stats().tracked, 0);

        // Un segundo aviso ya no cambia nada.
        let outcome = manager.on_event(window::EVENT_DESTROY, 10, None, now);
        assert!(!outcome.changed);
    }

    #[test]
    fn hidden_windows_stop_being_tracked() {
        let now = Instant::now();
        let mut manager = manager();
        let start = facts(10, LEFT, left_rect());
        on_facts(&mut manager, window::EVENT_MOVE_START, Some(&start), now);

        let mut hidden = facts(10, LEFT, left_rect());
        hidden.visible = false;
        let outcome = on_facts(&mut manager, window::EVENT_HIDE, Some(&hidden), now);
        assert!(outcome.changed);
        assert_eq!(manager.stats().tracked, 0);
    }
}
