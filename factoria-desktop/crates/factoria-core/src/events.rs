//! Bus de eventos interno.
//!
//! Un único flujo que ambos *hosts* reexponen: Tauri como eventos `factoria://…`
//! y el servidor HTTP como SSE en `/api/events`. La interfaz consume la misma
//! forma en los dos casos.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppEvent {
    #[serde(rename_all = "camelCase")]
    DownloadProgress {
        model_id: String,
        received_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_second: u64,
    },
    #[serde(rename_all = "camelCase")]
    DownloadFinished {
        model_id: String,
        ok: bool,
        message: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ModelStateChanged { model_id: String, state: String },
    #[serde(rename_all = "camelCase")]
    RuntimeStatus {
        runtime: String,
        state: String,
        detail: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ChatDelta {
        thread_id: String,
        message_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    ChatDone {
        thread_id: String,
        message_id: String,
        stopped: bool,
        metrics: Option<crate::metrics::GenerationMetrics>,
    },
    #[serde(rename_all = "camelCase")]
    ChatError { thread_id: String, message: String },
}

/// Capacidad del canal. Si un consumidor lento se queda atrás pierde eventos
/// intermedios pero no bloquea la generación, que es el comportamiento correcto
/// para un flujo de tokens.
const CHANNEL_CAPACITY: usize = 512;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx }
    }

    pub fn emit(&self, event: AppEvent) {
        // Sin suscriptores no es un error: la aplicación puede estar arrancando.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_what_is_emitted() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.emit(AppEvent::ChatDelta {
            thread_id: "t".into(),
            message_id: "m".into(),
            text: "hola".into(),
        });
        let got = rx.recv().await.unwrap();
        assert!(matches!(got, AppEvent::ChatDelta { text, .. } if text == "hola"));
    }

    #[test]
    fn emitting_without_subscribers_is_not_an_error() {
        EventBus::new().emit(AppEvent::ChatError {
            thread_id: "t".into(),
            message: "x".into(),
        });
    }

    #[test]
    fn events_serialise_with_a_discriminating_tag() {
        let json = serde_json::to_string(&AppEvent::ModelStateChanged {
            model_id: "m".into(),
            state: "running".into(),
        })
        .unwrap();
        assert!(json.contains(r#""type":"modelStateChanged""#));
        assert!(json.contains(r#""modelId":"m""#));
    }
}
