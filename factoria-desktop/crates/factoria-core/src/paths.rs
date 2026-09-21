//! Rutas de datos de la aplicación.
//!
//! Un único sitio decide dónde vive cada cosa, para que el shell Tauri, el
//! servidor HTTP y los tests coincidan. `FACTORIA_DATA_DIR` permite apuntar a un
//! directorio temporal en pruebas y pilotos.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// Directorio de datos real del sistema, o el que indique
    /// `FACTORIA_DATA_DIR`.
    pub fn resolve() -> Self {
        if let Some(dir) = std::env::var_os("FACTORIA_DATA_DIR") {
            return Self::at(PathBuf::from(dir));
        }
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::at(base.join(crate::APP_ID))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn local_policy_file(&self) -> PathBuf {
        self.root.join("policy.json")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn models_registry(&self) -> PathBuf {
        self.models_dir().join("models.json")
    }

    pub fn engines_dir(&self) -> PathBuf {
        self.root.join("engines")
    }

    pub fn engine_dir(&self, runtime: &str, release: &str) -> PathBuf {
        self.engines_dir().join(runtime).join(release)
    }

    pub fn conversations_dir(&self) -> PathBuf {
        self.root.join("conversations")
    }

    pub fn threads_file(&self) -> PathBuf {
        self.conversations_dir().join("threads.json")
    }

    pub fn thread_file(&self, thread_id: &str) -> PathBuf {
        self.conversations_dir().join(format!("{thread_id}.jsonl"))
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn audit_dir(&self) -> PathBuf {
        self.root.join("audit")
    }

    /// Crea todo el árbol. Se llama una vez al arrancar.
    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [
            self.root.clone(),
            self.models_dir(),
            self.engines_dir(),
            self.conversations_dir(),
            self.logs_dir(),
            self.audit_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_creates_the_whole_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path().join("data"));
        paths.ensure().unwrap();
        assert!(paths.models_dir().is_dir());
        assert!(paths.conversations_dir().is_dir());
        assert!(paths.audit_dir().is_dir());
        assert!(paths.logs_dir().is_dir());
    }

    #[test]
    fn ensure_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        paths.ensure().unwrap();
    }

    #[test]
    fn every_path_stays_under_the_root() {
        let paths = Paths::at("/tmp/factoria-test");
        for p in [
            paths.settings_file(),
            paths.models_registry(),
            paths.thread_file("abc"),
            paths.engine_dir("llamacpp", "b1-cpu"),
        ] {
            assert!(p.starts_with(paths.root()));
        }
    }
}
