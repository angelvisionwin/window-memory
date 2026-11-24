//! Identidad de la aplicación: única fuente de nombres, rutas y versión.
//!
//! Para renombrar la aplicación hay que ajustar [`APP_NAME`], [`APP_ID`],
//! [`BIN_NAME`] y los nombres derivados (clases, mutex y valor de `Run`), además
//! de los campos `name` y `description` de `Cargo.toml`.

use std::path::PathBuf;

use windows::core::{PCWSTR, w};

use crate::platform::log as platform_log;
use crate::platform::paths;

include!("../identity.rs");

/// Nombre visible de la aplicación.
pub const APP_NAME: &str = DISPLAY_NAME;

/// Identificador interno sin acentos: base de las clases, del mutex y de la carpeta de datos.
pub const APP_ID: &str = "WindowMemory";

/// Versión del paquete, definida en `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Carpeta de datos dentro de `%LOCALAPPDATA%`.
pub const DATA_DIR_NAME: &str = APP_ID;

/// Archivo de configuración dentro de la carpeta de datos.
pub const CONFIG_FILE: &str = "config.toml";

/// Archivo de geometrías recordadas, generado por la aplicación.
pub const GEOMETRY_FILE: &str = "geometries.toml";

/// Valor de `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub const RUN_VALUE: &str = "WindowMemory";

/// Ventana oculta que posee el icono de la bandeja.
pub const WINDOW_CLASS_MAIN: PCWSTR = w!("WindowMemoryTrayWnd");

/// Ventana del panel.
pub const WINDOW_CLASS_PANEL: PCWSTR = w!("WindowMemoryPanelWnd");

/// Ventana del diálogo de configuración.
pub const WINDOW_CLASS_SETTINGS: PCWSTR = w!("WindowMemorySettingsWnd");

/// Mutex de instancia única, en el espacio de nombres `Local`.
pub const MUTEX_NAME: PCWSTR = w!("Local\\WindowMemoryMutex");

/// Nombre del archivo de log, derivado del nombre del ejecutable.
pub fn log_file() -> String {
    format!("{BIN_NAME}.log")
}

/// Ruta del archivo de log, si `%LOCALAPPDATA%` está disponible.
pub fn log_path() -> Option<PathBuf> {
    paths::local_app_data_dir(DATA_DIR_NAME).map(|dir| dir.join(log_file()))
}

/// Registra un mensaje en el log de la aplicación.
pub fn log(message: &str) {
    if let Some(path) = log_path() {
        platform_log::write(&path, message);
    }
}
