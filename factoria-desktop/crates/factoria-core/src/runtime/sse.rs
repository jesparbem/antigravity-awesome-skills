//! Parseo del *streaming* de respuestas.
//!
//! `llama-server` emite SSE con el formato de OpenAI; Ollama emite JSON por
//! líneas. Los dos parsers viven aquí, separados del transporte, para poder
//! probarlos con las tramas exactas que emiten los motores.

use super::Delta;

/// Estado de un flujo SSE que llega en trozos arbitrarios.
///
/// Un `chunk` de red puede partir un evento por la mitad, así que hay que
/// acumular hasta ver el separador de línea.
#[derive(Default)]
pub struct SseParser {
    buffer: String,
    done: bool,
}

/// Lo que un evento del flujo significa para el orquestador.
#[derive(Debug, PartialEq)]
pub enum StreamEvent {
    Delta(Delta),
    /// El motor terminó. Lleva el motivo si lo declara.
    Done(Option<String>),
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    /// Consume un trozo del flujo y devuelve los eventos completos que contenga.
    pub fn push(&mut self, chunk: &str) -> Vec<StreamEvent> {
        self.buffer.push_str(chunk);
        let mut out = Vec::new();

        while let Some(idx) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=idx).collect();
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some(payload) = line.strip_prefix("data:") else {
                // Comentarios (`: keep-alive`) y campos que no usamos.
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                self.done = true;
                out.push(StreamEvent::Done(None));
                continue;
            }
            if let Some(event) = parse_openai_chunk(payload) {
                if matches!(event, StreamEvent::Done(_)) {
                    self.done = true;
                }
                out.push(event);
            }
        }
        out
    }
}

/// Un objeto `chat.completion.chunk` de la API de OpenAI, que es lo que
/// `llama-server` emite en `/v1/chat/completions` con `stream: true`.
fn parse_openai_chunk(payload: &str) -> Option<StreamEvent> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let choice = value.get("choices")?.as_array()?.first()?;

    if let Some(content) = choice
        .get("delta")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
    {
        if !content.is_empty() {
            return Some(StreamEvent::Delta(Delta::Text(content.to_string())));
        }
    }

    // Reservado para *tool calling*: el esquema ya lo contempla.
    if let Some(call) = choice
        .get("delta")
        .and_then(|d| d.get("tool_calls"))
        .and_then(|t| t.as_array())
        .and_then(|a| a.first())
    {
        let function = call.get("function")?;
        return Some(StreamEvent::Delta(Delta::ToolCall {
            name: function.get("name")?.as_str()?.to_string(),
            arguments: function
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string(),
        }));
    }

    match choice.get("finish_reason") {
        Some(serde_json::Value::String(reason)) => Some(StreamEvent::Done(Some(reason.clone()))),
        _ => None,
    }
}

/// Ollama emite un objeto JSON por línea, no SSE.
#[derive(Default)]
pub struct OllamaParser {
    buffer: String,
    done: bool,
}

impl OllamaParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    pub fn push(&mut self, chunk: &str) -> Vec<StreamEvent> {
        self.buffer.push_str(chunk);
        let mut out = Vec::new();
        while let Some(idx) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=idx).collect();
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if let Some(content) = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
            {
                if !content.is_empty() {
                    out.push(StreamEvent::Delta(Delta::Text(content.to_string())));
                }
            }
            if value.get("done").and_then(|d| d.as_bool()) == Some(true) {
                self.done = true;
                out.push(StreamEvent::Done(
                    value
                        .get("done_reason")
                        .and_then(|r| r.as_str())
                        .map(str::to_string),
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(events: &[StreamEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Delta(Delta::Text(t)) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_complete_openai_stream_is_reassembled() {
        let mut p = SseParser::new();
        let mut events = Vec::new();
        for line in [
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\", \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"mundo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ] {
            events.extend(p.push(line));
        }
        assert_eq!(text(&events), "Hola, mundo");
        assert!(p.finished());
        assert!(events.contains(&StreamEvent::Done(Some("stop".into()))));
    }

    #[test]
    fn an_event_split_across_network_chunks_is_not_lost() {
        let mut p = SseParser::new();
        assert!(p.push("data: {\"choices\":[{\"delta\":{\"con").is_empty());
        let events = p.push("tent\":\"hola\"}}]}\n");
        assert_eq!(text(&events), "hola");
    }

    #[test]
    fn keep_alive_comments_are_ignored() {
        let mut p = SseParser::new();
        assert!(p.push(": keep-alive\n\n").is_empty());
        assert!(!p.finished());
    }

    #[test]
    fn empty_deltas_do_not_produce_empty_tokens() {
        let mut p = SseParser::new();
        let events = p.push("data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n");
        assert!(events.is_empty(), "un delta vacío no es un token");
    }

    #[test]
    fn malformed_json_is_skipped_instead_of_breaking_the_stream() {
        let mut p = SseParser::new();
        p.push("data: {roto\n");
        let events = p.push("data: {\"choices\":[{\"delta\":{\"content\":\"sigo\"}}]}\n");
        assert_eq!(text(&events), "sigo");
    }

    #[test]
    fn tool_calls_are_surfaced_for_the_future() {
        let mut p = SseParser::new();
        let events = p.push(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"name\":\"buscar\",\"arguments\":\"{}\"}}]}}]}\n",
        );
        assert_eq!(
            events[0],
            StreamEvent::Delta(Delta::ToolCall {
                name: "buscar".into(),
                arguments: "{}".into()
            })
        );
    }

    #[test]
    fn done_without_a_data_prefix_does_not_end_the_stream() {
        let mut p = SseParser::new();
        p.push("[DONE]\n");
        assert!(!p.finished());
    }

    #[test]
    fn ollama_ndjson_is_reassembled() {
        let mut p = OllamaParser::new();
        let mut events = Vec::new();
        events.extend(
            p.push("{\"message\":{\"role\":\"assistant\",\"content\":\"Bue\"},\"done\":false}\n"),
        );
        events.extend(p.push("{\"message\":{\"content\":\"nas\"},\"done\":false}\n"));
        events.extend(
            p.push("{\"message\":{\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\"}\n"),
        );
        assert_eq!(text(&events), "Buenas");
        assert!(p.finished());
        assert!(events.contains(&StreamEvent::Done(Some("stop".into()))));
    }

    #[test]
    fn ollama_partial_line_waits_for_the_newline() {
        let mut p = OllamaParser::new();
        assert!(p.push("{\"message\":{\"content\":\"a").is_empty());
        assert_eq!(text(&p.push("\"},\"done\":false}\n")), "a");
    }
}
