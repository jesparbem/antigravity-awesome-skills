//! Catálogo de modelos autorizado y clasificación de aptitud por equipo.
//!
//! El catálogo embebido (`data.rs`) es el que se distribuye con la aplicación.
//! Una política corporativa puede sustituirlo por otro fichero o filtrarlo con
//! listas de permitidos/prohibidos (`policy::PolicyDocument`).

pub mod data;
pub mod fit;

pub use fit::{FitLevel, FitReason, FitVerdict, ModelFit};

use serde::{Deserialize, Serialize};

/// Dónde se obtienen los pesos de un modelo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ModelSource {
    /// Un único fichero GGUF descargable por HTTPS.
    ///
    /// `sha256` es un *pin* opcional. Cuando está, el fichero descargado debe
    /// coincidir con él. Cuando no está, la integridad se resuelve contra el
    /// propio origen (Hugging Face publica el SHA-256 del objeto LFS en la
    /// cabecera `X-Linked-Etag`); si el origen tampoco lo publica, la descarga
    /// se rechaza salvo que la política lo permita explícitamente.
    #[serde(rename_all = "camelCase")]
    GgufUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        /// Ruta relativa dentro de un *mirror* corporativo, si lo hay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mirror_path: Option<String>,
    },
    /// Etiqueta de la librería de Ollama (`ollama pull <tag>`).
    #[serde(rename_all = "camelCase")]
    OllamaTag { tag: String },
}

/// Una fila del catálogo: todo lo que la tarjeta necesita mostrar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub family: String,
    pub provider: String,
    /// Parámetros en miles de millones. `None` para mezclas de expertos donde la
    /// cifra no es comparable.
    pub params_b: Option<f32>,
    pub quantization: String,
    pub file_bytes: u64,
    pub context_window: u32,
    pub license: String,
    pub license_url: Option<String>,
    pub released: String,
    pub blurb: String,
    /// Runtimes que pueden ejecutar este modelo, en orden de preferencia.
    pub runtimes: Vec<String>,
    pub source: ModelSource,
    /// Orden de capacidad, mayor es mejor. Se fija en la máquina del mantenedor,
    /// nunca en tiempo de ejecución. Criterio heredado de Rebost.
    pub capability: u16,
    /// Geometría del modelo, para estimar la KV cache sin descargar nada.
    pub kv: KvGeometry,
}

/// Lo justo de la arquitectura del modelo para dimensionar la KV cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KvGeometry {
    pub layers: u32,
    /// Cabezas de *key/value* (GQA): lo que realmente ocupa la caché.
    pub kv_heads: u32,
    pub head_dim: u32,
}

impl KvGeometry {
    /// Bytes de KV cache por token con caché cuantizada a `q8_0` (1 byte por
    /// elemento), que es lo que FactorIA usa por defecto:
    ///
    /// `2 (K y V) × capas × cabezas_kv × dim_cabeza × 1 byte`
    pub fn bytes_per_token(&self) -> u64 {
        2 * self.layers as u64 * self.kv_heads as u64 * self.head_dim as u64
    }

    pub fn kv_cache_bytes(&self, context_tokens: u32) -> u64 {
        self.bytes_per_token() * context_tokens as u64
    }
}

impl ModelSpec {
    /// Etiqueta de tamaño para la tarjeta ("7B", "MoE").
    pub fn size_label(&self) -> String {
        match self.params_b {
            Some(p) if p >= 1.0 => format!("{}B", trim_float(p)),
            Some(p) => format!("{}M", trim_float(p * 1000.0)),
            None => "MoE".into(),
        }
    }

    pub fn supports_runtime(&self, runtime_id: &str) -> bool {
        self.runtimes.iter().any(|r| r == runtime_id)
    }
}

fn trim_float(v: f32) -> String {
    if (v - v.round()).abs() < f32::EPSILON {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// El catálogo efectivo: filas más el origen del que salieron.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub source: String,
    pub models: Vec<ModelSpec>,
}

impl Catalog {
    /// El catálogo que se distribuye con la aplicación.
    pub fn embedded() -> Self {
        Self {
            source: "embedded".into(),
            models: data::embedded_models(),
        }
    }

    /// Carga un catálogo desde JSON (fichero local o *mirror* corporativo).
    pub fn from_json(source: impl Into<String>, json: &str) -> Result<Self, serde_json::Error> {
        let models: Vec<ModelSpec> = serde_json::from_str(json)?;
        Ok(Self {
            source: source.into(),
            models,
        })
    }

    pub fn get(&self, id: &str) -> Option<&ModelSpec> {
        self.models.iter().find(|m| m.id == id)
    }

    /// Aplica las listas de la política. La lista de prohibidos manda sobre la de
    /// permitidos: en una empresa, prohibir algo por error es recuperable;
    /// permitirlo por error, no.
    pub fn filtered(&self, allow: &[String], deny: &[String]) -> Self {
        let models = self
            .models
            .iter()
            .filter(|m| !matches_any(&m.id, deny))
            .filter(|m| allow.is_empty() || matches_any(&m.id, allow))
            .cloned()
            .collect();
        Self {
            source: self.source.clone(),
            models,
        }
    }
}

/// Coincidencia de patrón sencilla: igualdad exacta o prefijo con `*` al final.
/// Deliberadamente no es una expresión regular — una política mal escrita no debe
/// poder colgar la aplicación.
fn matches_any(id: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => id.starts_with(prefix),
        None => id == p,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_well_formed() {
        let cat = Catalog::embedded();
        assert!(
            cat.models.len() >= 5,
            "el catálogo debe cubrir varias bandas de RAM"
        );
        let mut ids = std::collections::HashSet::new();
        for m in &cat.models {
            assert!(ids.insert(m.id.clone()), "id duplicado: {}", m.id);
            assert!(!m.runtimes.is_empty(), "{} sin runtime", m.id);
            assert!(m.file_bytes > 0, "{} sin tamaño", m.id);
            assert!(m.context_window >= 2048, "{} con contexto irreal", m.id);
            assert!(!m.license.is_empty(), "{} sin licencia declarada", m.id);
            if let ModelSource::GgufUrl { sha256, url, .. } = &m.source {
                assert!(
                    url.starts_with("https://"),
                    "{} debe descargarse por HTTPS",
                    m.id
                );
                if let Some(d) = sha256 {
                    assert_eq!(
                        d.len(),
                        64,
                        "{}: un SHA-256 anclado debe ser completo",
                        m.id
                    );
                    assert!(
                        d.chars().all(|c| c.is_ascii_hexdigit()),
                        "{}: SHA-256 no hexadecimal",
                        m.id
                    );
                }
            }
        }
    }

    #[test]
    fn catalog_is_ordered_by_capability_descending() {
        let cat = Catalog::embedded();
        let caps: Vec<u16> = cat.models.iter().map(|m| m.capability).collect();
        let mut sorted = caps.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(
            caps, sorted,
            "las filas deben ir de mayor a menor capacidad"
        );
    }

    #[test]
    fn denylist_wins_over_allowlist() {
        let cat = Catalog::embedded();
        let id = cat.models[0].id.clone();
        let one = std::slice::from_ref(&id);
        let filtered = cat.filtered(one, one);
        assert!(filtered.get(&id).is_none());
    }

    #[test]
    fn allowlist_prefix_pattern() {
        let cat = Catalog::embedded();
        let filtered = cat.filtered(&["qwen*".to_string()], &[]);
        assert!(!filtered.models.is_empty());
        assert!(filtered.models.iter().all(|m| m.id.starts_with("qwen")));
    }

    #[test]
    fn empty_allowlist_means_everything_allowed() {
        let cat = Catalog::embedded();
        assert_eq!(cat.filtered(&[], &[]).models.len(), cat.models.len());
    }

    #[test]
    fn kv_cache_grows_linearly_with_context() {
        let kv = KvGeometry {
            layers: 28,
            kv_heads: 4,
            head_dim: 128,
        };
        assert_eq!(kv.bytes_per_token(), 2 * 28 * 4 * 128);
        assert_eq!(kv.kv_cache_bytes(8192), kv.bytes_per_token() * 8192);
        assert_eq!(kv.kv_cache_bytes(0), 0);
    }

    #[test]
    fn size_label_reads_naturally() {
        let mut m = Catalog::embedded().models[0].clone();
        m.params_b = Some(7.0);
        assert_eq!(m.size_label(), "7B");
        m.params_b = Some(1.5);
        assert_eq!(m.size_label(), "1.5B");
        m.params_b = None;
        assert_eq!(m.size_label(), "MoE");
    }

    #[test]
    fn catalog_round_trips_through_json() {
        let cat = Catalog::embedded();
        let json = serde_json::to_string(&cat.models).unwrap();
        let back = Catalog::from_json("test", &json).unwrap();
        assert_eq!(back.models, cat.models);
    }
}
