//! Adaptador de **Ollama**, para equipos donde ya está instalado.
//!
//! Es un motor secundario a propósito: Ollama exige una instalación previa que
//! en un parque corporativo no está garantizada, así que FactorIA lo detecta y
//! lo aprovecha, pero no depende de él. Rebost usa Ollama solo como fuente de
//! catálogo; aquí es un motor de ejecución de pleno derecho.

use super::sse::{OllamaParser, StreamEvent};
use super::{
    CancelToken, ChatOutcome, ChatRequest, InstalledModel, LlmRuntime, ProgressSink, Result,
    RunningModel, RuntimeDescriptor, RuntimeError, RuntimeStatus, TokenSink, Tuning,
};
use crate::catalog::{ModelSource, ModelSpec};
use async_trait::async_trait;
use futures_util::StreamExt;
use std::time::Duration;

pub const RUNTIME_ID: &str = "ollama";
/// Puerto por defecto del demonio. `OLLAMA_HOST` lo sobreescribe, igual que en
/// el propio Ollama.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";

pub struct OllamaRuntime {
    client: reqwest::Client,
    endpoint: String,
}

impl OllamaRuntime {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            endpoint: std::env::var("OLLAMA_HOST")
                .ok()
                .filter(|h| !h.is_empty())
                .map(|h| {
                    if h.starts_with("http") {
                        h
                    } else {
                        format!("http://{h}")
                    }
                })
                .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
            client,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Etiqueta de Ollama que corresponde a un modelo del catálogo.
    fn tag_for(spec: &ModelSpec) -> Option<&str> {
        match &spec.source {
            ModelSource::OllamaTag { tag } => Some(tag),
            ModelSource::GgufUrl { .. } => None,
        }
    }
}

/// `/api/tags` → los modelos que el demonio tiene descargados.
pub fn parse_tags(json: &str) -> Vec<InstalledModel> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(models) = value.get("models").and_then(|m| m.as_array()) else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|m| {
            let name = m.get("name")?.as_str()?.to_string();
            Some(InstalledModel {
                model_id: name.clone(),
                runtime: RUNTIME_ID.into(),
                path: None,
                bytes: m.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                installed_at: m
                    .get("modified_at")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
                sha256: m
                    .get("digest")
                    .and_then(|d| d.as_str())
                    .map(|d| d.trim_start_matches("sha256:").to_string()),
                integrity: "origin".into(),
                context_window: m
                    .get("details")
                    .and_then(|d| d.get("context_length"))
                    .and_then(|c| c.as_u64())
                    .unwrap_or(4096) as u32,
            })
        })
        .collect()
}

/// Una línea de progreso de `/api/pull`: `{"total":…, "completed":…}`.
pub fn parse_pull_progress(line: &str) -> Option<(u64, Option<u64>)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let completed = value.get("completed").and_then(|c| c.as_u64())?;
    Some((completed, value.get("total").and_then(|t| t.as_u64())))
}

#[async_trait]
impl LlmRuntime for OllamaRuntime {
    fn descriptor(&self) -> RuntimeDescriptor {
        RuntimeDescriptor {
            id: RUNTIME_ID.into(),
            name: "Ollama".into(),
            description: "Usa el Ollama ya instalado en este equipo.".into(),
            self_installable: false,
        }
    }

    async fn probe(&self) -> RuntimeStatus {
        let res = self
            .client
            .get(format!("{}/api/tags", self.endpoint))
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => RuntimeStatus::Ready,
            _ => RuntimeStatus::Unavailable {
                detail: "Ollama no está en marcha en este equipo.".into(),
            },
        }
    }

    async fn installed_models(&self) -> Result<Vec<InstalledModel>> {
        let res = self
            .client
            .get(format!("{}/api/tags", self.endpoint))
            .send()
            .await
            .map_err(|e| RuntimeError::Unavailable(e.to_string()))?;
        let body = res
            .text()
            .await
            .map_err(|e| RuntimeError::Engine(e.to_string()))?;
        Ok(parse_tags(&body))
    }

    async fn install(
        &self,
        spec: &ModelSpec,
        mut progress: ProgressSink,
    ) -> Result<InstalledModel> {
        let tag = Self::tag_for(spec).ok_or_else(|| {
            RuntimeError::Engine("este modelo no tiene etiqueta de Ollama".into())
        })?;
        let res = self
            .client
            .post(format!("{}/api/pull", self.endpoint))
            .json(&serde_json::json!({ "model": tag, "stream": true }))
            .send()
            .await
            .map_err(|e| RuntimeError::Unavailable(e.to_string()))?;
        if !res.status().is_success() {
            return Err(RuntimeError::Engine(format!(
                "Ollama respondió {}",
                res.status()
            )));
        }

        let mut stream = res.bytes_stream();
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| RuntimeError::Engine(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(idx) = buffer.find('\n') {
                let line: String = buffer.drain(..=idx).collect();
                if let Some((done, total)) = parse_pull_progress(line.trim()) {
                    progress(done, total);
                }
            }
        }

        self.installed_models()
            .await?
            .into_iter()
            .find(|m| m.model_id == tag || m.model_id.starts_with(&format!("{tag}:")))
            .ok_or_else(|| RuntimeError::NotInstalled(tag.to_string()))
    }

    async fn remove(&self, model_id: &str) -> Result<()> {
        self.client
            .delete(format!("{}/api/delete", self.endpoint))
            .json(&serde_json::json!({ "model": model_id }))
            .send()
            .await
            .map_err(|e| RuntimeError::Unavailable(e.to_string()))?;
        Ok(())
    }

    async fn start(&self, spec: &ModelSpec, tuning: &Tuning) -> Result<RunningModel> {
        // Ollama carga el modelo bajo demanda: "arrancar" es confirmar que
        // responde. Se envía una petición mínima para forzar la carga, de modo
        // que el primer mensaje del empleado no pague la espera.
        let tag = Self::tag_for(spec).unwrap_or(&spec.id);
        let res = self
            .client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&serde_json::json!({ "model": tag, "prompt": "", "stream": false }))
            .send()
            .await
            .map_err(|e| RuntimeError::Unavailable(e.to_string()))?;
        if !res.status().is_success() {
            return Err(RuntimeError::Engine(format!(
                "Ollama respondió {}",
                res.status()
            )));
        }
        Ok(RunningModel {
            model_id: spec.id.clone(),
            runtime: RUNTIME_ID.into(),
            endpoint: self.endpoint.clone(),
            context_tokens: tuning.context_tokens,
            started_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn stop(&self, model_id: &str) -> Result<()> {
        // `keep_alive: 0` descarga el modelo de memoria inmediatamente.
        let _ = self
            .client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&serde_json::json!({ "model": model_id, "keep_alive": 0 }))
            .send()
            .await;
        Ok(())
    }

    async fn running(&self) -> Option<RunningModel> {
        // El demonio gestiona su propia residencia en memoria; FactorIA no
        // mantiene estado propio aquí.
        None
    }

    async fn chat_stream(
        &self,
        req: ChatRequest,
        mut sink: TokenSink,
        cancel: CancelToken,
    ) -> Result<ChatOutcome> {
        let body = serde_json::json!({
            "model": req.model_id,
            "messages": req.messages.iter().map(|m| serde_json::json!({
                "role": m.role,
                "content": m.content,
            })).collect::<Vec<_>>(),
            "stream": true,
            "options": { "temperature": req.temperature, "num_predict": req.max_tokens },
        });

        let res = self
            .client
            .post(format!("{}/api/chat", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Unavailable(e.to_string()))?;
        if !res.status().is_success() {
            return Err(RuntimeError::Engine(format!(
                "Ollama respondió {}",
                res.status()
            )));
        }

        let mut parser = OllamaParser::new();
        let mut stream = res.bytes_stream();
        let mut stopped = false;
        let mut finish_reason = None;

        while let Some(chunk) = stream.next().await {
            if cancel.is_cancelled() {
                stopped = true;
                break;
            }
            let chunk = chunk.map_err(|e| RuntimeError::Engine(e.to_string()))?;
            for event in parser.push(&String::from_utf8_lossy(&chunk)) {
                match event {
                    StreamEvent::Delta(delta) => {
                        if !sink(delta) {
                            stopped = true;
                            break;
                        }
                    }
                    StreamEvent::Done(reason) => finish_reason = reason,
                }
            }
            if stopped || parser.finished() {
                break;
            }
        }

        Ok(ChatOutcome {
            stopped: stopped || cancel.is_cancelled(),
            finish_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_parsed_into_installed_models() {
        let json = r#"{"models":[
            {"name":"llama3.1:8b","size":4920000000,"modified_at":"2026-01-02T10:00:00Z","digest":"sha256:abc"},
            {"name":"qwen2.5:7b","size":4683000000,"modified_at":"2026-01-03T10:00:00Z"}
        ]}"#;
        let models = parse_tags(json);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].model_id, "llama3.1:8b");
        assert_eq!(models[0].sha256.as_deref(), Some("abc"));
        assert_eq!(models[0].runtime, RUNTIME_ID);
        assert_eq!(models[1].bytes, 4683000000);
    }

    #[test]
    fn an_empty_or_broken_tags_response_is_no_models() {
        assert!(parse_tags(r#"{"models":[]}"#).is_empty());
        assert!(parse_tags("no es json").is_empty());
        assert!(parse_tags("{}").is_empty());
    }

    #[test]
    fn pull_progress_lines_are_parsed() {
        assert_eq!(
            parse_pull_progress(r#"{"status":"downloading","completed":120,"total":500}"#),
            Some((120, Some(500)))
        );
        assert_eq!(
            parse_pull_progress(r#"{"status":"pulling manifest"}"#),
            None,
            "una línea sin bytes no es progreso"
        );
        assert_eq!(parse_pull_progress("roto"), None);
    }

    #[test]
    fn the_endpoint_follows_ollama_host_when_it_is_set() {
        let client = reqwest::Client::new();
        std::env::set_var("OLLAMA_HOST", "127.0.0.1:12345");
        let rt = OllamaRuntime::new(client.clone());
        std::env::remove_var("OLLAMA_HOST");
        assert_eq!(rt.endpoint(), "http://127.0.0.1:12345");
        assert_eq!(OllamaRuntime::new(client).endpoint(), DEFAULT_ENDPOINT);
    }

    #[tokio::test]
    async fn probing_a_machine_without_ollama_reports_unavailable() {
        // Puerto cerrado a propósito: es el caso del PC corporativo típico.
        std::env::set_var("OLLAMA_HOST", "127.0.0.1:1");
        let rt = OllamaRuntime::new(reqwest::Client::new());
        std::env::remove_var("OLLAMA_HOST");
        assert!(!rt.probe().await.usable());
    }

    #[test]
    fn a_gguf_model_has_no_ollama_tag() {
        let spec = crate::catalog::Catalog::embedded().models[0].clone();
        assert!(OllamaRuntime::tag_for(&spec).is_none());
    }
}
