//! Registro de motores disponibles.
//!
//! Mantiene los motores en orden de preferencia y resuelve cuál ejecuta un
//! modelo dado. Añadir un motor nuevo (MLX, ONNX Runtime, vLLM…) es registrarlo
//! aquí; el resto de la aplicación no cambia.

use super::{LlmRuntime, RuntimeStatus};
use crate::catalog::ModelSpec;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReport {
    pub id: String,
    pub name: String,
    pub description: String,
    pub self_installable: bool,
    #[serde(flatten)]
    pub status: RuntimeStatus,
}

#[derive(Clone, Default)]
pub struct RuntimeRegistry {
    runtimes: Vec<Arc<dyn LlmRuntime>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra un motor. El orden de registro es el orden de preferencia.
    pub fn register(&mut self, runtime: Arc<dyn LlmRuntime>) {
        self.runtimes.push(runtime);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn LlmRuntime>> {
        self.runtimes.iter().find(|r| r.id() == id).cloned()
    }

    pub fn all(&self) -> &[Arc<dyn LlmRuntime>] {
        &self.runtimes
    }

    /// Estado de cada motor, para Ajustes → Diagnóstico.
    pub async fn report(&self) -> Vec<RuntimeReport> {
        let mut out = Vec::new();
        for runtime in &self.runtimes {
            let d = runtime.descriptor();
            out.push(RuntimeReport {
                id: d.id,
                name: d.name,
                description: d.description,
                self_installable: d.self_installable,
                status: runtime.probe().await,
            });
        }
        out
    }

    /// Identificadores de los motores utilizables ahora mismo. Es lo que
    /// `catalog::fit` necesita para no recomendar un modelo que no se puede
    /// ejecutar.
    pub async fn available_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for runtime in &self.runtimes {
            if runtime.probe().await.usable() {
                out.push(runtime.id());
            }
        }
        out
    }

    /// Motor que ejecuta este modelo: el primero que el modelo declara y que
    /// además está disponible. El orden lo marca el modelo, no el registro,
    /// porque un modelo puede preferir un motor concreto.
    pub async fn resolve_for(&self, spec: &ModelSpec) -> Option<Arc<dyn LlmRuntime>> {
        for wanted in &spec.runtimes {
            if let Some(runtime) = self.get(wanted) {
                if runtime.probe().await.usable() {
                    return Some(runtime);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        CancelToken, ChatOutcome, ChatRequest, InstalledModel, ProgressSink, Result, RunningModel,
        RuntimeDescriptor, RuntimeError, TokenSink, Tuning,
    };
    use async_trait::async_trait;

    struct Fake {
        id: &'static str,
        status: RuntimeStatus,
    }

    #[async_trait]
    impl LlmRuntime for Fake {
        fn descriptor(&self) -> RuntimeDescriptor {
            RuntimeDescriptor {
                id: self.id.into(),
                name: self.id.into(),
                description: "motor de prueba".into(),
                self_installable: true,
            }
        }
        async fn probe(&self) -> RuntimeStatus {
            self.status.clone()
        }
        async fn installed_models(&self) -> Result<Vec<InstalledModel>> {
            Ok(vec![])
        }
        async fn install(&self, _: &ModelSpec, _: ProgressSink) -> Result<InstalledModel> {
            Err(RuntimeError::NotRunning)
        }
        async fn remove(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn start(&self, _: &ModelSpec, _: &Tuning) -> Result<RunningModel> {
            Err(RuntimeError::NotRunning)
        }
        async fn stop(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn running(&self) -> Option<RunningModel> {
            None
        }
        async fn chat_stream(
            &self,
            _: ChatRequest,
            _: TokenSink,
            _: CancelToken,
        ) -> Result<ChatOutcome> {
            Err(RuntimeError::NotRunning)
        }
    }

    fn registry() -> RuntimeRegistry {
        let mut r = RuntimeRegistry::new();
        r.register(Arc::new(Fake {
            id: "llamacpp",
            status: RuntimeStatus::Ready,
        }));
        r.register(Arc::new(Fake {
            id: "ollama",
            status: RuntimeStatus::Unavailable {
                detail: "no instalado".into(),
            },
        }));
        r
    }

    fn spec(runtimes: &[&str]) -> ModelSpec {
        let mut s = crate::catalog::Catalog::embedded().models[0].clone();
        s.runtimes = runtimes.iter().map(|r| r.to_string()).collect();
        s
    }

    #[tokio::test]
    async fn only_usable_runtimes_are_reported_as_available() {
        assert_eq!(
            registry().available_ids().await,
            vec!["llamacpp".to_string()]
        );
    }

    #[tokio::test]
    async fn a_model_resolves_to_its_first_available_runtime() {
        let r = registry();
        let resolved = r.resolve_for(&spec(&["ollama", "llamacpp"])).await;
        assert_eq!(
            resolved.map(|r| r.id()),
            Some("llamacpp".to_string()),
            "Ollama no está disponible, así que cae al motor local"
        );
    }

    #[tokio::test]
    async fn a_model_with_no_available_runtime_resolves_to_nothing() {
        let r = registry();
        assert!(r.resolve_for(&spec(&["ollama"])).await.is_none());
    }

    #[tokio::test]
    async fn an_unknown_runtime_id_is_skipped_not_fatal() {
        let r = registry();
        let resolved = r.resolve_for(&spec(&["mlx", "llamacpp"])).await;
        assert_eq!(resolved.map(|r| r.id()), Some("llamacpp".to_string()));
    }

    #[tokio::test]
    async fn the_report_covers_every_registered_runtime() {
        let report = registry().report().await;
        assert_eq!(report.len(), 2);
        assert_eq!(report[0].status, RuntimeStatus::Ready);
        assert!(!report[1].status.usable());
    }

    #[test]
    fn an_empty_registry_is_valid() {
        assert!(RuntimeRegistry::new().get("llamacpp").is_none());
    }

    #[test]
    fn a_runtime_report_serialises_flat() {
        let json = serde_json::to_string(&RuntimeReport {
            id: "llamacpp".into(),
            name: "Motor local".into(),
            description: "d".into(),
            self_installable: true,
            status: RuntimeStatus::Ready,
        })
        .unwrap();
        assert!(json.contains(r#""state":"ready""#));
    }
}
