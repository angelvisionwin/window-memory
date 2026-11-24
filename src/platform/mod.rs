//! Capa de plataforma: operaciones Win32 reutilizables.
//!
//! Esta capa no conoce la aplicación: recibe nombres, rutas y textos como
//! parámetros y no depende de módulos de más alto nivel.

pub mod console;
pub mod dialog;
pub mod dpi;
pub mod geometry;
pub mod instance;
pub mod log;
pub mod monitor;
pub mod paths;
pub mod registry;
pub mod string;
pub mod time;
pub mod window;
