//! Textos de la interfaz. Idioma único (español).

pub const UPDATED: &str = "Actualizado";

pub const EMPTY_TITLE: &str = "Sin actividad";
pub const EMPTY_HINT: &str =
    "Mueve una ventana entre monitores para que Window Memory la recuerde.";
pub const FOOTER: &str = "Clic derecho en el icono para más opciones";

pub const MENU_SHOW: &str = "Mostrar estado";
pub const MENU_REFRESH: &str = "Actualizar ahora";
pub const MENU_TEST_NOTIFICATION: &str = "Probar notificación";
pub const MENU_MEMORY: &str = "Window Memory activo";
pub const MENU_AUTO_RESTORE: &str = "Restauración automática";
pub const MENU_AUTOSTART: &str = "Iniciar con Windows";
pub const MENU_CONFIG: &str = "Abrir configuración";
pub const MENU_EXIT: &str = "Salir";

pub const NOTIFY_TITLE: &str = "Aviso";
pub const NOTIFY_TEST_BODY: &str = "Notificación de prueba de la bandeja.";

pub const SETTINGS_TITLE: &str = "Configuración";
pub const SETTINGS_MEMORY: &str = "Activar Window Memory";
pub const SETTINGS_AUTO_RESTORE: &str = "Restaurar al cambiar de monitor";
pub const SETTINGS_REFRESH: &str = "Intervalo de actualización (s)";
pub const SETTINGS_SAVE: &str = "Guardar";
pub const SETTINGS_CANCEL: &str = "Cancelar";
pub const SETTINGS_OPEN_FOLDER: &str = "Abrir carpeta";
pub const SETTINGS_INVALID_REFRESH: &str = "El intervalo debe estar entre 10 y 3600 segundos.";
pub const SETTINGS_SAVE_FAILED: &str = "No se pudo guardar la configuración: {error}";

pub const ROW_MEMORY: &str = "Window Memory";
pub const ROW_MEMORY_DETAIL: &str = "Geometrías por aplicación y monitor";
pub const ROW_AUTO_RESTORE: &str = "Restauración automática";
pub const ROW_AUTO_RESTORE_DETAIL: &str = "Al mover una ventana a otro monitor";
pub const ROW_WINDOWS: &str = "Ventanas seguidas";
pub const ROW_WINDOWS_DETAIL: &str = "Se actualiza con los eventos del sistema";
pub const ROW_GEOMETRIES: &str = "Geometrías guardadas";
pub const ROW_GEOMETRIES_DETAIL: &str = "En todas las aplicaciones y monitores";
pub const ROW_MONITORS: &str = "Monitores detectados";
pub const ROW_MONITORS_DETAIL: &str = "Se revisan al cambiar la disposición";

pub const STATE_ACTIVE: &str = "Activo";
pub const STATE_PAUSED: &str = "Pausado";
pub const VALUE_YES: &str = "Sí";
pub const VALUE_NO: &str = "No";

/// Mensaje de error al guardar la configuración.
pub fn save_failed(error: &str) -> String {
    SETTINGS_SAVE_FAILED.replace("{error}", error)
}
