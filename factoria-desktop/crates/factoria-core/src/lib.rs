//! # factoria-core
//!
//! Núcleo de **FactorIA Desktop**. Contiene toda la lógica del producto y no
//! depende de ningún *shell*: lo alojan tanto la aplicación Tauri (`src-tauri`)
//! como el servidor HTTP local (`factoria-server`).
//!
//! Obra derivada de [Rebost](https://github.com/Frontierz-AI/Rebost) (MIT).
//! Ver `NOTICE.md` en la raíz del proyecto.

pub mod app;
pub mod audit;
pub mod catalog;
pub mod chat;
pub mod download;
pub mod events;
pub mod gguf;
pub mod hardware;
pub mod metrics;
pub mod paths;
pub mod policy;
pub mod runtime;
pub mod settings;

pub use app::AppCore;

/// Nombre de producto, usado en rutas, cabeceras y `User-Agent`.
pub const PRODUCT: &str = "FactorIA Desktop";
/// Identificador de aplicación para el directorio de datos.
pub const APP_ID: &str = "es.naturgy.factoria";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tamaños legibles para personas: "8,99 GB", "512 MB".
///
/// Se usan unidades binarias con etiqueta decimal, que es lo que muestran los
/// sistemas operativos y las fichas de los modelos.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        let v = b / GB;
        if v >= 10.0 {
            format!("{:.0} GB", v)
        } else {
            format!("{:.2} GB", v).replace('.', ",")
        }
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_read_like_a_file_manager() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(900), "900 B");
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5 MB");
        assert_eq!(format_bytes(8_988 * 1024 * 1024), "8,78 GB");
        assert_eq!(format_bytes(24 * 1024 * 1024 * 1024), "24 GB");
    }
}
