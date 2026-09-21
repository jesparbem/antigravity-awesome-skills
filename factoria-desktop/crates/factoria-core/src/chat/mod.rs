//! Conversaciones: modelo de datos y persistencia.
//!
//! Un índice `threads.json` con los metadatos y un `.jsonl` por conversación con
//! los mensajes. Formato tomado de Rebost: añadir un mensaje es una línea más,
//! sin reescribir el fichero, y un corte no corrompe lo anterior.
//!
//! Todo esto vive **solo en el equipo**. Ninguna ruta del producto envía estos
//! ficheros a ningún sitio.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub content: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<crate::metrics::GenerationMetrics>,
    /// Si la inferencia ocurrió en este equipo. Se guarda por mensaje porque es
    /// una propiedad de esa generación concreta, no de la conversación.
    #[serde(default)]
    pub local: bool,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            content: content.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            model_id: None,
            metrics: None,
            local: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
}

impl Thread {
    pub fn new(model_id: Option<String>) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Nueva conversación".into(),
            model_id,
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
        }
    }
}

/// Título a partir del primer mensaje del usuario, recortado por palabras para
/// que no quede una palabra partida por la mitad.
pub fn title_from(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return "Nueva conversación".into();
    }
    const MAX: usize = 48;
    if clean.chars().count() <= MAX {
        return clean;
    }
    let truncated: String = clean.chars().take(MAX).collect();
    match truncated.rsplit_once(' ') {
        Some((head, _)) if head.chars().count() >= 12 => format!("{head}…"),
        _ => format!("{truncated}…"),
    }
}

#[derive(Clone)]
pub struct ChatStore {
    paths: Paths,
}

impl ChatStore {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn threads(&self) -> Vec<Thread> {
        let mut threads: Vec<Thread> = std::fs::read_to_string(self.paths.threads_file())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        threads
    }

    fn save_threads(&self, threads: &[Thread]) -> std::io::Result<()> {
        std::fs::create_dir_all(self.paths.conversations_dir())?;
        let tmp = self.paths.threads_file().with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(threads)?)?;
        std::fs::rename(tmp, self.paths.threads_file())
    }

    pub fn create_thread(&self, model_id: Option<String>) -> std::io::Result<Thread> {
        let thread = Thread::new(model_id);
        let mut threads = self.threads();
        threads.push(thread.clone());
        self.save_threads(&threads)?;
        Ok(thread)
    }

    pub fn get_thread(&self, id: &str) -> Option<Thread> {
        self.threads().into_iter().find(|t| t.id == id)
    }

    pub fn messages(&self, thread_id: &str) -> Vec<Message> {
        std::fs::read_to_string(self.paths.thread_file(thread_id))
            .map(|raw| {
                raw.lines()
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn append(&self, thread_id: &str, message: &Message) -> std::io::Result<()> {
        std::fs::create_dir_all(self.paths.conversations_dir())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.paths.thread_file(thread_id))?;
        writeln!(file, "{}", serde_json::to_string(message)?)?;

        let mut threads = self.threads();
        if let Some(t) = threads.iter_mut().find(|t| t.id == thread_id) {
            t.updated_at = chrono::Utc::now().to_rfc3339();
            t.message_count += 1;
            if t.title == "Nueva conversación" && message.role == Role::User {
                t.title = title_from(&message.content);
            }
            if let Some(model) = &message.model_id {
                t.model_id = Some(model.clone());
            }
        }
        self.save_threads(&threads)
    }

    /// Reemplaza todos los mensajes de una conversación. Lo usan *regenerar*
    /// (que descarta la última respuesta) y *limpiar*.
    pub fn replace_messages(&self, thread_id: &str, messages: &[Message]) -> std::io::Result<()> {
        std::fs::create_dir_all(self.paths.conversations_dir())?;
        let body: String = messages
            .iter()
            .filter_map(|m| serde_json::to_string(m).ok())
            .map(|l| format!("{l}\n"))
            .collect();
        let tmp = self
            .paths
            .thread_file(thread_id)
            .with_extension("jsonl.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(tmp, self.paths.thread_file(thread_id))?;

        let mut threads = self.threads();
        if let Some(t) = threads.iter_mut().find(|t| t.id == thread_id) {
            t.message_count = messages.len() as u32;
            t.updated_at = chrono::Utc::now().to_rfc3339();
        }
        self.save_threads(&threads)
    }

    pub fn delete_thread(&self, thread_id: &str) -> std::io::Result<()> {
        let threads: Vec<Thread> = self
            .threads()
            .into_iter()
            .filter(|t| t.id != thread_id)
            .collect();
        let _ = std::fs::remove_file(self.paths.thread_file(thread_id));
        self.save_threads(&threads)
    }

    /// Quita el último turno del asistente, para regenerar la respuesta.
    /// Devuelve los mensajes que quedan.
    pub fn drop_last_answer(&self, thread_id: &str) -> std::io::Result<Vec<Message>> {
        let mut messages = self.messages(thread_id);
        while matches!(messages.last(), Some(m) if m.role == Role::Assistant) {
            messages.pop();
        }
        self.replace_messages(thread_id, &messages)?;
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ChatStore) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        (tmp, ChatStore::new(paths))
    }

    #[test]
    fn a_new_thread_is_listed_and_empty() {
        let (_t, store) = store();
        let thread = store.create_thread(Some("m".into())).unwrap();
        assert_eq!(store.threads().len(), 1);
        assert!(store.messages(&thread.id).is_empty());
        assert_eq!(
            store.get_thread(&thread.id).unwrap().model_id.as_deref(),
            Some("m")
        );
    }

    #[test]
    fn messages_are_appended_in_order() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "hola"))
            .unwrap();
        store
            .append(&thread.id, &Message::new(Role::Assistant, "qué tal"))
            .unwrap();
        let msgs = store.messages(&thread.id);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::User);
        assert_eq!(msgs[1].content, "qué tal");
        assert_eq!(store.get_thread(&thread.id).unwrap().message_count, 2);
    }

    #[test]
    fn the_first_user_message_names_the_thread() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(
                &thread.id,
                &Message::new(Role::User, "Resume el contrato de mantenimiento"),
            )
            .unwrap();
        assert_eq!(
            store.get_thread(&thread.id).unwrap().title,
            "Resume el contrato de mantenimiento"
        );
    }

    #[test]
    fn a_later_message_does_not_rename_the_thread() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "primero"))
            .unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "segundo"))
            .unwrap();
        assert_eq!(store.get_thread(&thread.id).unwrap().title, "primero");
    }

    #[test]
    fn regenerating_drops_only_the_trailing_answers() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "p1"))
            .unwrap();
        store
            .append(&thread.id, &Message::new(Role::Assistant, "r1"))
            .unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "p2"))
            .unwrap();
        store
            .append(&thread.id, &Message::new(Role::Assistant, "r2"))
            .unwrap();
        let left = store.drop_last_answer(&thread.id).unwrap();
        assert_eq!(left.len(), 3);
        assert_eq!(left.last().unwrap().content, "p2");
    }

    #[test]
    fn regenerating_with_no_answer_yet_changes_nothing() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "p1"))
            .unwrap();
        assert_eq!(store.drop_last_answer(&thread.id).unwrap().len(), 1);
    }

    #[test]
    fn clearing_empties_the_thread_but_keeps_it() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "x"))
            .unwrap();
        store.replace_messages(&thread.id, &[]).unwrap();
        assert!(store.messages(&thread.id).is_empty());
        assert_eq!(store.get_thread(&thread.id).unwrap().message_count, 0);
    }

    #[test]
    fn deleting_removes_the_thread_and_its_file() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "x"))
            .unwrap();
        store.delete_thread(&thread.id).unwrap();
        assert!(store.threads().is_empty());
        assert!(store.messages(&thread.id).is_empty());
    }

    #[test]
    fn threads_are_listed_newest_first() {
        let (_t, store) = store();
        let a = store.create_thread(None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = store.create_thread(None).unwrap();
        store.append(&b.id, &Message::new(Role::User, "x")).unwrap();
        let listed = store.threads();
        assert_eq!(listed[0].id, b.id);
        assert_eq!(listed[1].id, a.id);
    }

    #[test]
    fn a_corrupt_line_does_not_lose_the_rest_of_the_conversation() {
        let (_t, store) = store();
        let thread = store.create_thread(None).unwrap();
        store
            .append(&thread.id, &Message::new(Role::User, "bueno"))
            .unwrap();
        let file = store.paths.thread_file(&thread.id);
        let mut raw = std::fs::read_to_string(&file).unwrap();
        raw.push_str("{ línea rota\n");
        std::fs::write(&file, raw).unwrap();
        assert_eq!(store.messages(&thread.id).len(), 1);
    }

    #[test]
    fn titles_are_trimmed_at_a_word_boundary() {
        assert_eq!(title_from("   "), "Nueva conversación");
        assert_eq!(title_from("corto"), "corto");
        let long =
            "Necesito un resumen ejecutivo del informe de sostenibilidad del ejercicio anterior";
        let title = title_from(long);
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= 49);
        assert!(!title.contains("  "));
    }

    #[test]
    fn a_message_records_whether_it_was_generated_locally() {
        let mut m = Message::new(Role::Assistant, "hola");
        assert!(!m.local, "por defecto no se afirma nada");
        m.local = true;
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), m);
    }
}
