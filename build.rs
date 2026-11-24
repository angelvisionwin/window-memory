include!("identity.rs");

fn main() {
    println!("cargo:rerun-if-changed=identity.rs");
    println!("cargo:rerun-if-changed=assets/app.ico");

    #[cfg(target_os = "windows")]
    {
        // La identidad se toma de `Cargo.toml` y de `identity.rs`: el build
        // script no depende del crate para no duplicar esos datos.
        let description =
            std::env::var("CARGO_PKG_DESCRIPTION").unwrap_or_else(|_| DISPLAY_NAME.to_string());
        let license = std::env::var("CARGO_PKG_LICENSE").unwrap_or_default();

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/app.ico");
        resource.set("FileDescription", &description);
        resource.set("ProductName", DISPLAY_NAME);
        resource.set("LegalCopyright", &license);
        resource.set("OriginalFilename", &format!("{BIN_NAME}.exe"));
        if let Err(error) = resource.compile() {
            println!("cargo:warning=no se pudo incrustar el icono: {error}");
        }
    }
}
