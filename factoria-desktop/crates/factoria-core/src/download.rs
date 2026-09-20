//! Descarga de pesos y binarios de motor, con progreso e integridad.
//!
//! Responsabilidades:
//! - Construir el cliente HTTP respetando el **proxy y las CAs corporativas**.
//! - Reescribir la URL hacia el ***mirror* interno** cuando la política lo define.
//! - Resolver el SHA-256 esperado (anclado en el catálogo o publicado por el
//!   origen) y **verificar el fichero** antes de darlo por bueno.
//! - Emitir progreso para la interfaz.

use crate::events::{AppEvent, EventBus};
use crate::policy::PolicyDocument;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("la aplicación está en modo aislado: no se permite ninguna descarga")]
    Offline,
    #[error("error de red: {0}")]
    Network(String),
    #[error("el servidor respondió {0}")]
    Status(u16),
    #[error("error de disco: {0}")]
    Io(#[from] std::io::Error),
    #[error("la integridad del fichero no se puede verificar y la política no lo permite")]
    IntegrityUnavailable,
    #[error("el fichero descargado no coincide con el SHA-256 esperado")]
    IntegrityMismatch { expected: String, actual: String },
    #[error("configuración de red inválida: {0}")]
    Config(String),
}

/// De dónde salió la garantía de integridad, para poder mostrarlo y auditarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntegritySource {
    /// SHA-256 anclado en el catálogo corporativo.
    Pinned,
    /// SHA-256 publicado por el origen (cabecera `X-Linked-Etag` de Hugging Face).
    Origin,
    /// Nadie publicó un digest; solo se permite si la política lo autoriza.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOutcome {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub integrity: IntegritySource,
    pub elapsed_ms: u64,
}

/// Cliente HTTP configurado según la política corporativa.
///
/// Se construye una vez y se reutiliza: crear un cliente por descarga tira el
/// *pool* de conexiones y, con un proxy con inspección TLS, multiplica los
/// apretones de manos.
pub fn build_client(policy: &PolicyDocument) -> Result<reqwest::Client, DownloadError> {
    let mut builder = reqwest::Client::builder()
        .user_agent(format!(
            "{}/{}",
            crate::PRODUCT.replace(' ', "-"),
            crate::VERSION
        ))
        .connect_timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90));

    if let Some(proxy) = policy.network.proxy.as_deref().filter(|p| !p.is_empty()) {
        // `reqwest` acepta una cadena sin esquema y la trata como host, lo que
        // convertiría una política mal escrita en un proxy silenciosamente
        // distinto del que el administrador quiso. Se exige el esquema.
        if !matches!(
            proxy.split_once("://").map(|(scheme, _)| scheme),
            Some("http" | "https" | "socks5" | "socks5h")
        ) {
            return Err(DownloadError::Config(format!(
                "el proxy debe incluir el esquema (http://, https:// o socks5://): {proxy}"
            )));
        }
        let proxy = reqwest::Proxy::all(proxy)
            .map_err(|e| DownloadError::Config(format!("proxy inválido: {e}")))?;
        builder = builder.proxy(proxy);
    }

    if let Some(bundle) = policy.network.ca_bundle.as_ref() {
        let pem = std::fs::read(bundle).map_err(|e| {
            DownloadError::Config(format!(
                "no se puede leer el bundle de CAs {}: {e}",
                bundle.display()
            ))
        })?;
        // Un bundle corporativo suele traer varios certificados concatenados.
        for cert in reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|e| DownloadError::Config(format!("bundle de CAs inválido: {e}")))?
        {
            builder = builder.add_root_certificate(cert);
        }
    }

    builder
        .build()
        .map_err(|e| DownloadError::Config(e.to_string()))
}

/// Aplica el *mirror* corporativo a una URL de origen.
///
/// Con `mirror_base_url = https://artifacts.naturgy.com/ia` y
/// `mirror_path = bartowski/X-GGUF/X.gguf`, el destino pasa a ser
/// `https://artifacts.naturgy.com/ia/bartowski/X-GGUF/X.gguf`.
pub fn resolve_url(policy: &PolicyDocument, url: &str, mirror_path: Option<&str>) -> String {
    match (policy.catalog.mirror_base_url.as_deref(), mirror_path) {
        (Some(base), Some(path)) if !base.is_empty() => {
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                path.trim_start_matches('/')
            )
        }
        _ => url.to_string(),
    }
}

/// SHA-256 que el origen publica para este objeto.
///
/// Hugging Face sirve los ficheros LFS y expone su SHA-256 en `X-Linked-Etag`.
/// Un `ETag` normal es un hash MD5 o un identificador opaco, así que **no** se
/// acepta como digest: mejor declarar que no hay integridad que fingirla.
pub fn digest_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let raw = headers.get("x-linked-etag")?.to_str().ok()?;
    let cleaned = raw.trim().trim_matches('"');
    if cleaned.len() == 64 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(cleaned.to_ascii_lowercase())
    } else {
        None
    }
}

pub struct Downloader {
    client: reqwest::Client,
    policy: PolicyDocument,
    bus: EventBus,
}

impl Downloader {
    pub fn new(client: reqwest::Client, policy: PolicyDocument, bus: EventBus) -> Self {
        Self {
            client,
            policy,
            bus,
        }
    }

    /// Descarga a `dest`, verificando integridad. Escribe primero en `<dest>.part`
    /// y renombra al final, para que un corte de red nunca deje un fichero a
    /// medias que parezca un modelo válido.
    pub async fn fetch(
        &self,
        model_id: &str,
        url: &str,
        mirror_path: Option<&str>,
        pinned_sha256: Option<&str>,
        dest: &Path,
    ) -> Result<DownloadOutcome, DownloadError> {
        if self.policy.network.offline {
            return Err(DownloadError::Offline);
        }
        let started = Instant::now();
        let target = resolve_url(&self.policy, url, mirror_path);

        let response = self
            .client
            .get(&target)
            .send()
            .await
            .map_err(|e| DownloadError::Network(e.to_string()))?;
        if !response.status().is_success() {
            return Err(DownloadError::Status(response.status().as_u16()));
        }

        let total_bytes = response.content_length();
        let origin_digest = digest_from_headers(response.headers());

        let (expected, integrity) = match (pinned_sha256, origin_digest) {
            (Some(pin), _) => (Some(pin.to_ascii_lowercase()), IntegritySource::Pinned),
            (None, Some(origin)) => (Some(origin), IntegritySource::Origin),
            (None, None) => {
                if !self.policy.catalog.allow_unverified_downloads {
                    return Err(DownloadError::IntegrityUnavailable);
                }
                (None, IntegritySource::None)
            }
        };

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let part = dest.with_extension("part");
        let mut file = tokio::fs::File::create(&part).await?;
        let mut hasher = Sha256::new();
        let mut received: u64 = 0;
        let mut last_emit = Instant::now();

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| DownloadError::Network(e.to_string()))?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            received += chunk.len() as u64;
            // Un evento por chunk saturaría el canal en una descarga de 9 GB.
            if last_emit.elapsed() >= Duration::from_millis(250) {
                let secs = started.elapsed().as_secs_f64().max(0.001);
                self.bus.emit(AppEvent::DownloadProgress {
                    model_id: model_id.to_string(),
                    received_bytes: received,
                    total_bytes,
                    bytes_per_second: (received as f64 / secs) as u64,
                });
                last_emit = Instant::now();
            }
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);

        let actual = hex(&hasher.finalize());
        if let Some(expected) = &expected {
            if &actual != expected {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(DownloadError::IntegrityMismatch {
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        tokio::fs::rename(&part, dest).await?;
        let secs = started.elapsed().as_secs_f64().max(0.001);
        self.bus.emit(AppEvent::DownloadProgress {
            model_id: model_id.to_string(),
            received_bytes: received,
            total_bytes: Some(received),
            bytes_per_second: (received as f64 / secs) as u64,
        });

        Ok(DownloadOutcome {
            path: dest.to_path_buf(),
            bytes: received,
            sha256: actual,
            integrity,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::CatalogPolicy;

    #[test]
    fn a_client_builds_with_the_default_policy() {
        assert!(build_client(&PolicyDocument::default()).is_ok());
    }

    #[test]
    fn a_proxy_without_a_scheme_is_refused_instead_of_being_guessed() {
        let mut p = PolicyDocument::default();
        p.network.proxy = Some("proxy.naturgy.com:8080".into());
        let err = build_client(&p).unwrap_err().to_string();
        assert!(
            err.contains("esquema"),
            "el error debe explicar qué falta: {err}"
        );
    }

    #[test]
    fn a_well_formed_corporate_proxy_is_accepted() {
        let mut p = PolicyDocument::default();
        p.network.proxy = Some("http://proxy.naturgy.com:8080".into());
        assert!(build_client(&p).is_ok());
    }

    #[test]
    fn a_missing_ca_bundle_is_reported_with_its_path() {
        let mut p = PolicyDocument::default();
        p.network.ca_bundle = Some(PathBuf::from("/no/existe/ca.pem"));
        let err = build_client(&p).unwrap_err().to_string();
        assert!(
            err.contains("/no/existe/ca.pem"),
            "el error debe nombrar el fichero: {err}"
        );
    }

    #[test]
    fn without_a_mirror_the_original_url_is_used() {
        let p = PolicyDocument::default();
        assert_eq!(
            resolve_url(&p, "https://huggingface.co/a/b.gguf", Some("a/b.gguf")),
            "https://huggingface.co/a/b.gguf"
        );
    }

    #[test]
    fn a_mirror_rewrites_the_url() {
        let p = PolicyDocument {
            catalog: CatalogPolicy {
                mirror_base_url: Some("https://artifacts.naturgy.com/ia/".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_url(&p, "https://huggingface.co/a/b.gguf", Some("/a/b.gguf")),
            "https://artifacts.naturgy.com/ia/a/b.gguf"
        );
    }

    #[test]
    fn a_mirror_without_a_path_cannot_rewrite() {
        let p = PolicyDocument {
            catalog: CatalogPolicy {
                mirror_base_url: Some("https://artifacts.naturgy.com/ia".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_url(&p, "https://huggingface.co/a/b.gguf", None),
            "https://huggingface.co/a/b.gguf"
        );
    }

    #[test]
    fn only_a_real_sha256_header_counts_as_a_digest() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "etag",
            r#""d41d8cd98f00b204e9800998ecf8427e""#.parse().unwrap(),
        );
        assert_eq!(
            digest_from_headers(&headers),
            None,
            "un ETag MD5 no es una garantía de integridad"
        );

        let digest = "a".repeat(64);
        headers.insert("x-linked-etag", format!("\"{digest}\"").parse().unwrap());
        assert_eq!(digest_from_headers(&headers), Some(digest));
    }

    #[test]
    fn a_non_hex_linked_etag_is_rejected() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-linked-etag", "z".repeat(64).parse().unwrap());
        assert_eq!(digest_from_headers(&headers), None);
    }

    #[tokio::test]
    async fn offline_mode_refuses_before_touching_the_network() {
        let mut policy = PolicyDocument::default();
        policy.network.offline = true;
        let dl = Downloader::new(
            build_client(&PolicyDocument::default()).unwrap(),
            policy,
            EventBus::new(),
        );
        let tmp = tempfile::tempdir().unwrap();
        let err = dl
            .fetch(
                "m",
                "https://example.invalid/x.gguf",
                None,
                None,
                &tmp.path().join("x.gguf"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DownloadError::Offline));
    }
}
