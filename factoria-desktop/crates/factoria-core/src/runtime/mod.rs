//! Abstracción de motores de inferencia local.
//!
//! Rebost está acoplado a llama.cpp. FactorIA introduce un `trait` para que
//! añadir un motor (MLX en Apple Silicon, ONNX Runtime, vLLM…) sea implementar
//! `LlmRuntime` y registrarlo: nada fuera de este módulo cambia.
//!
//! Los dos adaptadores actuales hablan HTTP con un *endpoint* local. La
//! diferencia real entre ellos es el **ciclo de vida** (llama.cpp es un proceso
//! que lanzamos nosotros; Ollama es un demonio que ya existe) y la
//! **instalación** (GGUF descargado frente a `ollama pull`). Por eso el trait
//! cubre `install` y `start`, no solo `chat`.

pub mod llamacpp;
pub mod ollama;
pub mod registry;
pub mod sse;
pub mod tuning;

pub use registry::RuntimeRegistry;
pub use tuning::Tuning;

use crate::catalog::ModelSpec;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("el motor {0} no está disponible en este equipo")]
    Unavailable(String),
    #[error("el modelo {0} no está instalado")]
    NotInstalled(String),
    #[error("no hay ningún modelo en ejecución")]
    NotRunning,
    #[error("el motor no llegó a estar listo a tiempo")]
    StartTimeout,
    #[error("error del motor: {0}")]
    Engine(String),
    #[error(transparent)]
    Download(#[from] crate::download::DownloadError),
    #[error("error de disco: {0}")]
    Io(#[from] std::io::Error),
    #[error("el fichero de modelo no es válido: {0}")]
    Gguf(#[from] crate::gguf::GgufError),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Estado de un motor en este equipo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum RuntimeStatus {
    /// Listo para usarse.
    Ready,
    /// Se puede usar pero falta preparar algo (descargar el binario del motor).
    NeedsSetup { detail: String },
    /// No está y no lo podemos instalar nosotros (p. ej. Ollama no instalado).
    Unavailable { detail: String },
}

impl RuntimeStatus {
    pub fn usable(&self) -> bool {
        matches!(self, Self::Ready | Self::NeedsSetup { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Si FactorIA puede dejarlo funcionando sin intervención del empleado.
    pub self_installable: bool,
}

/// Un modelo presente en disco y listo para ejecutarse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub model_id: String,
    pub runtime: String,
    pub path: Option<String>,
    pub bytes: u64,
    pub installed_at: String,
    pub sha256: Option<String>,
    pub integrity: String,
    pub context_window: u32,
}

/// Un modelo cargado y atendiendo peticiones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningModel {
    pub model_id: String,
    pub runtime: String,
    /// *Endpoint* que atiende la generación. Es lo que decide si la interfaz
    /// puede afirmar "Procesando localmente".
    pub endpoint: String,
    pub context_tokens: u32,
    pub started_at: String,
}

impl RunningModel {
    /// ¿La inferencia ocurre de verdad en este equipo?
    ///
    /// La afirmación de la interfaz se deriva de aquí, no de una constante: si
    /// algún día hubiera un motor remoto, el indicador se apagaría solo.
    pub fn is_local(&self) -> bool {
        endpoint_is_loopback(&self.endpoint)
    }
}

/// `true` solo si el *host* del *endpoint* es la propia máquina.
pub fn endpoint_is_loopback(endpoint: &str) -> bool {
    let without_scheme = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    let host = without_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or_else(|| without_scheme.split(['/', '?']).next().unwrap_or(""));
    let host = host.trim_matches(|c| c == '[' || c == ']');
    matches!(host, "127.0.0.1" | "localhost" | "::1") || host.starts_with("127.")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Bloque de contexto adjunto a una petición.
///
/// Hoy siempre vacío. Existe para que el RAG local (documentos, embeddings) se
/// pueda añadir más adelante sin cambiar la firma del trait ni los adaptadores.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBlock {
    pub id: String,
    pub title: String,
    pub text: String,
}

/// Declaración de herramienta. Hoy siempre vacía; reservada para *tool calling*
/// y MCP. El adaptador de llama.cpp ya usa el esquema de OpenAI, que lo soporta.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub model_id: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub max_tokens: u32,
    #[serde(default)]
    pub context_blocks: Vec<ContextBlock>,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
}

/// Un trozo de la respuesta según llega.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    Text(String),
    /// Reservado para *tool calling*.
    ToolCall {
        name: String,
        arguments: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOutcome {
    pub stopped: bool,
    pub finish_reason: Option<String>,
}

/// Señal de parada compartida entre la interfaz y la generación en curso.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Consumidor de *deltas*. Devolver `false` interrumpe la generación.
pub type TokenSink = Box<dyn FnMut(Delta) -> bool + Send>;
/// Consumidor de progreso de instalación (bytes recibidos, total conocido).
pub type ProgressSink = Box<dyn FnMut(u64, Option<u64>) + Send>;

#[async_trait]
pub trait LlmRuntime: Send + Sync {
    fn descriptor(&self) -> RuntimeDescriptor;

    fn id(&self) -> String {
        self.descriptor().id
    }

    async fn probe(&self) -> RuntimeStatus;

    async fn installed_models(&self) -> Result<Vec<InstalledModel>>;

    async fn install(&self, spec: &ModelSpec, progress: ProgressSink) -> Result<InstalledModel>;

    async fn remove(&self, model_id: &str) -> Result<()>;

    async fn start(&self, spec: &ModelSpec, tuning: &Tuning) -> Result<RunningModel>;

    async fn stop(&self, model_id: &str) -> Result<()>;

    async fn running(&self) -> Option<RunningModel>;

    async fn chat_stream(
        &self,
        req: ChatRequest,
        sink: TokenSink,
        cancel: CancelToken,
    ) -> Result<ChatOutcome>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_endpoints_are_recognised() {
        for e in [
            "http://127.0.0.1:8081",
            "http://localhost:11434/api/chat",
            "http://[::1]:8080/v1",
            "http://127.0.0.53:9999",
        ] {
            assert!(endpoint_is_loopback(e), "debería ser local: {e}");
        }
    }

    #[test]
    fn remote_endpoints_are_not_local() {
        for e in [
            "https://api.openai.com/v1",
            "http://10.0.0.5:8080",
            "http://gpu-server.naturgy.com:8081",
            "http://192.168.1.20:11434",
        ] {
            assert!(!endpoint_is_loopback(e), "no debería ser local: {e}");
        }
    }

    #[test]
    fn the_local_badge_follows_the_endpoint_not_a_constant() {
        let mut m = RunningModel {
            model_id: "m".into(),
            runtime: "llamacpp".into(),
            endpoint: "http://127.0.0.1:8081".into(),
            context_tokens: 8192,
            started_at: "now".into(),
        };
        assert!(m.is_local());
        m.endpoint = "https://inference.example.com".into();
        assert!(!m.is_local(), "un motor remoto debe apagar el indicador");
    }

    #[test]
    fn a_cancel_token_is_shared_between_clones() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn runtime_status_usability() {
        assert!(RuntimeStatus::Ready.usable());
        assert!(RuntimeStatus::NeedsSetup { detail: "x".into() }.usable());
        assert!(!RuntimeStatus::Unavailable { detail: "x".into() }.usable());
    }
}
