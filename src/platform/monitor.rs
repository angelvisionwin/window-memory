//! Enumeración e identificación de monitores.
//!
//! El identificador de sesión (`HMONITOR`) sirve para comparar monitores dentro
//! de una misma ejecución. Para persistir geometrías se usa [`Monitor::key`],
//! basada en el nombre de dispositivo y, solo como último recurso, en la
//! geometría del monitor.

use std::mem::size_of;

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

use super::geometry::Rect;

/// Prefijo con el que Windows presenta los nombres de dispositivo de pantalla.
const DEVICE_PREFIX: &str = "\\\\.\\";

/// Monitor conectado, con la geometría y el área de trabajo actuales.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// Identificador de Win32; solo es válido mientras el monitor no cambie.
    pub handle: isize,
    /// Nombre de dispositivo (`\\.\DISPLAY1`), si Windows lo expone.
    pub device: Option<String>,
    /// Rectángulo completo del monitor, en píxeles físicos.
    pub bounds: Rect,
    /// Área de trabajo (sin la barra de tareas), en píxeles físicos.
    pub work: Rect,
    pub primary: bool,
    /// DPI efectivo del monitor; 96 cuando Windows no lo informa.
    pub dpi: u32,
}

impl Monitor {
    /// Clave estable para guardar geometrías de este monitor.
    ///
    /// Prioriza el nombre de dispositivo porque sobrevive a cambios de
    /// resolución y a reinicios. Si Windows no lo expone (por ejemplo, en
    /// sesiones remotas antiguas), se usa la geometría como aproximación.
    pub fn key(&self) -> String {
        match self.device.as_deref() {
            Some(device) if !device.trim().is_empty() => normalize_device(device),
            _ => format!(
                "rect:{},{},{},{}",
                self.bounds.left, self.bounds.top, self.bounds.right, self.bounds.bottom
            ),
        }
    }
}

/// Normaliza `\\.\DISPLAY1` a `display1`.
fn normalize_device(device: &str) -> String {
    let trimmed = device.trim();
    let trimmed = trimmed.strip_prefix(DEVICE_PREFIX).unwrap_or(trimmed);
    trimmed.to_ascii_lowercase()
}

/// Conjunto de monitores conectados en un instante.
#[derive(Clone, Debug, Default)]
pub struct MonitorSet {
    monitors: Vec<Monitor>,
}

impl MonitorSet {
    /// Enumera los monitores conectados.
    pub fn enumerate() -> MonitorSet {
        unsafe extern "system" fn collect(
            handle: HMONITOR,
            _dc: HDC,
            _rect: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            let monitors = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
            if let Some(monitor) = unsafe { read(handle) } {
                monitors.push(monitor);
            }
            BOOL(1)
        }

        let mut monitors: Vec<Monitor> = Vec::new();
        let collected = unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut monitors as *mut Vec<Monitor> as isize),
            )
        };
        if !collected.as_bool() {
            monitors.clear();
        }
        MonitorSet { monitors }
    }

    /// Construye el conjunto a partir de monitores ya resueltos (pruebas).
    #[cfg(test)]
    pub fn from_list(monitors: Vec<Monitor>) -> MonitorSet {
        MonitorSet { monitors }
    }

    #[cfg(test)]
    pub fn list(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn len(&self) -> usize {
        self.monitors.len()
    }

    pub fn by_handle(&self, handle: isize) -> Option<&Monitor> {
        self.monitors
            .iter()
            .find(|monitor| monitor.handle == handle)
    }

    pub fn by_key(&self, key: &str) -> Option<&Monitor> {
        self.monitors.iter().find(|monitor| monitor.key() == key)
    }

    /// `true` si ambos conjuntos describen los mismos monitores y geometrías.
    pub fn same_as(&self, other: &MonitorSet) -> bool {
        self.monitors == other.monitors
    }
}

/// Monitor que contiene la ventana, o el más cercano.
pub fn from_window(hwnd: HWND) -> Option<Monitor> {
    if hwnd.0.is_null() {
        return None;
    }
    unsafe { read(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)) }
}

unsafe fn read(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        szDevice: [0u16; 32],
    };
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
        return None;
    }
    let base = info.monitorInfo;
    let end = info
        .szDevice
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(info.szDevice.len());
    let device = String::from_utf16_lossy(&info.szDevice[..end]);
    let device = (!device.trim().is_empty()).then_some(device);

    Some(Monitor {
        handle: handle.0 as isize,
        device,
        bounds: Rect::from_rect(base.rcMonitor),
        work: Rect::from_rect(base.rcWork),
        primary: base.dwFlags & MONITORINFOF_PRIMARY != 0,
        dpi: unsafe { dpi_of(handle) },
    })
}

unsafe fn dpi_of(handle: HMONITOR) -> u32 {
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    match unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
        Ok(()) if dpi_x > 0 => dpi_x,
        _ => 96,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(handle: isize, device: Option<&str>, bounds: Rect, work: Rect) -> Monitor {
        Monitor {
            handle,
            device: device.map(str::to_string),
            bounds,
            work,
            primary: false,
            dpi: 96,
        }
    }

    #[test]
    fn device_names_normalise_to_a_stable_key() {
        let left = monitor(
            1,
            Some("\\\\.\\DISPLAY1"),
            Rect::new(0, 0, 3840, 2160),
            Rect::new(0, 0, 3840, 2120),
        );
        assert_eq!(left.key(), "display1");
    }

    #[test]
    fn key_survives_resolution_changes_when_the_device_is_known() {
        let before = monitor(
            1,
            Some("\\\\.\\DISPLAY2"),
            Rect::new(0, 0, 1080, 1920),
            Rect::new(0, 0, 1080, 1880),
        );
        let after = monitor(
            1,
            Some("\\\\.\\DISPLAY2"),
            Rect::new(0, 0, 2160, 3840),
            Rect::new(0, 0, 2160, 3800),
        );
        assert_eq!(before.key(), after.key());
    }

    #[test]
    fn without_a_device_name_the_geometry_is_used() {
        let first = monitor(
            1,
            None,
            Rect::new(0, 0, 1920, 1080),
            Rect::new(0, 0, 1920, 1040),
        );
        let second = monitor(
            2,
            None,
            Rect::new(1920, 0, 3000, 1080),
            Rect::new(1920, 0, 3000, 1040),
        );
        assert_eq!(first.key(), "rect:0,0,1920,1080");
        assert_ne!(first.key(), second.key());
    }

    #[test]
    fn lookups_use_handle_and_key() {
        let set = MonitorSet::from_list(vec![
            monitor(
                1,
                Some("\\\\.\\DISPLAY1"),
                Rect::new(0, 0, 1920, 1080),
                Rect::new(0, 0, 1920, 1040),
            ),
            monitor(
                2,
                Some("\\\\.\\DISPLAY2"),
                Rect::new(1920, 0, 3000, 1080),
                Rect::new(1920, 0, 3000, 1040),
            ),
        ]);
        assert_eq!(set.len(), 2);
        assert_eq!(
            set.by_handle(2).map(Monitor::key),
            Some("display2".to_string())
        );
        assert!(set.by_key("display1").is_some());
        assert!(set.by_key("display3").is_none());
        assert!(set.by_handle(9).is_none());
    }

    #[test]
    fn changes_in_layout_are_detected() {
        let before = MonitorSet::from_list(vec![monitor(
            1,
            Some("\\\\.\\DISPLAY1"),
            Rect::new(0, 0, 1920, 1080),
            Rect::new(0, 0, 1920, 1040),
        )]);
        let same = before.clone();
        let moved = MonitorSet::from_list(vec![monitor(
            1,
            Some("\\\\.\\DISPLAY1"),
            Rect::new(0, 0, 1920, 1200),
            Rect::new(0, 0, 1920, 1160),
        )]);
        assert!(before.same_as(&same));
        assert!(!before.same_as(&moved));
    }
}
