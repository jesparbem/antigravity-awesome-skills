//! Registro de auditoría local.
//!
//! Fichero `audit/audit-YYYY-MM.jsonl`, una línea por evento, solo se añade.
//! **Nunca contiene el texto de los prompts ni de las respuestas**: registra qué
//! se hizo, con qué modelo y con qué resultado, que es lo que un despliegue
//! corporativo necesita poder recolectar.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub ts: String,
    pub event: String,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub outcome: String,
}

impl AuditEntry {
    pub fn new(event: &str, outcome: &str) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            event: event.to_string(),
            actor: "local-user".into(),
            model_id: None,
            runtime: None,
            bytes: None,
            duration_ms: None,
            outcome: outcome.to_string(),
        }
    }

    pub fn model(mut self, id: impl Into<String>) -> Self {
        self.model_id = Some(id.into());
        self
    }

    pub fn runtime(mut self, id: impl Into<String>) -> Self {
        self.runtime = Some(id.into());
        self
    }

    pub fn bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    pub fn duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }
}

#[derive(Clone)]
pub struct AuditLog {
    paths: Paths,
    enabled: bool,
}

impl AuditLog {
    pub fn new(paths: Paths, enabled: bool) -> Self {
        Self { paths, enabled }
    }

    pub fn record(&self, entry: AuditEntry) {
        if !self.enabled {
            return;
        }
        if let Err(err) = self.append(&entry) {
            // Un fallo de auditoría no puede tumbar la aplicación del empleado.
            tracing::warn!(?err, "no se pudo escribir la auditoría");
        }
    }

    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        std::fs::create_dir_all(self.paths.audit_dir())?;
        let file = self.paths.audit_dir().join(format!(
            "audit-{}.jsonl",
            chrono::Utc::now().format("%Y-%m")
        ));
        let mut handle = OpenOptions::new().create(true).append(true).open(file)?;
        let line = serde_json::to_string(entry)?;
        writeln!(handle, "{line}")
    }

    /// Últimas `limit` entradas del mes en curso, para Ajustes → Auditoría.
    pub fn tail(&self, limit: usize) -> Vec<AuditEntry> {
        let file = self.paths.audit_dir().join(format!(
            "audit-{}.jsonl",
            chrono::Utc::now().format("%Y-%m")
        ));
        let Ok(raw) = std::fs::read_to_string(file) else {
            return Vec::new();
        };
        let mut all: Vec<AuditEntry> = raw
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if all.len() > limit {
            all.drain(..all.len() - limit);
        }
        all.reverse();
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(enabled: bool) -> (tempfile::TempDir, AuditLog) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        let log = AuditLog::new(paths, enabled);
        (tmp, log)
    }

    #[test]
    fn entries_are_appended_and_read_back_newest_first() {
        let (_tmp, log) = log(true);
        log.record(AuditEntry::new("model.install", "ok").model("a"));
        log.record(AuditEntry::new("model.start", "ok").model("b"));
        let tail = log.tail(10);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].model_id.as_deref(), Some("b"));
    }

    #[test]
    fn a_disabled_log_writes_nothing() {
        let (_tmp, log) = log(false);
        log.record(AuditEntry::new("model.install", "ok"));
        assert!(log.tail(10).is_empty());
    }

    #[test]
    fn the_audit_record_has_no_field_that_could_carry_a_prompt() {
        let entry = AuditEntry::new("chat.turn", "ok").model("m").duration(120);
        let json = serde_json::to_value(&entry).unwrap();
        let allowed = [
            "ts",
            "event",
            "actor",
            "modelId",
            "runtime",
            "bytes",
            "durationMs",
            "outcome",
        ];
        for key in json.as_object().unwrap().keys() {
            assert!(
                allowed.contains(&key.as_str()),
                "campo inesperado en auditoría: {key}"
            );
        }
    }

    #[test]
    fn tail_truncates_to_the_requested_limit() {
        let (_tmp, log) = log(true);
        for i in 0..20 {
            log.record(AuditEntry::new("model.start", "ok").model(format!("m{i}")));
        }
        let tail = log.tail(5);
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0].model_id.as_deref(), Some("m19"));
    }

    #[test]
    fn tail_on_a_fresh_install_is_empty_not_an_error() {
        let (_tmp, log) = log(true);
        assert!(log.tail(10).is_empty());
    }
}
