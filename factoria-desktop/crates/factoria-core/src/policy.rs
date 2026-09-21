//! Política corporativa.
//!
//! Precedencia (de mayor a menor): política gestionada por la empresa →
//! `FACTORIA_POLICY_FILE` → ajustes del usuario → valores por defecto del
//! producto. Cada bloque puede marcarse `locked`, y entonces la interfaz lo
//! muestra como **Gestionado por Naturgy** y no lo deja editar.
//!
//! Esto es lo que permite un despliegue masivo por GPO/Intune/Jamf **sin**
//! ningún servicio de configuración en la nube.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum UpdateChannel {
    /// Sin comprobación de actualizaciones. Lo esperable en un parque gestionado.
    Off,
    /// Comprueba y avisa; instala el usuario.
    #[default]
    Manual,
    /// Comprueba e instala.
    Auto,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkPolicy {
    /// Proxy corporativo explícito. Si está vacío se usan `HTTPS_PROXY`/`HTTP_PROXY`.
    pub proxy: Option<String>,
    /// Bundle de CAs corporativas en PEM, para redes con inspección TLS.
    pub ca_bundle: Option<PathBuf>,
    /// Modo aislado: ninguna salida de red. Solo modelos ya presentes.
    pub offline: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogPolicy {
    /// `embedded` (por defecto), una ruta de fichero, o una URL https.
    pub source: Option<String>,
    /// Mirror interno que sustituye a Hugging Face / GitHub Releases.
    pub mirror_base_url: Option<String>,
    pub allowlist: Vec<String>,
    pub denylist: Vec<String>,
    /// Permitir instalar pesos cuyo SHA-256 no se puede verificar contra el
    /// origen. **Falso** por defecto: en un parque corporativo, un fichero de
    /// varios GB sin integridad comprobable no se instala.
    pub allow_unverified_downloads: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct TelemetryPolicy {
    /// **Apagada por defecto.** Activarla es una decisión de Naturgy.
    pub enabled: bool,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AuditPolicy {
    pub enabled: bool,
    pub retention_months: u32,
}

impl Default for AuditPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_months: 12,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdatePolicy {
    pub channel: UpdateChannel,
    pub feed_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatPolicy {
    /// Instrucciones corporativas añadidas a todas las conversaciones.
    pub system_prompt: Option<String>,
}

/// El documento tal y como lo escribe el administrador.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PolicyDocument {
    /// Nombre de la organización, mostrado en Ajustes.
    pub organization: Option<String>,
    /// Claves cuyo valor no puede cambiar el usuario, p. ej.
    /// `["network.proxy", "catalog.denylist"]`.
    pub locked: Vec<String>,
    pub network: NetworkPolicy,
    pub catalog: CatalogPolicy,
    pub telemetry: TelemetryPolicy,
    pub audit: AuditPolicy,
    pub updates: UpdatePolicy,
    pub chat: ChatPolicy,
}

impl PolicyDocument {
    pub fn is_locked(&self, key: &str) -> bool {
        self.locked.iter().any(|k| k == key || k == "*")
    }
}

/// La política efectiva más su procedencia, que es lo que la interfaz muestra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectivePolicy {
    #[serde(flatten)]
    pub document: PolicyDocument,
    /// `defaults` | `managed:<ruta>` | `env:<ruta>` | `user:<ruta>`
    pub origin: String,
    pub managed: bool,
}

impl Default for EffectivePolicy {
    fn default() -> Self {
        Self {
            document: PolicyDocument::default(),
            origin: "defaults".into(),
            managed: false,
        }
    }
}

impl EffectivePolicy {
    /// Resuelve la política siguiendo la precedencia documentada.
    pub fn load(user_policy_file: &Path) -> Self {
        for (path, origin, managed) in candidate_sources(user_policy_file) {
            if !path.is_file() {
                continue;
            }
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<PolicyDocument>(&raw).ok())
            {
                Some(document) => {
                    return Self {
                        document,
                        origin: format!("{origin}:{}", path.display()),
                        managed,
                    }
                }
                None => {
                    tracing::warn!(path = %path.display(), "política ilegible; se ignora");
                }
            }
        }
        Self::default()
    }

    pub fn organization_label(&self) -> &str {
        self.document.organization.as_deref().unwrap_or("Naturgy")
    }
}

/// Orígenes en orden de precedencia. Las rutas gestionadas son las que un
/// despliegue por GPO/Intune/Jamf deja en el equipo.
fn candidate_sources(user_policy_file: &Path) -> Vec<(PathBuf, &'static str, bool)> {
    let mut out: Vec<(PathBuf, &'static str, bool)> = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let program_data =
            std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_string());
        out.push((
            PathBuf::from(program_data)
                .join("Naturgy")
                .join("FactorIA")
                .join("policy.json"),
            "managed",
            true,
        ));
    }
    #[cfg(target_os = "macos")]
    {
        out.push((
            PathBuf::from("/Library/Application Support/Naturgy/FactorIA/policy.json"),
            "managed",
            true,
        ));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        out.push((
            PathBuf::from("/etc/naturgy/factoria/policy.json"),
            "managed",
            true,
        ));
    }

    if let Some(env_path) = std::env::var_os("FACTORIA_POLICY_FILE") {
        out.push((PathBuf::from(env_path), "env", true));
    }
    out.push((user_policy_file.to_path_buf(), "user", false));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_privacy_conscious_ones() {
        let p = PolicyDocument::default();
        assert!(!p.telemetry.enabled, "la telemetría nace apagada");
        assert!(p.audit.enabled, "la auditoría local nace encendida");
        assert!(!p.network.offline);
        assert!(
            !p.catalog.allow_unverified_downloads,
            "sin integridad comprobable no se instala"
        );
        assert_eq!(p.updates.channel, UpdateChannel::Manual);
        assert!(p.catalog.allowlist.is_empty() && p.catalog.denylist.is_empty());
    }

    #[test]
    fn no_policy_file_means_defaults_and_unmanaged() {
        let tmp = tempfile::tempdir().unwrap();
        let eff = EffectivePolicy::load(&tmp.path().join("policy.json"));
        assert_eq!(eff.origin, "defaults");
        assert!(!eff.managed);
    }

    #[test]
    fn a_user_policy_file_is_read_and_marked_unmanaged() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("policy.json");
        std::fs::write(
            &file,
            r#"{"organization":"Naturgy IT","catalog":{"denylist":["gemma*"]}}"#,
        )
        .unwrap();
        let eff = EffectivePolicy::load(&file);
        assert_eq!(eff.organization_label(), "Naturgy IT");
        assert_eq!(eff.document.catalog.denylist, vec!["gemma*".to_string()]);
        assert!(!eff.managed);
        assert!(eff.origin.starts_with("user:"));
    }

    #[test]
    fn a_broken_policy_file_falls_through_instead_of_crashing() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("policy.json");
        std::fs::write(&file, "{ no es json").unwrap();
        assert_eq!(EffectivePolicy::load(&file).origin, "defaults");
    }

    #[test]
    fn partial_policy_keeps_the_defaults_for_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("policy.json");
        std::fs::write(&file, r#"{"network":{"offline":true}}"#).unwrap();
        let eff = EffectivePolicy::load(&file);
        assert!(eff.document.network.offline);
        assert!(!eff.document.telemetry.enabled);
        assert!(eff.document.audit.enabled);
    }

    #[test]
    fn locked_keys_are_reported() {
        let doc = PolicyDocument {
            locked: vec!["network.proxy".into()],
            ..Default::default()
        };
        assert!(doc.is_locked("network.proxy"));
        assert!(!doc.is_locked("chat.systemPrompt"));
    }

    #[test]
    fn a_wildcard_locks_everything() {
        let doc = PolicyDocument {
            locked: vec!["*".into()],
            ..Default::default()
        };
        assert!(doc.is_locked("cualquier.cosa"));
    }

    #[test]
    fn policy_round_trips_through_json() {
        let doc = PolicyDocument {
            organization: Some("Naturgy".into()),
            locked: vec!["network.proxy".into()],
            network: NetworkPolicy {
                proxy: Some("http://proxy.naturgy.com:8080".into()),
                ca_bundle: Some(PathBuf::from("/etc/ssl/naturgy.pem")),
                offline: false,
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&doc).unwrap();
        assert_eq!(serde_json::from_str::<PolicyDocument>(&json).unwrap(), doc);
    }
}
