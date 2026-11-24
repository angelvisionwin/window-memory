//! Almacenamiento y restauración de geometrías por aplicación y monitor.
//!
//! La geometría se guarda relativa al área de trabajo (`rcWork`) del monitor,
//! así que sobrevive a cambios de la barra de tareas y a variaciones de tamaño
//! de los monitores mientras su origen no cambie.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::platform::geometry::{MIN_SIDE, Rect};

/// Geometría de una ventana relativa al área de trabajo de un monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Estado de maximización en el momento de recordar la geometría.
    #[serde(default)]
    pub maximized: bool,
}

impl Geometry {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Geometry {
        Geometry {
            x,
            y,
            width,
            height,
            maximized: false,
        }
    }

    /// Rectángulo absoluto (píxeles de escritorio) para el área de trabajo dada.
    pub fn absolute(&self, work: Rect) -> Rect {
        Rect::new(
            work.left + self.x,
            work.top + self.y,
            work.left + self.x + self.width,
            work.top + self.y + self.height,
        )
    }

    /// `true` si la geometría tiene un tamaño que merece recordarse.
    pub fn is_usable(&self) -> bool {
        self.width >= MIN_SIDE && self.height >= MIN_SIDE
    }
}

/// Geometría de `rect` expresada respecto al área de trabajo `work`.
pub fn relative(rect: Rect, work: Rect) -> Geometry {
    Geometry::new(
        rect.left - work.left,
        rect.top - work.top,
        rect.width(),
        rect.height(),
    )
}

/// Encaja una geometría dentro del área de trabajo sin deformarla.
pub fn clamp(geometry: Geometry, work: Rect) -> Geometry {
    let width = geometry.width.min(work.width()).max(1);
    let height = geometry.height.min(work.height()).max(1);
    let x = geometry.x.clamp(0, (work.width() - width).max(0));
    let y = geometry.y.clamp(0, (work.height() - height).max(0));
    Geometry {
        x,
        y,
        width,
        height,
        maximized: geometry.maximized,
    }
}

/// Adapta una geometría de un monitor a otro.
///
/// Conserva la posición relativa del centro y el tamaño físico aparente: si los
/// DPI difieren, el tamaño se escala por su cociente. El resultado siempre cabe
/// en el área de trabajo de destino, que es el caso habitual al pasar de un
/// monitor horizontal a uno vertical (o al contrario).
pub fn adapt(
    geometry: Geometry,
    source_work: Rect,
    source_dpi: u32,
    target_work: Rect,
    target_dpi: u32,
) -> Geometry {
    let scale = if source_dpi >= MIN_DPI && target_dpi >= MIN_DPI {
        target_dpi as f64 / source_dpi as f64
    } else {
        1.0
    };
    let source_width = source_work.width().max(1) as f64;
    let source_height = source_work.height().max(1) as f64;
    let target_width = target_work.width().max(1);
    let target_height = target_work.height().max(1);

    let width = (geometry.width as f64 * scale)
        .round()
        .clamp(1.0, target_width as f64) as i32;
    let height = (geometry.height as f64 * scale)
        .round()
        .clamp(1.0, target_height as f64) as i32;

    let center_x = (geometry.x as f64 + geometry.width as f64 / 2.0) / source_width;
    let center_y = (geometry.y as f64 + geometry.height as f64 / 2.0) / source_height;
    let x = (center_x * target_width as f64 - width as f64 / 2.0).round() as i32;
    let y = (center_y * target_height as f64 - height as f64 / 2.0).round() as i32;

    clamp(
        Geometry {
            x,
            y,
            width,
            height,
            maximized: false,
        },
        target_work,
    )
}

/// DPI mínimo para considerar fiable el cociente de escala; por debajo se usa 1.
const MIN_DPI: u32 = 48;

/// Clave de una geometría guardada: aplicación y monitor.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub app: String,
    pub monitor: String,
}

/// Geometrías recordadas, indexadas por aplicación y monitor.
#[derive(Clone, Debug, Default)]
pub struct GeometryStore {
    entries: BTreeMap<Key, Geometry>,
}

impl GeometryStore {
    pub fn new() -> GeometryStore {
        GeometryStore::default()
    }

    pub fn get(&self, app: &str, monitor: &str) -> Option<Geometry> {
        self.entries.get(&key(app, monitor)).copied()
    }

    /// Guarda una geometría y devuelve `true` si el valor cambió.
    pub fn insert(&mut self, app: &str, monitor: &str, geometry: Geometry) -> bool {
        if app.trim().is_empty() || monitor.trim().is_empty() || !geometry.is_usable() {
            return false;
        }
        let key = key(app, monitor);
        if self.entries.get(&key) == Some(&geometry) {
            return false;
        }
        self.entries.insert(key, geometry);
        true
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` si todavía no se ha recordado ninguna geometría.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Texto TOML del archivo de geometrías.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# Geometrías recordadas por Window Memory, por aplicación y monitor.\n\
             # Archivo generado automáticamente: se reescribe al recordar una ventana.\n\
             version = 1\n",
        );
        for (key, geometry) in &self.entries {
            out.push_str("\n[[geometry]]\n");
            out.push_str(&format!("app = {}\n", toml_string(&key.app)));
            out.push_str(&format!("monitor = {}\n", toml_string(&key.monitor)));
            out.push_str(&format!("x = {}\n", geometry.x));
            out.push_str(&format!("y = {}\n", geometry.y));
            out.push_str(&format!("width = {}\n", geometry.width));
            out.push_str(&format!("height = {}\n", geometry.height));
            out.push_str(&format!("maximized = {}\n", geometry.maximized));
        }
        out
    }

    /// Interpreta el archivo de geometrías; las entradas inválidas se descartan.
    pub fn parse(text: &str) -> GeometryStore {
        let Ok(document) = toml::from_str::<Document>(text) else {
            return GeometryStore::default();
        };
        let mut store = GeometryStore::default();
        for entry in document.geometry {
            store.insert(
                &entry.app,
                &entry.monitor,
                Geometry {
                    x: entry.x,
                    y: entry.y,
                    width: entry.width,
                    height: entry.height,
                    maximized: entry.maximized,
                },
            );
        }
        store
    }

    /// Lee el archivo de geometrías. Un archivo ausente equivale a no tener datos.
    pub fn load(path: &Path) -> std::io::Result<GeometryStore> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(GeometryStore::parse(&text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(GeometryStore::new()),
            Err(error) => Err(error),
        }
    }

    /// Escribe el archivo de geometrías, creando el directorio si hace falta.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render())
    }
}

fn key(app: &str, monitor: &str) -> Key {
    Key {
        app: app.to_string(),
        monitor: monitor.to_string(),
    }
}

/// Escapa un texto como cadena TOML básica.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[derive(Serialize, Deserialize)]
struct Document {
    #[serde(default = "document_version")]
    version: u32,
    #[serde(default)]
    geometry: Vec<StoredEntry>,
}

fn document_version() -> u32 {
    1
}

#[derive(Serialize, Deserialize)]
struct StoredEntry {
    app: String,
    monitor: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    #[serde(default)]
    maximized: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD_WORK: Rect = Rect::new(0, 0, 3840, 2120);
    const VERTICAL_WORK: Rect = Rect::new(3840, 0, 6000, 3800);

    #[test]
    fn absolute_and_relative_are_inverse() {
        let work = Rect::new(3840, 0, 6000, 3800);
        let rect = Rect::new(4000, 200, 5600, 1400);
        let geometry = relative(rect, work);
        assert_eq!(geometry, Geometry::new(160, 200, 1600, 1200));
        assert_eq!(geometry.absolute(work), rect);
    }

    #[test]
    fn relative_coordinates_ignore_negative_desktop_origins() {
        let work = Rect::new(-1920, -200, 0, 880);
        let rect = Rect::new(-1900, -150, -400, 830);
        let geometry = relative(rect, work);
        assert_eq!(geometry, Geometry::new(20, 50, 1500, 980));
        assert_eq!(geometry.absolute(work), rect);
    }

    #[test]
    fn clamp_keeps_the_window_inside_the_work_area() {
        let work = Rect::new(0, 0, 1920, 1040);
        let geometry = Geometry::new(-100, 900, 1200, 800);
        let clamped = clamp(geometry, work);
        assert_eq!(clamped.x, 0);
        assert_eq!(clamped.y, 240);
        assert_eq!(clamped.width, 1200);
        assert_eq!(clamped.height, 800);
    }

    #[test]
    fn adapt_keeps_the_size_when_dpi_matches() {
        let geometry = Geometry::new(120, 90, 1600, 1000);
        let adapted = adapt(geometry, HD_WORK, 96, VERTICAL_WORK, 96);
        assert_eq!((adapted.width, adapted.height), (1600, 1000));
        assert!(adapted.absolute(VERTICAL_WORK).left >= VERTICAL_WORK.left);
        assert!(adapted.absolute(VERTICAL_WORK).right <= VERTICAL_WORK.right);
        assert!(adapted.absolute(VERTICAL_WORK).bottom <= VERTICAL_WORK.bottom);
    }

    #[test]
    fn adapt_scales_by_dpi_and_clamps_to_the_target() {
        let geometry = Geometry::new(100, 100, 900, 700);
        let doubled = adapt(geometry, HD_WORK, 96, VERTICAL_WORK, 192);
        assert_eq!((doubled.width, doubled.height), (1800, 1400));

        let too_wide = adapt(geometry, HD_WORK, 96, Rect::new(0, 0, 800, 600), 192);
        assert_eq!((too_wide.width, too_wide.height), (800, 600));
    }

    #[test]
    fn adapt_maps_the_position_proportionally() {
        let top_left = Geometry::new(0, 0, 960, 1060);
        let adapted = adapt(top_left, HD_WORK, 96, VERTICAL_WORK, 96);
        assert_eq!(adapted.x, 0);
        assert!(adapted.y < VERTICAL_WORK.height() / 2);

        let bottom_right = Geometry::new(2880, 1060, 960, 1060);
        let adapted = adapt(bottom_right, HD_WORK, 96, VERTICAL_WORK, 96);
        assert!(adapted.x > VERTICAL_WORK.width() / 2);
        assert!(adapted.y > VERTICAL_WORK.height() / 2);
    }

    #[test]
    fn store_round_trips_through_toml() {
        let mut store = GeometryStore::new();
        assert!(store.insert("chrome", "display1", Geometry::new(100, 80, 1600, 1000)));
        assert!(store.insert("code", "display2", Geometry::new(0, 0, 1080, 1900)));
        let text = store.render();
        let parsed = GeometryStore::parse(&text);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed.get("chrome", "display1"),
            Some(Geometry::new(100, 80, 1600, 1000))
        );
        assert_eq!(
            parsed.get("code", "display2"),
            Some(Geometry::new(0, 0, 1080, 1900))
        );
    }

    #[test]
    fn insert_reports_only_real_changes() {
        let mut store = GeometryStore::new();
        let geometry = Geometry::new(10, 10, 800, 600);
        assert!(store.insert("code", "display1", geometry));
        assert!(!store.insert("code", "display1", geometry));
        assert!(store.insert("code", "display1", Geometry::new(20, 10, 800, 600)));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn invalid_entries_are_rejected() {
        let mut store = GeometryStore::new();
        assert!(!store.insert("", "display1", Geometry::new(0, 0, 800, 600)));
        assert!(!store.insert("code", " ", Geometry::new(0, 0, 800, 600)));
        assert!(!store.insert("code", "display1", Geometry::new(0, 0, 0, 600)));
        assert!(!store.insert("code", "display1", Geometry::new(0, 0, 800, 10)));
        assert!(store.is_empty());
    }

    #[test]
    fn corrupt_or_partial_files_degrade_gracefully() {
        assert!(GeometryStore::parse("esto no es toml").is_empty());
        let partial = GeometryStore::parse(
            "version = 1\n\n[[geometry]]\napp = \"code\"\nmonitor = \"display1\"\nx = 0\ny = 0\nwidth = 10\nheight = 10\n",
        );
        assert!(
            partial.is_empty(),
            "una geometría diminuta no debe cargarse"
        );

        let extra = GeometryStore::parse(
            "version = 1\n\n[[geometry]]\napp = \"code\"\nmonitor = \"display1\"\nx = 0\ny = 0\nwidth = 800\nheight = 600\nfield = 1\n",
        );
        assert_eq!(extra.len(), 1);
    }

    #[test]
    fn saved_files_round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("window-memory-store-{}", std::process::id()));
        let path = dir.join("geometries.toml");
        let mut store = GeometryStore::new();
        store.insert("chrome", "display2", Geometry::new(10, 20, 900, 700));
        store.save(&path).unwrap();

        let loaded = GeometryStore::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded.get("chrome", "display2"),
            Some(Geometry::new(10, 20, 900, 700))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let path = std::env::temp_dir().join("window-memory-store-inexistente.toml");
        let store = GeometryStore::load(&path).unwrap();
        assert!(store.is_empty());
    }
}
