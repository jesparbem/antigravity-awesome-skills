//! Integridad de las descargas.
//!
//! Un fichero de varios GB que no se puede verificar no se instala. Estas
//! pruebas levantan un servidor HTTP mínimo en loopback y comprueban el
//! comportamiento real del descargador frente a cada caso.

use factoria_core::download::{build_client, DownloadError, Downloader, IntegritySource};
use factoria_core::events::EventBus;
use factoria_core::policy::PolicyDocument;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CUERPO: &[u8] = b"pesos de mentira pero bytes de verdad";

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Servidor HTTP mínimo que sirve `CUERPO` una vez por conexión.
/// `linked_etag` decide si el origen publica el SHA-256 del objeto.
async fn origen(linked_etag: Option<String>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let etag = linked_etag.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let mut head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n",
                    CUERPO.len()
                );
                if let Some(etag) = etag {
                    head.push_str(&format!("X-Linked-Etag: \"{etag}\"\r\n"));
                }
                head.push_str("Connection: close\r\n\r\n");
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(CUERPO).await;
                let _ = socket.flush().await;
            });
        }
    });
    (format!("http://{addr}/pesos.gguf"), handle)
}

fn descargador(policy: PolicyDocument) -> Arc<Downloader> {
    let client = build_client(&policy).unwrap();
    Arc::new(Downloader::new(client, policy, EventBus::new()))
}

#[tokio::test]
async fn un_digest_anclado_que_coincide_instala_el_fichero() {
    let (url, server) = origen(None).await;
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("m.gguf");

    let outcome = descargador(PolicyDocument::default())
        .fetch("m", &url, None, Some(&digest(CUERPO)), &dest)
        .await
        .expect("debe instalarse");

    assert_eq!(outcome.integrity, IntegritySource::Pinned);
    assert_eq!(outcome.bytes, CUERPO.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), CUERPO);
    server.abort();
}

#[tokio::test]
async fn un_digest_anclado_que_no_coincide_descarta_el_fichero() {
    let (url, server) = origen(None).await;
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("m.gguf");

    let err = descargador(PolicyDocument::default())
        .fetch("m", &url, None, Some(&"b".repeat(64)), &dest)
        .await
        .unwrap_err();

    assert!(matches!(err, DownloadError::IntegrityMismatch { .. }));
    assert!(!dest.exists(), "un fichero que no cuadra no puede quedarse");
    assert!(
        !dest.with_extension("part").exists(),
        "tampoco puede quedarse el parcial"
    );
    server.abort();
}

#[tokio::test]
async fn sin_digest_anclado_se_usa_el_que_publica_el_origen() {
    let (url, server) = origen(Some(digest(CUERPO))).await;
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("m.gguf");

    let outcome = descargador(PolicyDocument::default())
        .fetch("m", &url, None, None, &dest)
        .await
        .expect("el origen publica el SHA-256");

    assert_eq!(outcome.integrity, IntegritySource::Origin);
    assert_eq!(outcome.sha256, digest(CUERPO));
    server.abort();
}

#[tokio::test]
async fn un_digest_del_origen_que_no_coincide_tambien_se_rechaza() {
    let (url, server) = origen(Some("c".repeat(64))).await;
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("m.gguf");

    let err = descargador(PolicyDocument::default())
        .fetch("m", &url, None, None, &dest)
        .await
        .unwrap_err();

    assert!(matches!(err, DownloadError::IntegrityMismatch { .. }));
    assert!(!dest.exists());
    server.abort();
}

#[tokio::test]
async fn sin_ninguna_garantia_de_integridad_no_se_instala() {
    let (url, server) = origen(None).await;
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("m.gguf");

    let err = descargador(PolicyDocument::default())
        .fetch("m", &url, None, None, &dest)
        .await
        .unwrap_err();

    assert!(
        matches!(err, DownloadError::IntegrityUnavailable),
        "por defecto, sin digest no hay instalación"
    );
    assert!(!dest.exists());
    server.abort();
}

#[tokio::test]
async fn la_politica_puede_autorizar_una_descarga_sin_verificar() {
    let (url, server) = origen(None).await;
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("m.gguf");

    let mut policy = PolicyDocument::default();
    policy.catalog.allow_unverified_downloads = true;

    let outcome = descargador(policy)
        .fetch("m", &url, None, None, &dest)
        .await
        .expect("la política lo permite explícitamente");

    assert_eq!(outcome.integrity, IntegritySource::None);
    assert!(dest.exists());
    server.abort();
}

#[tokio::test]
async fn el_mirror_corporativo_sustituye_al_origen_publico() {
    let (url, server) = origen(None).await;
    // El mirror apunta al servidor de prueba; la URL "pública" no existe.
    let base = url.rsplit_once('/').unwrap().0.to_string();
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("m.gguf");

    let mut policy = PolicyDocument::default();
    policy.catalog.mirror_base_url = Some(base);

    let outcome = descargador(policy)
        .fetch(
            "m",
            "https://huggingface.co/no/existe.gguf",
            Some("pesos.gguf"),
            Some(&digest(CUERPO)),
            &dest,
        )
        .await
        .expect("debe descargarse del mirror");

    assert_eq!(outcome.bytes, CUERPO.len() as u64);
    server.abort();
}

#[tokio::test]
async fn una_respuesta_de_error_del_servidor_se_reporta_con_su_codigo() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        }
    });

    let tmp = tempfile::tempdir().unwrap();
    let err = descargador(PolicyDocument::default())
        .fetch(
            "m",
            &format!("http://{addr}/no-existe.gguf"),
            None,
            None,
            &tmp.path().join("m.gguf"),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, DownloadError::Status(404)));
    server.abort();
}

#[tokio::test]
async fn el_progreso_llega_por_el_bus_de_eventos() {
    let (url, server) = origen(None).await;
    let tmp = tempfile::tempdir().unwrap();
    let bus = EventBus::new();
    let mut rx = bus.subscribe();
    let policy = PolicyDocument::default();
    let downloader = Downloader::new(build_client(&policy).unwrap(), policy, bus);

    downloader
        .fetch(
            "mi-modelo",
            &url,
            None,
            Some(&digest(CUERPO)),
            &tmp.path().join("m.gguf"),
        )
        .await
        .unwrap();

    let event = rx
        .try_recv()
        .expect("al terminar siempre se emite el progreso final");
    match event {
        factoria_core::events::AppEvent::DownloadProgress {
            model_id,
            received_bytes,
            ..
        } => {
            assert_eq!(model_id, "mi-modelo");
            assert_eq!(received_bytes, CUERPO.len() as u64);
        }
        other => panic!("evento inesperado: {other:?}"),
    }
    server.abort();
}
