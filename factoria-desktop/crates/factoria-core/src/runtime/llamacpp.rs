//! Adaptador de **llama.cpp**: el motor primario de FactorIA Desktop.
//!
//! Es el motor primario porque se puede empaquetar y *pinnear*: el empleado no
//! tiene que instalar nada por su cuenta. Ollama, en cambio, exige una
//! instalación previa que en un parque corporativo no está garantizada.
//!
//! Ciclo de vida: `llama-server` se lanza como **proceso hijo** escuchando en
//! `127.0.0.1` con un puerto efímero, se espera a `/health`, y se habla con él
//! por el API compatible con OpenAI. Enfoque tomado de Rebost
//! (`src-tauri/src/engine/`), con la diferencia de que aquí vive detrás del
//! `trait LlmRuntime`.

use super::sse::{SseParser, StreamEvent};
use super::{
    CancelToken, ChatOutcome, ChatRequest, InstalledModel, LlmRuntime, ProgressSink, Result,
    RunningModel, RuntimeDescriptor, RuntimeError, RuntimeStatus, TokenSink, Tuning,
};
use crate::catalog::{ModelSource, ModelSpec};
use crate::download::Downloader;
use crate::events::{AppEvent, EventBus};
use crate::paths::Paths;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub const RUNTIME_ID: &str = "llamacpp";

/// Cuánto se espera a que el motor cargue los pesos y responda `/health`.
/// Un modelo de 9 GB en un disco lento tarda más de un minuto la primera vez.
const READY_TIMEOUT: Duration = Duration::from_secs(180);
const HEALTH_POLL: Duration = Duration::from_millis(250);

/// Dónde obtener el binario del motor para este sistema.
///
/// Las URLs apuntan a las *releases* oficiales de llama.cpp. En un despliegue
/// corporativo, `catalog.mirror_base_url` las redirige al artefactorio interno.
#[derive(Debug, Clone)]
pub struct EnginePin {
    pub os: &'static str,
    pub arch: &'static str,
    pub accelerator: &'static str,
    pub url: &'static str,
    pub mirror_path: &'static str,
    pub archive: ArchiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

/// Versión de llama.cpp con la que FactorIA está probada. Fijar la versión es
/// deliberado: una actualización del motor puede cambiar los formatos aceptados.
pub const ENGINE_RELEASE: &str = "b4585";

pub fn engine_pins() -> Vec<EnginePin> {
    vec![
        EnginePin {
            os: "macos",
            arch: "aarch64",
            accelerator: "Metal",
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b4585/llama-b4585-bin-macos-arm64.zip",
            mirror_path: "llama.cpp/b4585/llama-b4585-bin-macos-arm64.zip",
            archive: ArchiveKind::Zip,
        },
        EnginePin {
            os: "macos",
            arch: "x86_64",
            accelerator: "Metal",
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b4585/llama-b4585-bin-macos-x64.zip",
            mirror_path: "llama.cpp/b4585/llama-b4585-bin-macos-x64.zip",
            archive: ArchiveKind::Zip,
        },
        EnginePin {
            os: "windows",
            arch: "x86_64",
            accelerator: "Vulkan",
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b4585/llama-b4585-bin-win-vulkan-x64.zip",
            mirror_path: "llama.cpp/b4585/llama-b4585-bin-win-vulkan-x64.zip",
            archive: ArchiveKind::Zip,
        },
        EnginePin {
            os: "windows",
            arch: "aarch64",
            accelerator: "CPU",
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b4585/llama-b4585-bin-win-cpu-arm64.zip",
            mirror_path: "llama.cpp/b4585/llama-b4585-bin-win-cpu-arm64.zip",
            archive: ArchiveKind::Zip,
        },
        EnginePin {
            os: "linux",
            arch: "x86_64",
            accelerator: "Vulkan",
            url: "https://github.com/ggml-org/llama.cpp/releases/download/b4585/llama-b4585-bin-ubuntu-vulkan-x64.zip",
            mirror_path: "llama.cpp/b4585/llama-b4585-bin-ubuntu-vulkan-x64.zip",
            archive: ArchiveKind::Zip,
        },
    ]
}

/// El *pin* que corresponde a esta máquina, si lo hay.
pub fn pin_for_host() -> Option<EnginePin> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    engine_pins()
        .into_iter()
        .find(|p| p.os == os && p.arch == arch)
}

/// Registro en disco de los modelos instalados para este motor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct Registry(HashMap<String, InstalledModel>);

fn load_registry(paths: &Paths) -> Registry {
    std::fs::read_to_string(paths.models_registry())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_registry(paths: &Paths, registry: &Registry) -> std::io::Result<()> {
    std::fs::create_dir_all(paths.models_dir())?;
    let json = serde_json::to_string_pretty(registry)?;
    // Escritura atómica: un corte de luz no debe dejar el registro a medias.
    let tmp = paths.models_registry().with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, paths.models_registry())
}

struct Child {
    model_id: String,
    process: tokio::process::Child,
    running: RunningModel,
}

pub struct LlamaCppRuntime {
    paths: Paths,
    client: reqwest::Client,
    downloader: Arc<Downloader>,
    bus: EventBus,
    child: Arc<Mutex<Option<Child>>>,
}

impl LlamaCppRuntime {
    pub fn new(
        paths: Paths,
        client: reqwest::Client,
        downloader: Arc<Downloader>,
        bus: EventBus,
    ) -> Self {
        Self {
            paths,
            client,
            downloader,
            bus,
            child: Arc::new(Mutex::new(None)),
        }
    }

    fn model_path(&self, model_id: &str) -> PathBuf {
        self.paths.models_dir().join(format!("{model_id}.gguf"))
    }

    /// Ruta del ejecutable `llama-server`.
    ///
    /// `FACTORIA_LLAMA_SERVER_BIN` tiene prioridad: es lo que permite un
    /// despliegue aislado con el binario ya colocado por el paquete corporativo,
    /// y lo que usan las pruebas de extremo a extremo.
    pub fn engine_binary(&self) -> Option<PathBuf> {
        if let Some(explicit) = std::env::var_os("FACTORIA_LLAMA_SERVER_BIN") {
            let path = PathBuf::from(explicit);
            if path.is_file() {
                return Some(path);
            }
            tracing::warn!(path = %path.display(), "FACTORIA_LLAMA_SERVER_BIN no apunta a un fichero");
            return None;
        }
        let dir = self.paths.engine_dir(RUNTIME_ID, ENGINE_RELEASE);
        let name = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        // El archivo oficial mete los binarios en `build/bin/`.
        [dir.join(name), dir.join("build").join("bin").join(name)]
            .into_iter()
            .find(|candidate| candidate.is_file())
    }

    /// Puerto libre en *loopback*. Pedir el 0 al sistema y quedarse con el que
    /// asigne evita chocar con otra aplicación del empleado.
    fn free_port() -> std::io::Result<u16> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        Ok(port)
    }

    async fn wait_until_ready(&self, endpoint: &str) -> Result<()> {
        let deadline = std::time::Instant::now() + READY_TIMEOUT;
        let health = format!("{endpoint}/health");
        while std::time::Instant::now() < deadline {
            if let Ok(res) = self.client.get(&health).send().await {
                if res.status().is_success() {
                    return Ok(());
                }
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
        Err(RuntimeError::StartTimeout)
    }
}

#[async_trait]
impl LlmRuntime for LlamaCppRuntime {
    fn descriptor(&self) -> RuntimeDescriptor {
        RuntimeDescriptor {
            id: RUNTIME_ID.into(),
            name: "Motor local FactorIA".into(),
            description: "Ejecuta modelos GGUF en este equipo. No requiere instalar nada más."
                .into(),
            self_installable: true,
        }
    }

    async fn probe(&self) -> RuntimeStatus {
        if self.engine_binary().is_some() {
            return RuntimeStatus::Ready;
        }
        match pin_for_host() {
            Some(_) => RuntimeStatus::NeedsSetup {
                detail: "El motor se descargará la primera vez que ejecutes un modelo.".into(),
            },
            None => RuntimeStatus::Unavailable {
                detail: format!(
                    "No hay motor para {} {}.",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            },
        }
    }

    async fn installed_models(&self) -> Result<Vec<InstalledModel>> {
        let registry = load_registry(&self.paths);
        Ok(registry
            .0
            .into_values()
            // Un registro puede quedar desfasado si alguien borra el fichero a
            // mano: se comprueba el disco, que es la verdad.
            .filter(|m| {
                m.path
                    .as_ref()
                    .map(|p| std::path::Path::new(p).is_file())
                    .unwrap_or(false)
            })
            .collect())
    }

    async fn install(
        &self,
        spec: &ModelSpec,
        mut progress: ProgressSink,
    ) -> Result<InstalledModel> {
        let ModelSource::GgufUrl {
            url,
            sha256,
            mirror_path,
        } = &spec.source
        else {
            return Err(RuntimeError::Engine(
                "este modelo no se distribuye como GGUF".into(),
            ));
        };

        let dest = self.model_path(&spec.id);
        let mut rx = self.bus.subscribe();
        let model_id = spec.id.clone();
        // El progreso del descargador llega por el bus; se reenvía al llamante.
        let pump = tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if let AppEvent::DownloadProgress {
                    model_id: id,
                    received_bytes,
                    total_bytes,
                    ..
                } = event
                {
                    if id == model_id {
                        progress(received_bytes, total_bytes);
                    }
                }
            }
        });

        let outcome = self
            .downloader
            .fetch(
                &spec.id,
                url,
                mirror_path.as_deref(),
                sha256.as_deref(),
                &dest,
            )
            .await;
        pump.abort();
        let outcome = outcome?;

        // Un GGUF que no se puede leer no llega a registrarse: mejor un error
        // claro ahora que un motor que no arranca dentro de tres minutos.
        let header = match crate::gguf::is_loadable(&dest) {
            Ok(h) => h,
            Err(err) => {
                let _ = std::fs::remove_file(&dest);
                return Err(RuntimeError::Gguf(err));
            }
        };

        let installed = InstalledModel {
            model_id: spec.id.clone(),
            runtime: RUNTIME_ID.into(),
            path: Some(dest.to_string_lossy().into_owned()),
            bytes: outcome.bytes,
            installed_at: chrono::Utc::now().to_rfc3339(),
            sha256: Some(outcome.sha256),
            integrity: format!("{:?}", outcome.integrity).to_lowercase(),
            context_window: header.context_length.unwrap_or(spec.context_window),
        };

        let mut registry = load_registry(&self.paths);
        registry.0.insert(spec.id.clone(), installed.clone());
        save_registry(&self.paths, &registry)?;

        self.bus.emit(AppEvent::ModelStateChanged {
            model_id: spec.id.clone(),
            state: "installed".into(),
        });
        Ok(installed)
    }

    async fn remove(&self, model_id: &str) -> Result<()> {
        // Detener antes de borrar: en Windows no se puede borrar un fichero que
        // el motor tiene abierto.
        let _ = self.stop(model_id).await;
        let mut registry = load_registry(&self.paths);
        if let Some(entry) = registry.0.remove(model_id) {
            if let Some(path) = entry.path {
                let _ = std::fs::remove_file(path);
            }
        }
        let _ = std::fs::remove_file(self.model_path(model_id));
        save_registry(&self.paths, &registry)?;
        self.bus.emit(AppEvent::ModelStateChanged {
            model_id: model_id.into(),
            state: "removed".into(),
        });
        Ok(())
    }

    async fn start(&self, spec: &ModelSpec, tuning: &Tuning) -> Result<RunningModel> {
        let registry = load_registry(&self.paths);
        let entry = registry
            .0
            .get(&spec.id)
            .ok_or_else(|| RuntimeError::NotInstalled(spec.id.clone()))?;
        let model_path = entry
            .path
            .clone()
            .unwrap_or_else(|| self.model_path(&spec.id).to_string_lossy().into_owned());
        if !std::path::Path::new(&model_path).is_file() {
            return Err(RuntimeError::NotInstalled(spec.id.clone()));
        }

        let binary = self.engine_binary().ok_or_else(|| {
            RuntimeError::Unavailable("motor local (no está preparado todavía)".into())
        })?;

        // Un solo modelo a la vez: cargar dos llenaría la memoria del equipo.
        {
            let mut guard = self.child.lock().await;
            if let Some(mut existing) = guard.take() {
                let _ = existing.process.kill().await;
            }
        }

        let port = Self::free_port()?;
        let endpoint = format!("http://127.0.0.1:{port}");
        let args = tuning.llama_server_args(&model_path, port);

        tracing::info!(model = %spec.id, port, "arrancando el motor local");
        let mut command = tokio::process::Command::new(&binary);
        command.args(&args).kill_on_drop(true);
        if let Some(dir) = binary.parent() {
            // El directorio del motor es el de trabajo para que Windows
            // encuentre las DLL que acompañan al ejecutable.
            command.current_dir(dir);
        }
        let process = command
            .spawn()
            .map_err(|e| RuntimeError::Engine(format!("no se pudo lanzar el motor: {e}")))?;

        let running = RunningModel {
            model_id: spec.id.clone(),
            runtime: RUNTIME_ID.into(),
            endpoint: endpoint.clone(),
            context_tokens: tuning.context_tokens,
            started_at: chrono::Utc::now().to_rfc3339(),
        };

        {
            let mut guard = self.child.lock().await;
            *guard = Some(Child {
                model_id: spec.id.clone(),
                process,
                running: running.clone(),
            });
        }

        self.bus.emit(AppEvent::ModelStateChanged {
            model_id: spec.id.clone(),
            state: "starting".into(),
        });

        if let Err(err) = self.wait_until_ready(&endpoint).await {
            let mut guard = self.child.lock().await;
            if let Some(mut child) = guard.take() {
                let _ = child.process.kill().await;
            }
            self.bus.emit(AppEvent::ModelStateChanged {
                model_id: spec.id.clone(),
                state: "error".into(),
            });
            return Err(err);
        }

        self.bus.emit(AppEvent::ModelStateChanged {
            model_id: spec.id.clone(),
            state: "running".into(),
        });
        Ok(running)
    }

    async fn stop(&self, model_id: &str) -> Result<()> {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            if child.model_id != model_id {
                // No es el que se pidió parar: se devuelve al estado anterior.
                *guard = Some(child);
                return Ok(());
            }
            let _ = child.process.kill().await;
            self.bus.emit(AppEvent::ModelStateChanged {
                model_id: model_id.into(),
                state: "stopped".into(),
            });
        }
        Ok(())
    }

    async fn running(&self) -> Option<RunningModel> {
        self.child.lock().await.as_ref().map(|c| c.running.clone())
    }

    async fn chat_stream(
        &self,
        req: ChatRequest,
        mut sink: TokenSink,
        cancel: CancelToken,
    ) -> Result<ChatOutcome> {
        let running = self.running().await.ok_or(RuntimeError::NotRunning)?;
        if running.model_id != req.model_id {
            return Err(RuntimeError::NotRunning);
        }

        let body = serde_json::json!({
            "model": req.model_id,
            "messages": req.messages.iter().map(|m| serde_json::json!({
                "role": m.role,
                "content": m.content,
            })).collect::<Vec<_>>(),
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
            "stream": true,
        });

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", running.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Engine(e.to_string()))?;
        if !response.status().is_success() {
            return Err(RuntimeError::Engine(format!(
                "el motor respondió {}",
                response.status()
            )));
        }

        let mut parser = SseParser::new();
        let mut stream = response.bytes_stream();
        let mut finish_reason = None;
        let mut stopped = false;

        while let Some(chunk) = stream.next().await {
            if cancel.is_cancelled() {
                stopped = true;
                break;
            }
            let chunk = chunk.map_err(|e| RuntimeError::Engine(e.to_string()))?;
            let text = String::from_utf8_lossy(&chunk);
            for event in parser.push(&text) {
                match event {
                    StreamEvent::Delta(delta) => {
                        if !sink(delta) {
                            stopped = true;
                            break;
                        }
                    }
                    StreamEvent::Done(reason) => {
                        if reason.is_some() {
                            finish_reason = reason;
                        }
                    }
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

/// Descomprime un archivo de motor de forma segura frente a *path traversal*.
/// Una entrada con `..` o con ruta absoluta se descarta en lugar de escribirse.
pub fn extract_archive(
    archive: &std::path::Path,
    dest: &std::path::Path,
    kind: ArchiveKind,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    match kind {
        ArchiveKind::TarGz => {
            let file = std::fs::File::open(archive)?;
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            for entry in tar.entries()? {
                let mut entry = entry?;
                let path = entry.path()?.into_owned();
                let Some(safe) = safe_join(dest, &path) else {
                    continue;
                };
                if let Some(parent) = safe.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                entry.unpack(&safe)?;
                make_executable(&safe);
            }
        }
        ArchiveKind::Zip => {
            let file = std::fs::File::open(archive)?;
            let mut zip = zip::ZipArchive::new(file)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            for i in 0..zip.len() {
                let mut entry = zip
                    .by_index(i)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                let Some(name) = entry.enclosed_name() else {
                    continue;
                };
                let Some(safe) = safe_join(dest, &name) else {
                    continue;
                };
                if entry.is_dir() {
                    std::fs::create_dir_all(&safe)?;
                    continue;
                }
                if let Some(parent) = safe.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::File::create(&safe)?;
                std::io::copy(&mut entry, &mut out)?;
                drop(out);
                make_executable(&safe);
            }
        }
    }
    Ok(())
}

fn safe_join(root: &std::path::Path, relative: &std::path::Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut out = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            // `..`, raíz o prefijo de unidad: la entrada se descarta entera.
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("llama-"))
        .unwrap_or(false)
    {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
}

#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_host_has_exactly_one_pin() {
        let pins = engine_pins();
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "x86_64"),
        ] {
            let matches: Vec<_> = pins
                .iter()
                .filter(|p| p.os == os && p.arch == arch)
                .collect();
            assert_eq!(matches.len(), 1, "{os}/{arch} debe tener un único pin");
        }
    }

    #[test]
    fn pins_point_at_https_and_carry_a_mirror_path() {
        for pin in engine_pins() {
            assert!(pin.url.starts_with("https://"), "{} no usa HTTPS", pin.url);
            assert!(
                pin.url.contains(ENGINE_RELEASE),
                "{} no corresponde a la versión fijada",
                pin.url
            );
            assert!(
                !pin.mirror_path.is_empty(),
                "un despliegue corporativo necesita la ruta de mirror"
            );
        }
    }

    #[test]
    fn path_traversal_entries_are_refused() {
        let root = std::path::Path::new("/data/engines");
        assert!(safe_join(root, std::path::Path::new("../../etc/passwd")).is_none());
        assert!(safe_join(root, std::path::Path::new("/etc/passwd")).is_none());
        assert_eq!(
            safe_join(root, std::path::Path::new("build/bin/llama-server")),
            Some(PathBuf::from("/data/engines/build/bin/llama-server"))
        );
    }

    #[test]
    fn an_explicit_engine_binary_is_honoured() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("llama-server");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        let rt = runtime_at(tmp.path());
        std::env::set_var("FACTORIA_LLAMA_SERVER_BIN", &bin);
        let found = rt.engine_binary();
        std::env::remove_var("FACTORIA_LLAMA_SERVER_BIN");
        assert_eq!(found, Some(bin));
    }

    #[test]
    fn a_bundled_engine_is_found_in_either_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime_at(tmp.path());
        assert!(rt.engine_binary().is_none(), "todavía no hay motor");

        let dir = rt
            .paths
            .engine_dir(RUNTIME_ID, ENGINE_RELEASE)
            .join("build")
            .join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        std::fs::write(dir.join(name), b"x").unwrap();
        assert!(
            rt.engine_binary().is_some(),
            "debe encontrarlo en build/bin"
        );
    }

    #[tokio::test]
    async fn probe_reports_ready_once_the_binary_is_there() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime_at(tmp.path());
        // Sin binario: o bien hay pin (hay que preparar) o el host no está soportado.
        assert!(!matches!(rt.probe().await, RuntimeStatus::Ready));

        let dir = rt.paths.engine_dir(RUNTIME_ID, ENGINE_RELEASE);
        std::fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        std::fs::write(dir.join(name), b"x").unwrap();
        assert_eq!(rt.probe().await, RuntimeStatus::Ready);
    }

    #[tokio::test]
    async fn installed_models_ignores_registry_entries_whose_file_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime_at(tmp.path());
        let mut registry = Registry::default();
        registry.0.insert(
            "fantasma".into(),
            InstalledModel {
                model_id: "fantasma".into(),
                runtime: RUNTIME_ID.into(),
                path: Some(
                    tmp.path()
                        .join("no-existe.gguf")
                        .to_string_lossy()
                        .into_owned(),
                ),
                bytes: 1,
                installed_at: "now".into(),
                sha256: None,
                integrity: "none".into(),
                context_window: 4096,
            },
        );
        save_registry(&rt.paths, &registry).unwrap();
        assert!(rt.installed_models().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn chatting_without_a_running_model_is_a_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime_at(tmp.path());
        let err = rt
            .chat_stream(
                ChatRequest {
                    model_id: "x".into(),
                    messages: vec![],
                    temperature: 0.7,
                    max_tokens: 64,
                    context_blocks: vec![],
                    tools: vec![],
                },
                Box::new(|_| true),
                CancelToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, RuntimeError::NotRunning));
    }

    #[tokio::test]
    async fn starting_a_model_that_is_not_installed_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime_at(tmp.path());
        let spec = crate::catalog::Catalog::embedded().models[0].clone();
        let hw = crate::hardware::HardwareProfile::detect(tmp.path());
        let err = rt
            .start(&spec, &Tuning::derive(&spec, &hw))
            .await
            .unwrap_err();
        assert!(matches!(err, RuntimeError::NotInstalled(_)));
    }

    #[test]
    fn free_port_returns_something_usable() {
        let port = LlamaCppRuntime::free_port().unwrap();
        assert!(port > 1024);
    }

    #[test]
    fn the_registry_survives_a_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        let mut registry = Registry::default();
        registry.0.insert(
            "m".into(),
            InstalledModel {
                model_id: "m".into(),
                runtime: RUNTIME_ID.into(),
                path: Some("/x/m.gguf".into()),
                bytes: 10,
                installed_at: "now".into(),
                sha256: Some("a".repeat(64)),
                integrity: "origin".into(),
                context_window: 8192,
            },
        );
        save_registry(&paths, &registry).unwrap();
        assert_eq!(load_registry(&paths).0.len(), 1);
    }

    fn runtime_at(root: &std::path::Path) -> LlamaCppRuntime {
        let paths = Paths::at(root);
        paths.ensure().unwrap();
        let policy = crate::policy::PolicyDocument::default();
        let client = crate::download::build_client(&policy).unwrap();
        let bus = EventBus::new();
        let downloader = Arc::new(Downloader::new(client.clone(), policy, bus.clone()));
        LlamaCppRuntime::new(paths, client, downloader, bus)
    }
}
