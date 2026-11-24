//! Configuración persistente en el directorio de datos de la aplicación.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::meta;
use crate::platform::paths;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Ajustes generales de Window Memory.
    pub general: General,
    /// Intervalo de actualización del panel, en segundos.
    pub refresh_seconds: u64,
    /// Aplicaciones supervisadas, por ejecutable sin extensión.
    ///
    /// Si la lista está vacía se supervisan todas las aplicaciones salvo las
    /// excluidas.
    pub supervised: Vec<String>,
    /// Aplicaciones que nunca se supervisan.
    pub excluded: Vec<String>,
    /// Ajustes por aplicación, con el ejecutable sin extensión como clave.
    pub applications: BTreeMap<String, Application>,
}

/// Ajustes que afectan a todas las aplicaciones.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct General {
    /// Activa o desactiva Window Memory por completo.
    pub enabled: bool,
    /// Restaura la geometría recordada al mover una ventana de monitor.
    pub auto_restore: bool,
}

impl Default for General {
    fn default() -> Self {
        General {
            enabled: true,
            auto_restore: true,
        }
    }
}

/// Ajustes de una aplicación concreta.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Application {
    pub enabled: bool,
}

impl Default for Application {
    fn default() -> Self {
        Application { enabled: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            general: General::default(),
            refresh_seconds: 60,
            supervised: Vec::new(),
            excluded: Vec::new(),
            applications: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Intervalo efectivo, acotado a un rango razonable.
    pub fn refresh_interval_secs(&self) -> u64 {
        self.refresh_seconds.clamp(10, 3600)
    }

    /// `true` si Window Memory está activo.
    pub fn enabled(&self) -> bool {
        self.general.enabled
    }

    /// `true` si debe restaurarse la geometría al cambiar de monitor.
    pub fn auto_restore(&self) -> bool {
        self.general.auto_restore
    }

    /// `true` si la aplicación `app` debe supervisarse.
    ///
    /// Acepta el nombre del ejecutable con o sin extensión, en cualquier caja.
    /// La decisión combina [`General::enabled`], las listas `supervised` y
    /// `excluded` y la sección `[applications.<app>]`.
    pub fn supervises(&self, app: &str) -> bool {
        let app = normalize_app(app);
        if app.is_empty() || !self.general.enabled {
            return false;
        }
        if self.excluded.iter().any(|name| normalize_app(name) == app) {
            return false;
        }
        if let Some((_, settings)) = self
            .applications
            .iter()
            .find(|(name, _)| normalize_app(name) == app)
            && !settings.enabled
        {
            return false;
        }
        self.supervised.is_empty()
            || self
                .supervised
                .iter()
                .any(|name| normalize_app(name) == app)
    }

    /// Carga la configuración; si no existe, escribe la de por defecto.
    pub fn load() -> Config {
        let path = path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(config) => config,
                Err(error) => {
                    meta::log(&format!("config: {error}; se usan los valores por defecto"));
                    Config::default()
                }
            },
            Err(_) => {
                let config = Config::default();
                config.write_default(&path);
                config
            }
        }
    }

    /// Texto TOML con comentarios listo para escribir en disco.
    pub fn render(&self) -> String {
        let mut out = format!("# Configuración de {}\n\n", meta::APP_NAME);
        out.push_str("# Intervalo de actualización del panel, en segundos (10-3600).\n");
        out.push_str(&format!("refresh_seconds = {}\n\n", self.refresh_seconds));
        out.push_str(
            "# Aplicaciones supervisadas por su ejecutable (sin .exe).\n\
             # Si la lista está vacía se supervisan todas menos las excluidas.\n",
        );
        out.push_str(&format!(
            "supervised = {}\n\n",
            toml_array(&self.supervised)
        ));
        out.push_str("# Aplicaciones que nunca se supervisan.\n");
        out.push_str(&format!("excluded = {}\n\n", toml_array(&self.excluded)));
        out.push_str("[general]\n\n");
        out.push_str("# Activa o desactiva Window Memory.\n");
        out.push_str(&format!("enabled = {}\n\n", self.general.enabled));
        out.push_str("# Restaura la geometría recordada al mover una ventana a otro monitor.\n");
        out.push_str(&format!("auto_restore = {}\n", self.general.auto_restore));
        for (name, application) in &self.applications {
            out.push_str(&format!(
                "\n[applications.{}]\nenabled = {}\n",
                toml_key(name),
                application.enabled
            ));
        }
        out
    }

    /// Escribe la configuración en `config.toml`.
    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render())
    }

    fn write_default(&self, path: &PathBuf) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(path, self.render()) {
            meta::log(&format!("config: no se pudo crear: {error}"));
        }
    }
}

/// Clave de aplicación: nombre sin extensión, en minúsculas.
pub fn normalize_app(name: &str) -> String {
    let name = name.trim().to_ascii_lowercase();
    name.strip_suffix(".exe")
        .unwrap_or(&name)
        .trim()
        .to_string()
}

pub fn dir() -> PathBuf {
    paths::data_dir(meta::DATA_DIR_NAME)
}

pub fn path() -> PathBuf {
    dir().join(meta::CONFIG_FILE)
}

/// Archivo donde se guardan las geometrías recordadas.
pub fn geometry_path() -> PathBuf {
    dir().join(meta::GEOMETRY_FILE)
}

fn toml_array(values: &[String]) -> String {
    let items: Vec<String> = values
        .iter()
        .map(|value| toml_basic_string(value))
        .collect();
    format!("[{}]", items.join(", "))
}

/// Clave TOML: entre comillas solo si no es una clave desnuda válida.
fn toml_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if bare {
        key.to_string()
    } else {
        toml_basic_string(key)
    }
}

fn toml_basic_string(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_applied_for_missing_keys() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.refresh_interval_secs(), 60);
        assert!(config.enabled());
        assert!(config.auto_restore());
        assert!(config.supervises("chrome"));
    }

    #[test]
    fn values_are_clamped() {
        let low: Config = toml::from_str("refresh_seconds = 2").unwrap();
        assert_eq!(low.refresh_interval_secs(), 10);
        let high: Config = toml::from_str("refresh_seconds = 99999").unwrap();
        assert_eq!(high.refresh_interval_secs(), 3600);
    }

    #[test]
    fn rendered_config_round_trips() {
        let mut config = Config {
            refresh_seconds: 120,
            ..Config::default()
        };
        config.general.enabled = false;
        config.supervised = vec!["chrome".to_string()];
        config.excluded = vec!["explorer".to_string()];
        config
            .applications
            .insert("code".to_string(), Application { enabled: true });

        let parsed: Config = toml::from_str(&config.render()).unwrap();
        assert_eq!(parsed.refresh_interval_secs(), 120);
        assert!(!parsed.enabled());
        assert!(parsed.auto_restore());
        assert_eq!(parsed.supervised, vec!["chrome".to_string()]);
        assert_eq!(parsed.excluded, vec!["explorer".to_string()]);
        assert_eq!(parsed.applications.len(), 1);
    }

    #[test]
    fn the_documented_example_is_understood() {
        let text = "\
[general]
enabled = true
auto_restore = true

[applications.chrome]
enabled = true

[applications.code]
enabled = true

[applications.explorer]
enabled = false
";
        let config: Config = toml::from_str(text).unwrap();
        assert!(config.enabled());
        assert!(config.auto_restore());
        assert!(config.supervises("chrome"));
        assert!(config.supervises("Code"));
        assert!(!config.supervises("explorer"));
        assert!(config.supervises("notepad"));
    }

    #[test]
    fn the_supervised_list_restricts_everything_else() {
        let config: Config = toml::from_str("supervised = [\"chrome\", \"Code.exe\"]").unwrap();
        assert!(config.supervises("chrome"));
        assert!(config.supervises("code"));
        assert!(!config.supervises("notepad"));
    }

    #[test]
    fn exclusions_ignore_case_and_extension() {
        let config: Config = toml::from_str("excluded = [\"Explorer.EXE\"]").unwrap();
        assert!(!config.supervises("explorer"));
        assert!(!config.supervises("explorer.exe"));
        assert!(config.supervises("chrome"));
    }

    #[test]
    fn disabling_the_utility_or_one_application_wins() {
        let disabled: Config = toml::from_str("[general]\nenabled = false\n").unwrap();
        assert!(!disabled.supervises("chrome"));

        let per_app: Config = toml::from_str("[applications.image]\nenabled = false\n").unwrap();
        assert!(!per_app.supervises("image"));
        assert!(per_app.supervises("chrome"));
    }

    #[test]
    fn application_keys_are_normalised() {
        assert_eq!(normalize_app("  Chrome.EXE "), "chrome");
        assert_eq!(normalize_app("code"), "code");
        assert_eq!(normalize_app(""), "");
    }

    #[test]
    fn unusual_application_keys_are_quoted_in_toml() {
        let mut config = Config::default();
        config
            .applications
            .insert("my app.exe".to_string(), Application { enabled: false });
        let text = config.render();
        assert!(text.contains("[applications.\"my app.exe\"]"));
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.applications.len(), 1);
        assert!(!parsed.supervises("my app"));
    }
}
