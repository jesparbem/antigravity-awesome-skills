//! Rutas HTTP: adaptadores finos sobre `AppCore`.
//!
//! Ninguna decisión de producto vive aquí. Si un manejador crece más allá de
//! "traducir entrada, llamar al núcleo, traducir salida", la lógica está en el
//! sitio equivocado.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use factoria_core::AppCore;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Clone)]
pub struct AppState {
    pub core: Arc<AppCore>,
    /// Token de sesión, generado al arrancar. Una página local sin él no pasa.
    pub token: String,
}

/// Error HTTP con un mensaje que la interfaz puede mostrar tal cual.
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<factoria_core::app::CoreError> for ApiError {
    fn from(err: factoria_core::app::CoreError) -> Self {
        use factoria_core::app::CoreError;
        let status = match &err {
            CoreError::UnknownModel(_) | CoreError::UnknownThread(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, err.to_string())
    }
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/home", get(home))
        .route("/api/hardware", get(hardware))
        .route("/api/resources", get(resources))
        .route("/api/models", get(models))
        .route("/api/models/{id}/install", post(install))
        .route("/api/models/{id}/start", post(start))
        .route("/api/models/{id}/stop", post(stop))
        .route("/api/models/{id}", delete(remove))
        .route("/api/threads", get(threads).post(create_thread))
        .route("/api/threads/{id}", delete(delete_thread))
        .route("/api/threads/{id}/messages", get(messages))
        .route("/api/threads/{id}/send", post(send))
        .route("/api/threads/{id}/clear", post(clear_thread))
        .route("/api/generation/stop", post(stop_generation))
        .route("/api/settings", get(settings).post(save_settings))
        .route("/api/policy", get(policy))
        .route("/api/audit", get(audit))
        .route("/api/events", get(events))
        .with_state(state)
}

/// Comprueba el token de sesión.
///
/// Cualquier página web que el empleado abra puede intentar hablar con
/// `127.0.0.1`. El token, que solo conoce la interfaz servida por este proceso,
/// impide que una pestaña ajena use la IA local del equipo.
fn authorize(state: &AppState, headers: &HeaderMap, query: Option<&str>) -> Result<(), ApiError> {
    let presented = headers
        .get("x-factoria-token")
        .and_then(|v| v.to_str().ok())
        .or(query);
    if presented == Some(state.token.as_str()) {
        return Ok(());
    }
    Err(ApiError(
        StatusCode::UNAUTHORIZED,
        "token de sesión ausente o incorrecto".into(),
    ))
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "product": factoria_core::PRODUCT,
        "version": factoria_core::VERSION,
        "dataDir": state.core.paths().root().to_string_lossy(),
    }))
}

async fn home(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<factoria_core::app::HomeSnapshot> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.home().await))
}

async fn hardware(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<factoria_core::hardware::HardwareProfile> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.hardware().clone()))
}

async fn resources(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<factoria_core::metrics::ResourceSample> {
    authorize(&state, &headers, None)?;
    let core = state.core.clone();
    // El muestreo de CPU duerme un instante: fuera del hilo del runtime.
    let sample = tokio::task::spawn_blocking(move || core.resources())
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(sample))
}

async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Vec<factoria_core::app::ModelCard>> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.model_cards().await))
}

async fn install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<factoria_core::runtime::InstalledModel> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.install_model(&id).await?))
}

async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<factoria_core::runtime::RunningModel> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.start_model(&id).await?))
}

async fn stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    authorize(&state, &headers, None)?;
    state.core.stop_model(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    authorize(&state, &headers, None)?;
    state.core.remove_model(&id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn threads(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Vec<factoria_core::chat::Thread>> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.threads()))
}

async fn create_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<factoria_core::chat::Thread> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.create_thread().await?))
}

async fn delete_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    authorize(&state, &headers, None)?;
    state.core.delete_thread(&id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn clear_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    authorize(&state, &headers, None)?;
    state.core.clear_thread(&id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Vec<factoria_core::chat::Message>> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.messages(&id)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendBody {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub regenerate: bool,
}

async fn send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<SendBody>,
) -> ApiResult<factoria_core::chat::Message> {
    authorize(&state, &headers, None)?;
    Ok(Json(
        state.core.send_message(&id, &body.text, body.regenerate).await?,
    ))
}

async fn stop_generation(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<serde_json::Value> {
    authorize(&state, &headers, None)?;
    state.core.stop_generation().await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<factoria_core::settings::Settings> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.settings().await))
}

async fn save_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<factoria_core::settings::Settings>,
) -> ApiResult<factoria_core::settings::Settings> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.update_settings(body).await?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyView {
    #[serde(flatten)]
    policy: factoria_core::policy::EffectivePolicy,
    runtimes: Vec<factoria_core::runtime::registry::RuntimeReport>,
    catalog_source: String,
    data_dir: String,
}

async fn policy(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<PolicyView> {
    authorize(&state, &headers, None)?;
    Ok(Json(PolicyView {
        policy: state.core.policy().clone(),
        runtimes: state.core.runtime_reports().await,
        catalog_source: state.core.catalog().source.clone(),
        data_dir: state.core.paths().root().to_string_lossy().into_owned(),
    }))
}

#[derive(Debug, Deserialize)]
struct AuditQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

async fn audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Vec<factoria_core::audit::AuditEntry>> {
    authorize(&state, &headers, None)?;
    Ok(Json(state.core.audit().tail(q.limit.min(500))))
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    token: Option<String>,
}

/// Flujo SSE con los eventos del núcleo.
///
/// `EventSource` del navegador no permite cabeceras propias, así que aquí el
/// token viaja en la query. Sigue siendo un secreto que solo conoce la interfaz
/// servida por este proceso.
async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    authorize(&state, &headers, q.token.as_deref())?;
    let stream = BroadcastStream::new(state.core.bus().subscribe()).filter_map(|event| {
        // Un consumidor lento pierde eventos intermedios; el flujo continúa.
        event
            .ok()
            .and_then(|e| serde_json::to_string(&e).ok())
            .map(|json| Ok(Event::default().data(json)))
    });
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}
