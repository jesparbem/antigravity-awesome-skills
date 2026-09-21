//! Host HTTP local de FactorIA Desktop.
//!
//! Sirve la interfaz compilada y expone la misma API que usa el shell Tauri.
//! Escucha **solo en `127.0.0.1`** y exige un token de sesión efímero.
//!
//! Tiene dos usos:
//! 1. Desarrollo y validación de extremo a extremo sin abrir una ventana (y en CI).
//! 2. Es la base de la "API local" del plan de producto: cuando otras
//!    herramientas internas de Naturgy necesiten hablar con la IA local del
//!    empleado, lo harán por aquí.

pub mod api;

use anyhow::Result;
use axum::http::{header, HeaderValue};
use factoria_core::AppCore;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

pub struct Server {
    pub addr: SocketAddr,
    pub token: String,
    pub core: Arc<AppCore>,
    handle: tokio::task::JoinHandle<()>,
}

impl Server {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// URL que abre la interfaz ya autenticada.
    pub fn ui_url(&self) -> String {
        format!("{}/?token={}", self.url(), self.token)
    }

    pub fn shutdown(self) {
        self.handle.abort();
    }
}

/// Arranca el servidor. `port = 0` pide un puerto libre al sistema, que es lo
/// correcto: un puerto fijo chocaría con otra aplicación del empleado.
pub async fn serve(core: Arc<AppCore>, port: u16, static_dir: Option<PathBuf>) -> Result<Server> {
    let token = uuid::Uuid::new_v4().to_string();
    let state = api::AppState {
        core: core.clone(),
        token: token.clone(),
    };

    let mut app = api::router(state);

    if let Some(dir) = static_dir.filter(|d| d.is_dir()) {
        let index = dir.join("index.html");
        // Aplicación de una sola página: cualquier ruta desconocida sirve el
        // index y el enrutado lo resuelve el cliente.
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }

    // Sin CDNs ni `unsafe-eval`: la interfaz debe funcionar detrás de un proxy
    // corporativo y sin internet.
    let csp = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
               script-src 'self'; connect-src 'self'; font-src 'self'; object-src 'none'; \
               base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
    let app = app
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(csp),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ));

    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "FactorIA escuchando en loopback");

    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            tracing::error!(?err, "el servidor se detuvo");
        }
    });

    Ok(Server {
        addr,
        token,
        core,
        handle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use factoria_core::paths::Paths;

    async fn server() -> (tempfile::TempDir, Server, reqwest::Client) {
        let tmp = tempfile::tempdir().unwrap();
        let core = AppCore::bootstrap_at(Paths::at(tmp.path())).unwrap();
        let server = serve(core, 0, None).await.unwrap();
        (tmp, server, reqwest::Client::new())
    }

    #[tokio::test]
    async fn it_listens_only_on_loopback() {
        let (_t, server, _c) = server().await;
        assert!(
            server.addr.ip().is_loopback(),
            "no puede escuchar fuera del equipo"
        );
        assert_ne!(server.addr.port(), 0);
    }

    #[tokio::test]
    async fn health_needs_no_token_so_the_ui_can_probe_it() {
        let (_t, server, client) = server().await;
        let res = client
            .get(format!("{}/api/health", server.url()))
            .send()
            .await
            .unwrap();
        assert!(res.status().is_success());
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], factoria_core::VERSION);
    }

    #[tokio::test]
    async fn every_data_endpoint_refuses_a_request_without_the_token() {
        let (_t, server, client) = server().await;
        for path in [
            "/api/home",
            "/api/models",
            "/api/threads",
            "/api/settings",
            "/api/policy",
        ] {
            let res = client
                .get(format!("{}{path}", server.url()))
                .send()
                .await
                .unwrap();
            assert_eq!(res.status(), 401, "{path} debería exigir token");
        }
    }

    #[tokio::test]
    async fn a_wrong_token_is_refused() {
        let (_t, server, client) = server().await;
        let res = client
            .get(format!("{}/api/home", server.url()))
            .header("x-factoria-token", "no-es-el-token")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401);
    }

    #[tokio::test]
    async fn the_home_snapshot_comes_back_with_the_token() {
        let (_t, server, client) = server().await;
        let body: serde_json::Value = client
            .get(format!("{}/api/home", server.url()))
            .header("x-factoria-token", &server.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(body["hardware"]["totalRamBytes"].as_u64().unwrap() > 0);
        assert!(body["runtimes"].as_array().unwrap().len() >= 2);
        assert_eq!(body["organization"], "Naturgy");
    }

    #[tokio::test]
    async fn the_catalog_arrives_classified() {
        let (_t, server, client) = server().await;
        let cards: Vec<serde_json::Value> = client
            .get(format!("{}/api/models", server.url()))
            .header("x-factoria-token", &server.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!cards.is_empty());
        for card in &cards {
            assert!(card["model"]["id"].is_string());
            assert!(card["verdict"]["level"].is_string());
            assert_eq!(card["state"], "notInstalled");
        }
    }

    #[tokio::test]
    async fn a_conversation_can_be_created_listed_and_deleted() {
        let (_t, server, client) = server().await;
        let thread: serde_json::Value = client
            .post(format!("{}/api/threads", server.url()))
            .header("x-factoria-token", &server.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = thread["id"].as_str().unwrap().to_string();

        let listed: Vec<serde_json::Value> = client
            .get(format!("{}/api/threads", server.url()))
            .header("x-factoria-token", &server.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);

        let res = client
            .delete(format!("{}/api/threads/{id}", server.url()))
            .header("x-factoria-token", &server.token)
            .send()
            .await
            .unwrap();
        assert!(res.status().is_success());
    }

    #[tokio::test]
    async fn an_unknown_model_is_a_404_not_a_500() {
        let (_t, server, client) = server().await;
        let res = client
            .post(format!("{}/api/models/no-existe/start", server.url()))
            .header("x-factoria-token", &server.token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 404);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("no-existe"));
    }

    #[tokio::test]
    async fn settings_can_be_read_and_written() {
        let (_t, server, client) = server().await;
        let mut settings: serde_json::Value = client
            .get(format!("{}/api/settings", server.url()))
            .header("x-factoria-token", &server.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        settings["onboarded"] = serde_json::Value::Bool(true);
        let saved: serde_json::Value = client
            .post(format!("{}/api/settings", server.url()))
            .header("x-factoria-token", &server.token)
            .json(&settings)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(saved["onboarded"], true);
    }

    #[tokio::test]
    async fn the_event_stream_accepts_the_token_in_the_query() {
        let (_t, server, client) = server().await;
        let res = client
            .get(format!(
                "{}/api/events?token={}",
                server.url(),
                server.token
            ))
            .send()
            .await
            .unwrap();
        assert!(res.status().is_success());
        assert!(res
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/event-stream"));
    }

    #[tokio::test]
    async fn the_event_stream_without_a_token_is_refused() {
        let (_t, server, client) = server().await;
        let res = client
            .get(format!("{}/api/events", server.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401);
    }

    #[tokio::test]
    async fn responses_carry_a_strict_content_security_policy() {
        let (_t, server, client) = server().await;
        let res = client
            .get(format!("{}/api/health", server.url()))
            .send()
            .await
            .unwrap();
        let csp = res
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(csp.contains("default-src 'self'"));
        assert!(!csp.contains("unsafe-eval"), "la interfaz no necesita eval");
        assert_eq!(
            res.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
    }

    #[tokio::test]
    async fn the_ui_url_carries_the_token() {
        let (_t, server, _c) = server().await;
        assert!(server.ui_url().contains(&server.token));
    }
}
