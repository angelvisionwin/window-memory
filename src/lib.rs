//! Window Memory: recuerda el tamaño y la posición de las ventanas de cada
//! aplicación en cada monitor.
//!
//! El ejecutable solo llama a [`run`]; todo lo demás es interno.

#![allow(unsafe_op_in_unsafe_fn)]

pub(crate) mod app;
pub(crate) mod cli;
pub(crate) mod config;
pub(crate) mod meta;
pub(crate) mod panel;
pub(crate) mod platform;
pub(crate) mod settings;
pub(crate) mod strings;
pub(crate) mod tray;
pub(crate) mod window_manager;
pub(crate) mod window_memory;

/// Ejecuta la aplicación.
pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        cli::Invocation::Version => cli::print_version(),
        cli::Invocation::Help => cli::print_help(),
        cli::Invocation::Run(args) => app::run(args),
    }
}
