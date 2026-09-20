//! Ajustes del usuario.
//!
//! Solo lo que el empleado puede cambiar. Cualquier campo que la política
//! corporativa bloquee se muestra en la interfaz, pero no se edita.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Modelo activo. `None` en una instalación recién hecha.
    pub active_model_id: Option<String>,
    /// Instrucciones que el empleado añade a todas sus conversaciones.
    pub system_prompt: String,
    pub temperature: f32,
    /// Arrancar el último modelo al abrir la aplicación.
    pub autostart_last_model: bool,
    /// Primera ejecución: la Home muestra la guía de puesta en marcha.
    pub onboarded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_model_id: None,
            system_prompt: String::new(),
            temperature: 0.7,
            autostart_last_model: true,
            onboarded: false,
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Self {
        std::fs::read_to_string(paths.settings_file())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        std::fs::create_dir_all(paths.root())?;
        let tmp = paths.settings_file().with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(tmp, paths.settings_file())
    }

    /// Instrucciones efectivas: las de la empresa primero, las del empleado
    /// después. El orden importa — lo corporativo enmarca, lo personal matiza.
    pub fn effective_system_prompt(&self, corporate: Option<&str>) -> Option<String> {
        let parts: Vec<&str> = [corporate.unwrap_or("").trim(), self.system_prompt.trim()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_a_fresh_install() {
        let s = Settings::default();
        assert!(s.active_model_id.is_none());
        assert!(!s.onboarded);
        assert_eq!(s.temperature, 0.7);
    }

    #[test]
    fn settings_round_trip_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        let s = Settings {
            active_model_id: Some("qwen2.5-7b-instruct-q4km".into()),
            onboarded: true,
            ..Default::default()
        };
        s.save(&paths).unwrap();
        assert_eq!(Settings::load(&paths), s);
    }

    #[test]
    fn a_missing_or_broken_file_falls_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        assert_eq!(Settings::load(&paths), Settings::default());
        std::fs::write(paths.settings_file(), "{ roto").unwrap();
        assert_eq!(Settings::load(&paths), Settings::default());
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        std::fs::write(paths.settings_file(), r#"{"temperature":0.2}"#).unwrap();
        let s = Settings::load(&paths);
        assert_eq!(s.temperature, 0.2);
        assert!(
            s.autostart_last_model,
            "el resto conserva su valor por defecto"
        );
    }

    #[test]
    fn corporate_instructions_come_before_personal_ones() {
        let s = Settings {
            system_prompt: "Responde en bullets.".into(),
            ..Default::default()
        };
        let combined = s
            .effective_system_prompt(Some("No compartas datos de clientes."))
            .unwrap();
        assert!(combined.starts_with("No compartas datos de clientes."));
        assert!(combined.ends_with("Responde en bullets."));
    }

    #[test]
    fn no_instructions_at_all_means_none() {
        assert!(Settings::default().effective_system_prompt(None).is_none());
        assert!(Settings::default()
            .effective_system_prompt(Some("   "))
            .is_none());
    }
}
