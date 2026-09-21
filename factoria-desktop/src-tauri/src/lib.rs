//! Shell de escritorio de FactorIA Desktop.
//!
//! Es un **adaptador**: cada comando traduce la entrada, llama a `AppCore` y
//! devuelve el resultado. Toda la lógica vive en `factoria-core`, que el
//! servidor HTTP local aloja de la misma manera. Si un comando de este fichero
//! empieza a tomar decisiones, están en el sitio equivocado.

use factoria_core::app::{HomeSnapshot, ModelCard};
use factoria_core::audit::AuditEntry;
use factoria_core::chat::{Message, Thread};
use factoria_core::hardware::HardwareProfile;
use factoria_core::metrics::ResourceSample;
use factoria_core::policy::EffectivePolicy;
use factoria_core::runtime::registry::RuntimeReport;
use factoria_core::runtime::{InstalledModel, RunningModel};
use factoria_core::settings::Settings;
use factoria_core::AppCore;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// Nombre del evento que recibe la interfaz. El frontend escucha exactamente
/// este, igual que escucha el SSE del host HTTP.
const EVENT_NAME: &str = "factoria://event";

type State<'a> = tauri::State<'a, Arc<AppCore>>;

/// Error de comando con un mensaje que la interfaz puede mostrar tal cual.
#[derive(Debug, Serialize)]
struct CommandError(String);

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<factoria_core::app::CoreError> for CommandError {
    fn from(err: factoria_core::app::CoreError) -> Self {
        Self(err.to_string())
    }
}

type Result<T> = std::result::Result<T, CommandError>;

#[tauri::command]
async fn home(core: State<'_>) -> Result<HomeSnapshot> {
    Ok(core.home().await)
}

#[tauri::command]
async fn hardware(core: State<'_>) -> Result<HardwareProfile> {
    Ok(core.hardware().clone())
}

#[tauri::command]
async fn resources(core: State<'_>) -> Result<ResourceSample> {
    let core = core.inner().clone();
    // El muestreo de CPU necesita una pausa: fuera del hilo del runtime async.
    tokio::task::spawn_blocking(move || core.resources())
        .await
        .map_err(|e| CommandError(e.to_string()))
}

#[tauri::command]
async fn models(core: State<'_>) -> Result<Vec<ModelCard>> {
    Ok(core.model_cards().await)
}

#[tauri::command]
async fn install_model(core: State<'_>, id: String) -> Result<InstalledModel> {
    Ok(core.install_model(&id).await?)
}

#[tauri::command]
async fn start_model(core: State<'_>, id: String) -> Result<RunningModel> {
    Ok(core.start_model(&id).await?)
}

#[tauri::command]
async fn stop_model(core: State<'_>, id: String) -> Result<()> {
    Ok(core.stop_model(&id).await?)
}

#[tauri::command]
async fn remove_model(core: State<'_>, id: String) -> Result<()> {
    Ok(core.remove_model(&id).await?)
}

#[tauri::command]
async fn threads(core: State<'_>) -> Result<Vec<Thread>> {
    Ok(core.threads())
}

#[tauri::command]
async fn create_thread(core: State<'_>) -> Result<Thread> {
    Ok(core.create_thread().await?)
}

#[tauri::command]
async fn delete_thread(core: State<'_>, id: String) -> Result<()> {
    Ok(core.delete_thread(&id)?)
}

#[tauri::command]
async fn clear_thread(core: State<'_>, id: String) -> Result<()> {
    Ok(core.clear_thread(&id)?)
}

#[tauri::command]
async fn messages(core: State<'_>, id: String) -> Result<Vec<Message>> {
    Ok(core.messages(&id))
}

#[tauri::command]
async fn send_message(
    core: State<'_>,
    id: String,
    text: String,
    regenerate: bool,
) -> Result<Message> {
    Ok(core.send_message(&id, &text, regenerate).await?)
}

#[tauri::command]
async fn stop_generation(core: State<'_>) -> Result<()> {
    core.stop_generation().await;
    Ok(())
}

#[tauri::command]
async fn settings(core: State<'_>) -> Result<Settings> {
    Ok(core.settings().await)
}

#[tauri::command]
async fn save_settings(core: State<'_>, settings: Settings) -> Result<Settings> {
    Ok(core.update_settings(settings).await?)
}

/// Misma forma que `/api/policy` del host HTTP, para que la interfaz no
/// distinga entre ambos.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyView {
    #[serde(flatten)]
    policy: EffectivePolicy,
    runtimes: Vec<RuntimeReport>,
    catalog_source: String,
    data_dir: String,
}

#[tauri::command]
async fn policy(core: State<'_>) -> Result<PolicyView> {
    Ok(PolicyView {
        policy: core.policy().clone(),
        runtimes: core.runtime_reports().await,
        catalog_source: core.catalog().source.clone(),
        data_dir: core.paths().root().to_string_lossy().into_owned(),
    })
}

#[tauri::command]
async fn audit(core: State<'_>, limit: Option<usize>) -> Result<Vec<AuditEntry>> {
    Ok(core.audit().tail(limit.unwrap_or(50).min(500)))
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("FACTORIA_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let core = AppCore::bootstrap()?;
            app.manage(core.clone());

            // El bus del núcleo se reexpone como un evento de Tauri, con la
            // misma forma que el SSE del host HTTP.
            let handle = app.handle().clone();
            let mut rx = core.bus().subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            let _ = handle.emit(EVENT_NAME, &event);
                        }
                        // Un consumidor lento pierde eventos intermedios, pero
                        // el flujo continúa; solo el cierre del canal termina.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            home,
            hardware,
            resources,
            models,
            install_model,
            start_model,
            stop_model,
            remove_model,
            threads,
            create_thread,
            delete_thread,
            clear_thread,
            messages,
            send_message,
            stop_generation,
            settings,
            save_settings,
            policy,
            audit,
        ])
        .run(tauri::generate_context!())
        .expect("no se pudo arrancar FactorIA Desktop");
}

/// Comprobación en tiempo de compilación de que el evento y los tipos que la
/// interfaz espera siguen existiendo con la forma esperada.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_nombre_del_evento_coincide_con_el_que_escucha_la_interfaz() {
        // `src/lib/api.ts` escucha exactamente esta cadena.
        assert_eq!(EVENT_NAME, "factoria://event");
    }

    #[test]
    fn los_eventos_del_nucleo_se_serializan_con_la_misma_forma_que_el_sse() {
        let json = serde_json::to_string(&factoria_core::events::AppEvent::ChatDelta {
            thread_id: "t".into(),
            message_id: "m".into(),
            text: "hola".into(),
        })
        .unwrap();
        assert!(json.contains(r#""type":"chatDelta""#));
        assert!(json.contains(r#""threadId":"t""#));
    }

    #[test]
    fn un_error_del_nucleo_llega_a_la_interfaz_como_texto_legible() {
        let err: CommandError = factoria_core::app::CoreError::UnknownModel("x".into()).into();
        assert!(err.to_string().contains('x'));
    }
}
