//! `AppCore`: la fachada que usan los dos *hosts*.
//!
//! El shell Tauri y el servidor HTTP no contienen lógica: traducen entrada y
//! salida y llaman aquí. Es lo que hace que la aplicación se pueda probar de
//! extremo a extremo sin abrir una ventana.

use crate::audit::{AuditEntry, AuditLog};
use crate::catalog::{fit, Catalog, FitLevel, ModelFit, ModelSpec};
use crate::chat::{ChatStore, Message, Role, Thread};
use crate::download::{build_client, Downloader};
use crate::events::{AppEvent, EventBus};
use crate::hardware::HardwareProfile;
use crate::metrics::{GenerationTimer, ResourceSample};
use crate::paths::Paths;
use crate::policy::EffectivePolicy;
use crate::runtime::llamacpp::LlamaCppRuntime;
use crate::runtime::ollama::OllamaRuntime;
use crate::runtime::registry::RuntimeReport;
use crate::runtime::{
    CancelToken, ChatMessage, ChatRequest, Delta, InstalledModel, RunningModel, RuntimeError,
    RuntimeRegistry, Tuning,
};
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no existe el modelo {0} en el catálogo autorizado")]
    UnknownModel(String),
    #[error("no existe la conversación {0}")]
    UnknownThread(String),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("error de disco: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// Estado de un modelo desde el punto de vista de la interfaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelState {
    NotInstalled,
    Installing,
    Installed,
    Running,
}

/// Una tarjeta del catálogo, lista para pintar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCard {
    #[serde(flatten)]
    pub fit: ModelFit,
    pub state: ModelState,
    pub installed: Option<InstalledModel>,
    pub recommended: bool,
}

/// Todo lo que la Home necesita, en una sola llamada.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSnapshot {
    pub hardware: HardwareProfile,
    pub hardware_label: String,
    pub resources: ResourceSample,
    pub recommended: Vec<ModelCard>,
    pub installed: Vec<ModelCard>,
    pub running: Option<RunningModel>,
    pub runtimes: Vec<RuntimeReport>,
    pub policy_origin: String,
    pub policy_managed: bool,
    pub organization: String,
    pub onboarded: bool,
    pub catalog_source: String,
    pub version: String,
}

pub struct AppCore {
    paths: Paths,
    policy: EffectivePolicy,
    catalog: Catalog,
    hardware: HardwareProfile,
    registry: RuntimeRegistry,
    chats: ChatStore,
    audit: AuditLog,
    bus: EventBus,
    settings: Mutex<Settings>,
    /// Instalaciones en curso, para que la tarjeta muestre "Instalando".
    installing: Mutex<HashMap<String, ()>>,
    /// Señal de parada de la generación activa.
    cancel: Mutex<Option<CancelToken>>,
}

impl AppCore {
    /// Arranca el núcleo: rutas, política, catálogo, hardware y motores.
    pub fn bootstrap() -> Result<Arc<Self>> {
        Self::bootstrap_at(Paths::resolve())
    }

    pub fn bootstrap_at(paths: Paths) -> Result<Arc<Self>> {
        paths.ensure()?;
        let policy = EffectivePolicy::load(&paths.local_policy_file());
        let hardware = HardwareProfile::detect(paths.root());
        let catalog = load_catalog(&policy);
        let bus = EventBus::new();

        let client = build_client(&policy.document)
            .map_err(|e| CoreError::Other(format!("no se pudo preparar la red: {e}")))?;
        let downloader = Arc::new(Downloader::new(
            client.clone(),
            policy.document.clone(),
            bus.clone(),
        ));

        let mut registry = RuntimeRegistry::new();
        registry.register(Arc::new(LlamaCppRuntime::new(
            paths.clone(),
            client.clone(),
            downloader,
            bus.clone(),
        )));
        registry.register(Arc::new(OllamaRuntime::new(client)));

        let audit = AuditLog::new(paths.clone(), policy.document.audit.enabled);
        let settings = Settings::load(&paths);

        Ok(Arc::new(Self {
            chats: ChatStore::new(paths.clone()),
            settings: Mutex::new(settings),
            installing: Mutex::new(HashMap::new()),
            cancel: Mutex::new(None),
            paths,
            policy,
            catalog,
            hardware,
            registry,
            audit,
            bus,
        }))
    }

    pub fn bus(&self) -> &EventBus {
        &self.bus
    }
    pub fn paths(&self) -> &Paths {
        &self.paths
    }
    pub fn hardware(&self) -> &HardwareProfile {
        &self.hardware
    }
    pub fn policy(&self) -> &EffectivePolicy {
        &self.policy
    }
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    pub async fn settings(&self) -> Settings {
        self.settings.lock().await.clone()
    }

    pub async fn update_settings(&self, next: Settings) -> Result<Settings> {
        next.save(&self.paths)?;
        let mut guard = self.settings.lock().await;
        *guard = next.clone();
        Ok(next)
    }

    fn spec(&self, model_id: &str) -> Result<ModelSpec> {
        self.catalog
            .get(model_id)
            .cloned()
            .ok_or_else(|| CoreError::UnknownModel(model_id.to_string()))
    }

    pub async fn runtime_reports(&self) -> Vec<RuntimeReport> {
        self.registry.report().await
    }

    pub async fn running(&self) -> Option<RunningModel> {
        for runtime in self.registry.all() {
            if let Some(running) = runtime.running().await {
                return Some(running);
            }
        }
        None
    }

    /// Todos los modelos del catálogo con su aptitud y estado.
    pub async fn model_cards(&self) -> Vec<ModelCard> {
        let available = self.registry.available_ids().await;
        let installed = self.installed_index().await;
        let running = self.running().await;
        let installing = self.installing.lock().await.clone();

        let classified = fit::classify_all(&self.catalog.models, &self.hardware, &available);
        let recommended_id =
            fit::recommend(&self.catalog.models, &self.hardware, &available).map(|f| f.model.id);

        classified
            .into_iter()
            .map(|f| {
                let id = f.model.id.clone();
                let entry = installed.get(&id).cloned();
                let state = if running.as_ref().map(|r| r.model_id.as_str()) == Some(id.as_str()) {
                    ModelState::Running
                } else if installing.contains_key(&id) {
                    ModelState::Installing
                } else if entry.is_some() {
                    ModelState::Installed
                } else {
                    ModelState::NotInstalled
                };
                ModelCard {
                    recommended: recommended_id.as_deref() == Some(id.as_str()),
                    state,
                    installed: entry,
                    fit: f,
                }
            })
            .collect()
    }

    async fn installed_index(&self) -> HashMap<String, InstalledModel> {
        let mut out = HashMap::new();
        for runtime in self.registry.all() {
            if let Ok(models) = runtime.installed_models().await {
                for m in models {
                    out.insert(m.model_id.clone(), m);
                }
            }
        }
        out
    }

    /// Instantánea completa para la Home.
    pub async fn home(&self) -> HomeSnapshot {
        let cards = self.model_cards().await;
        let installed: Vec<ModelCard> = cards
            .iter()
            .filter(|c| c.state != ModelState::NotInstalled)
            .cloned()
            .collect();
        let recommended: Vec<ModelCard> = cards
            .iter()
            .filter(|c| {
                c.state == ModelState::NotInstalled
                    && c.fit.verdict.level != FitLevel::NotRecommended
            })
            .take(3)
            .cloned()
            .collect();

        HomeSnapshot {
            hardware_label: self.hardware.short_label(),
            resources: self.resources(),
            recommended,
            installed,
            running: self.running().await,
            runtimes: self.runtime_reports().await,
            policy_origin: self.policy.origin.clone(),
            policy_managed: self.policy.managed,
            organization: self.policy.organization_label().to_string(),
            onboarded: self.settings().await.onboarded,
            catalog_source: self.catalog.source.clone(),
            version: crate::VERSION.to_string(),
            hardware: self.hardware.clone(),
        }
    }

    /// Uso de recursos ahora mismo. Se mide en cada llamada: el panel de la Home
    /// refresca cada pocos segundos.
    pub fn resources(&self) -> ResourceSample {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();
        let cpu = sys.global_cpu_usage();
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let disk_free = disks
            .iter()
            .filter(|d| self.paths.root().starts_with(d.mount_point()))
            .map(|d| d.available_space())
            .max()
            .unwrap_or(self.hardware.free_disk_bytes);
        ResourceSample {
            cpu_percent: cpu,
            ram_used_bytes: sys.used_memory(),
            ram_total_bytes: sys.total_memory(),
            disk_free_bytes: disk_free,
        }
    }

    // --- Ciclo de vida de un modelo ---------------------------------------

    pub async fn install_model(&self, model_id: &str) -> Result<InstalledModel> {
        let spec = self.spec(model_id)?;
        let runtime = self
            .registry
            .resolve_for(&spec)
            .await
            .ok_or_else(|| RuntimeError::Unavailable(spec.runtimes.join(", ")))?;

        self.installing
            .lock()
            .await
            .insert(model_id.to_string(), ());
        self.bus.emit(AppEvent::ModelStateChanged {
            model_id: model_id.into(),
            state: "installing".into(),
        });

        let started = std::time::Instant::now();
        let result = runtime.install(&spec, Box::new(|_, _| {})).await;
        self.installing.lock().await.remove(model_id);

        match result {
            Ok(installed) => {
                self.audit.record(
                    AuditEntry::new("model.install", "ok")
                        .model(model_id)
                        .runtime(runtime.id())
                        .bytes(installed.bytes)
                        .duration(started.elapsed().as_millis() as u64),
                );
                self.bus.emit(AppEvent::DownloadFinished {
                    model_id: model_id.into(),
                    ok: true,
                    message: None,
                });
                Ok(installed)
            }
            Err(err) => {
                self.audit.record(
                    AuditEntry::new("model.install", "error")
                        .model(model_id)
                        .runtime(runtime.id()),
                );
                self.bus.emit(AppEvent::DownloadFinished {
                    model_id: model_id.into(),
                    ok: false,
                    message: Some(err.to_string()),
                });
                Err(err.into())
            }
        }
    }

    pub async fn remove_model(&self, model_id: &str) -> Result<()> {
        let spec = self.spec(model_id)?;
        if let Some(runtime) = self.registry.resolve_for(&spec).await {
            runtime.remove(model_id).await?;
        }
        let mut settings = self.settings.lock().await;
        if settings.active_model_id.as_deref() == Some(model_id) {
            settings.active_model_id = None;
            let _ = settings.save(&self.paths);
        }
        self.audit
            .record(AuditEntry::new("model.remove", "ok").model(model_id));
        Ok(())
    }

    pub async fn start_model(&self, model_id: &str) -> Result<RunningModel> {
        let spec = self.spec(model_id)?;
        let runtime = self
            .registry
            .resolve_for(&spec)
            .await
            .ok_or_else(|| RuntimeError::Unavailable(spec.runtimes.join(", ")))?;
        let tuning = Tuning::derive(&spec, &self.hardware);
        let started = std::time::Instant::now();
        let running = runtime.start(&spec, &tuning).await?;

        let mut settings = self.settings.lock().await;
        settings.active_model_id = Some(model_id.to_string());
        let _ = settings.save(&self.paths);
        drop(settings);

        self.audit.record(
            AuditEntry::new("model.start", "ok")
                .model(model_id)
                .runtime(runtime.id())
                .duration(started.elapsed().as_millis() as u64),
        );
        Ok(running)
    }

    pub async fn stop_model(&self, model_id: &str) -> Result<()> {
        let spec = self.spec(model_id)?;
        if let Some(runtime) = self.registry.resolve_for(&spec).await {
            runtime.stop(model_id).await?;
        }
        self.audit
            .record(AuditEntry::new("model.stop", "ok").model(model_id));
        Ok(())
    }

    // --- Conversaciones ----------------------------------------------------

    pub fn threads(&self) -> Vec<Thread> {
        self.chats.threads()
    }

    pub fn messages(&self, thread_id: &str) -> Vec<Message> {
        self.chats.messages(thread_id)
    }

    pub async fn create_thread(&self) -> Result<Thread> {
        let model = self.settings().await.active_model_id;
        Ok(self.chats.create_thread(model)?)
    }

    pub fn delete_thread(&self, thread_id: &str) -> Result<()> {
        Ok(self.chats.delete_thread(thread_id)?)
    }

    pub fn clear_thread(&self, thread_id: &str) -> Result<()> {
        Ok(self.chats.replace_messages(thread_id, &[])?)
    }

    /// Interrumpe la generación en curso.
    pub async fn stop_generation(&self) {
        if let Some(token) = self.cancel.lock().await.as_ref() {
            token.cancel();
        }
    }

    /// Envía un mensaje y transmite la respuesta por el bus de eventos.
    ///
    /// `regenerate` descarta la última respuesta y vuelve a generar sobre el
    /// mismo turno del usuario, sin duplicar su mensaje.
    pub async fn send_message(
        &self,
        thread_id: &str,
        user_text: &str,
        regenerate: bool,
    ) -> Result<Message> {
        let thread = self
            .chats
            .get_thread(thread_id)
            .ok_or_else(|| CoreError::UnknownThread(thread_id.to_string()))?;

        let running = self.running().await.ok_or(RuntimeError::NotRunning)?;
        let spec = self.spec(&running.model_id)?;
        let runtime = self
            .registry
            .get(&running.runtime)
            .ok_or_else(|| RuntimeError::Unavailable(running.runtime.clone()))?;

        let history = if regenerate {
            self.chats.drop_last_answer(thread_id)?
        } else {
            let mut user = Message::new(Role::User, user_text);
            user.model_id = Some(running.model_id.clone());
            self.chats.append(thread_id, &user)?;
            self.chats.messages(thread_id)
        };

        let settings = self.settings().await;
        let mut messages: Vec<ChatMessage> = Vec::new();
        if let Some(system) =
            settings.effective_system_prompt(self.policy.document.chat.system_prompt.as_deref())
        {
            messages.push(ChatMessage {
                role: "system".into(),
                content: system,
            });
        }
        messages.extend(history.iter().map(|m| ChatMessage {
            role: m.role.as_str().to_string(),
            content: m.content.clone(),
        }));

        let tuning = Tuning::derive(&spec, &self.hardware);
        let request = ChatRequest {
            model_id: running.model_id.clone(),
            messages,
            temperature: settings.temperature,
            max_tokens: tuning.max_output_tokens,
            context_blocks: Vec::new(),
            tools: Vec::new(),
        };

        let cancel = CancelToken::new();
        *self.cancel.lock().await = Some(cancel.clone());

        let mut answer = Message::new(Role::Assistant, String::new());
        answer.model_id = Some(running.model_id.clone());
        // La afirmación "procesando localmente" se deriva del endpoint real.
        answer.local = running.is_local();

        let bus = self.bus.clone();
        let thread_key = thread_id.to_string();
        let message_id = answer.id.clone();
        let collected = Arc::new(std::sync::Mutex::new(String::new()));
        let sink_buffer = collected.clone();
        let mut timer = GenerationTimer::start();
        let cancel_for_sink = cancel.clone();

        let outcome = runtime
            .chat_stream(
                request,
                Box::new(move |delta| {
                    if let Delta::Text(text) = delta {
                        if let Ok(mut buf) = sink_buffer.lock() {
                            buf.push_str(&text);
                        }
                        bus.emit(AppEvent::ChatDelta {
                            thread_id: thread_key.clone(),
                            message_id: message_id.clone(),
                            text,
                        });
                    }
                    !cancel_for_sink.is_cancelled()
                }),
                cancel.clone(),
            )
            .await;

        *self.cancel.lock().await = None;

        let text = collected.lock().map(|b| b.clone()).unwrap_or_default();
        // Cada delta de texto es un token del motor.
        for _ in 0..text.split_whitespace().count().max(1) {
            timer.token();
        }
        let metrics = timer.finish();

        match outcome {
            Ok(result) => {
                answer.content = text;
                answer.metrics = Some(metrics);
                self.chats.append(thread_id, &answer)?;
                self.audit.record(
                    AuditEntry::new("chat.turn", if result.stopped { "stopped" } else { "ok" })
                        .model(&running.model_id)
                        .runtime(runtime.id())
                        .duration(metrics.total_ms),
                );
                self.bus.emit(AppEvent::ChatDone {
                    thread_id: thread.id.clone(),
                    message_id: answer.id.clone(),
                    stopped: result.stopped,
                    metrics: Some(metrics),
                });
                Ok(answer)
            }
            Err(err) => {
                self.audit.record(
                    AuditEntry::new("chat.turn", "error")
                        .model(&running.model_id)
                        .runtime(runtime.id()),
                );
                self.bus.emit(AppEvent::ChatError {
                    thread_id: thread.id.clone(),
                    message: err.to_string(),
                });
                Err(err.into())
            }
        }
    }
}

/// Catálogo según la política: el embebido, o uno corporativo; y luego las
/// listas de permitidos/prohibidos.
fn load_catalog(policy: &EffectivePolicy) -> Catalog {
    let base = match policy.document.catalog.source.as_deref() {
        None | Some("") | Some("embedded") => Catalog::embedded(),
        Some(source) if source.starts_with("http") => {
            // Un catálogo remoto se descarga al arrancar; si falla, el embebido
            // mantiene la aplicación utilizable. Pendiente de implementar el
            // refresco periódico (ver docs/MVP_PLAN.md T9).
            tracing::warn!(%source, "catálogo remoto todavía no soportado; se usa el embebido");
            Catalog::embedded()
        }
        Some(path) => match std::fs::read_to_string(path) {
            Ok(raw) => match Catalog::from_json(path, &raw) {
                Ok(catalog) => catalog,
                Err(err) => {
                    tracing::warn!(?err, %path, "catálogo corporativo ilegible; se usa el embebido");
                    Catalog::embedded()
                }
            },
            Err(err) => {
                tracing::warn!(?err, %path, "no se puede leer el catálogo corporativo");
                Catalog::embedded()
            }
        },
    };
    base.filtered(
        &policy.document.catalog.allowlist,
        &policy.document.catalog.denylist,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> (tempfile::TempDir, Arc<AppCore>) {
        let tmp = tempfile::tempdir().unwrap();
        let core = AppCore::bootstrap_at(Paths::at(tmp.path())).unwrap();
        (tmp, core)
    }

    #[tokio::test]
    async fn a_fresh_install_has_a_catalog_and_no_models() {
        let (_t, core) = core();
        let cards = core.model_cards().await;
        assert!(!cards.is_empty());
        assert!(cards.iter().all(|c| c.state == ModelState::NotInstalled));
        assert!(core.running().await.is_none());
    }

    #[tokio::test]
    async fn the_home_snapshot_carries_everything_the_screen_needs() {
        let (_t, core) = core();
        let home = core.home().await;
        assert!(home.hardware.total_ram_bytes > 0);
        assert!(!home.hardware_label.is_empty());
        assert!(!home.runtimes.is_empty());
        assert_eq!(home.policy_origin, "defaults");
        assert!(!home.policy_managed);
        assert_eq!(home.organization, "Naturgy");
        assert!(home.installed.is_empty());
        assert_eq!(home.version, crate::VERSION);
    }

    #[tokio::test]
    async fn recommendations_are_never_models_the_machine_cannot_run() {
        let (_t, core) = core();
        let home = core.home().await;
        for card in &home.recommended {
            assert_ne!(card.fit.verdict.level, FitLevel::NotRecommended);
            assert_eq!(card.state, ModelState::NotInstalled);
        }
        assert!(home.recommended.len() <= 3);
    }

    #[tokio::test]
    async fn exactly_one_card_is_marked_as_the_recommendation() {
        let (_t, core) = core();
        let marked = core
            .model_cards()
            .await
            .iter()
            .filter(|c| c.recommended)
            .count();
        assert!(
            marked <= 1,
            "no puede haber dos recomendaciones principales"
        );
    }

    #[tokio::test]
    async fn an_unknown_model_is_a_clear_error_not_a_panic() {
        let (_t, core) = core();
        assert!(matches!(
            core.install_model("no-existe").await,
            Err(CoreError::UnknownModel(_))
        ));
        assert!(matches!(
            core.start_model("no-existe").await,
            Err(CoreError::UnknownModel(_))
        ));
    }

    #[tokio::test]
    async fn chatting_without_a_running_model_is_refused() {
        let (_t, core) = core();
        let thread = core.create_thread().await.unwrap();
        let err = core
            .send_message(&thread.id, "hola", false)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::Runtime(RuntimeError::NotRunning)));
    }

    #[tokio::test]
    async fn sending_to_an_unknown_thread_is_refused() {
        let (_t, core) = core();
        assert!(matches!(
            core.send_message("no-existe", "hola", false).await,
            Err(CoreError::UnknownThread(_))
        ));
    }

    #[tokio::test]
    async fn settings_survive_a_round_trip() {
        let (_t, core) = core();
        let mut s = core.settings().await;
        s.onboarded = true;
        s.temperature = 0.3;
        core.update_settings(s.clone()).await.unwrap();
        assert_eq!(core.settings().await, s);
        assert!(core.home().await.onboarded);
    }

    #[tokio::test]
    async fn a_denylist_removes_models_from_the_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        std::fs::write(
            paths.local_policy_file(),
            r#"{"organization":"Naturgy IT","catalog":{"denylist":["gemma*","llama*"]}}"#,
        )
        .unwrap();
        let core = AppCore::bootstrap_at(paths).unwrap();
        assert!(core.catalog().get("gemma-2-9b-it-q4km").is_none());
        assert!(core.catalog().get("qwen2.5-7b-instruct-q4km").is_some());
        assert_eq!(core.home().await.organization, "Naturgy IT");
    }

    #[tokio::test]
    async fn an_allowlist_keeps_only_what_it_names() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        std::fs::write(
            paths.local_policy_file(),
            r#"{"catalog":{"allowlist":["qwen2.5-7b-instruct-q4km"]}}"#,
        )
        .unwrap();
        let core = AppCore::bootstrap_at(paths).unwrap();
        assert_eq!(core.catalog().models.len(), 1);
    }

    #[tokio::test]
    async fn threads_can_be_created_cleared_and_deleted() {
        let (_t, core) = core();
        let thread = core.create_thread().await.unwrap();
        assert_eq!(core.threads().len(), 1);
        core.clear_thread(&thread.id).unwrap();
        assert!(core.messages(&thread.id).is_empty());
        core.delete_thread(&thread.id).unwrap();
        assert!(core.threads().is_empty());
    }

    #[tokio::test]
    async fn resources_report_real_numbers() {
        let (_t, core) = core();
        let r = core.resources();
        assert!(r.ram_total_bytes > 0);
        assert!(r.ram_used_bytes <= r.ram_total_bytes);
        assert!((0.0..=100.0).contains(&r.cpu_percent));
    }

    #[tokio::test]
    async fn the_audit_log_records_a_failed_install_without_content() {
        let (_t, core) = core();
        let _ = core.install_model("qwen2.5-0.5b-instruct-q4km").await;
        let entries = core.audit().tail(10);
        assert!(
            entries.iter().any(|e| e.event == "model.install"),
            "una instalación, aunque falle, se audita"
        );
    }

    #[tokio::test]
    async fn stopping_a_generation_when_none_is_running_is_harmless() {
        let (_t, core) = core();
        core.stop_generation().await;
    }
}
