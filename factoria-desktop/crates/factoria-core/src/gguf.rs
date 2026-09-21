//! Lectura mínima de la cabecera GGUF.
//!
//! Solo lo necesario para dos cosas: confirmar que el fichero descargado es de
//! verdad un GGUF antes de activarlo, y leer el contexto entrenado para no pedir
//! al motor una ventana que el modelo no tiene. Enfoque tomado de Rebost
//! (`src-tauri/src/engine/gguf.rs`), reducido a lo que el MVP usa.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

const MAGIC: &[u8; 4] = b"GGUF";
/// Ningún GGUF real tiene una cabecera mayor; el límite evita que un fichero
/// manipulado nos haga leer indefinidamente.
const MAX_HEADER_BYTES: u64 = 8 * 1024 * 1024;
const MAX_KV: u64 = 4_096;
const MAX_STRING_BYTES: u64 = 64 * 1024;
const MAX_ARRAY_ITEMS: u64 = 1_000_000;

// Tipos de valor GGUF.
const TY_UINT8: u32 = 0;
const TY_INT8: u32 = 1;
const TY_UINT16: u32 = 2;
const TY_INT16: u32 = 3;
const TY_UINT32: u32 = 4;
const TY_INT32: u32 = 5;
const TY_FLOAT32: u32 = 6;
const TY_BOOL: u32 = 7;
const TY_STRING: u32 = 8;
const TY_ARRAY: u32 = 9;
const TY_UINT64: u32 = 10;
const TY_INT64: u32 = 11;
const TY_FLOAT64: u32 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufHeader {
    pub version: u32,
    pub tensor_count: u64,
    pub architecture: Option<String>,
    pub context_length: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum GgufError {
    #[error("no se puede abrir el fichero: {0}")]
    Io(#[from] std::io::Error),
    #[error("el fichero no es un GGUF (falta la firma)")]
    NotGguf,
    #[error("versión de GGUF no soportada: {0}")]
    UnsupportedVersion(u32),
    #[error("la cabecera está incompleta o dañada")]
    Malformed,
}

/// Lee la cabecera. Un error aquí significa "no actives este fichero".
pub fn read_header(path: &Path) -> Result<GgufHeader, GgufError> {
    let mut reader = BufReader::new(File::open(path)?);

    let mut magic = [0u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|_| GgufError::NotGguf)?;
    if &magic != MAGIC {
        return Err(GgufError::NotGguf);
    }
    let version = read_u32(&mut reader)?;
    if !(2..=3).contains(&version) {
        return Err(GgufError::UnsupportedVersion(version));
    }
    let tensor_count = read_u64(&mut reader)?;
    let kv_count = read_u64(&mut reader)?;
    if kv_count > MAX_KV {
        return Err(GgufError::Malformed);
    }

    let mut architecture = None;
    let mut context_length = None;

    for _ in 0..kv_count {
        if reader.stream_position()? > MAX_HEADER_BYTES {
            return Err(GgufError::Malformed);
        }
        let key = read_string(&mut reader)?;
        let ty = read_u32(&mut reader)?;
        if key == "general.architecture" && ty == TY_STRING {
            architecture = Some(read_string(&mut reader)?);
            continue;
        }
        if key.ends_with(".context_length") {
            context_length = read_int(&mut reader, ty)?;
            continue;
        }
        skip_value(&mut reader, ty)?;
    }

    Ok(GgufHeader {
        version,
        tensor_count,
        architecture,
        context_length,
    })
}

/// ¿Puede activarse este fichero como modelo? Más estricto que `read_header`:
/// un GGUF sin tensores no sirve de nada.
pub fn is_loadable(path: &Path) -> Result<GgufHeader, GgufError> {
    let header = read_header(path)?;
    if header.tensor_count == 0 {
        return Err(GgufError::Malformed);
    }
    Ok(header)
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, GgufError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf).map_err(|_| GgufError::Malformed)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64, GgufError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf).map_err(|_| GgufError::Malformed)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_string<R: Read>(r: &mut R) -> Result<String, GgufError> {
    let len = read_u64(r)?;
    if len > MAX_STRING_BYTES {
        return Err(GgufError::Malformed);
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).map_err(|_| GgufError::Malformed)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn read_int<R: Read>(r: &mut R, ty: u32) -> Result<Option<u32>, GgufError> {
    let value = match ty {
        TY_UINT32 | TY_INT32 => read_u32(r)?,
        TY_UINT64 | TY_INT64 => read_u64(r)? as u32,
        TY_UINT16 | TY_INT16 => {
            let mut buf = [0u8; 2];
            r.read_exact(&mut buf).map_err(|_| GgufError::Malformed)?;
            u16::from_le_bytes(buf) as u32
        }
        other => {
            skip_value(r, other)?;
            return Ok(None);
        }
    };
    Ok(Some(value))
}

fn skip_value<R: Read>(r: &mut R, ty: u32) -> Result<(), GgufError> {
    let width = match ty {
        TY_UINT8 | TY_INT8 | TY_BOOL => 1,
        TY_UINT16 | TY_INT16 => 2,
        TY_UINT32 | TY_INT32 | TY_FLOAT32 => 4,
        TY_UINT64 | TY_INT64 | TY_FLOAT64 => 8,
        TY_STRING => {
            read_string(r)?;
            return Ok(());
        }
        TY_ARRAY => {
            let item_ty = read_u32(r)?;
            let count = read_u64(r)?;
            if count > MAX_ARRAY_ITEMS {
                return Err(GgufError::Malformed);
            }
            for _ in 0..count {
                skip_value(r, item_ty)?;
            }
            return Ok(());
        }
        _ => return Err(GgufError::Malformed),
    };
    let mut buf = vec![0u8; width];
    r.read_exact(&mut buf).map_err(|_| GgufError::Malformed)?;
    Ok(())
}

/// Escribe un GGUF mínimo pero **válido**. Se usa en los tests y en el arranque
/// de demostración, donde no hay acceso a pesos reales (ver `docs/VALIDATION.md`).
pub fn write_minimal_gguf(
    path: &Path,
    architecture: &str,
    context_length: u32,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&3u32.to_le_bytes()); // versión
    out.extend_from_slice(&1u64.to_le_bytes()); // tensor_count
    out.extend_from_slice(&2u64.to_le_bytes()); // kv_count

    let kv_string = |key: &str, value: &str, out: &mut Vec<u8>| {
        out.extend_from_slice(&(key.len() as u64).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&TY_STRING.to_le_bytes());
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    };
    kv_string("general.architecture", architecture, &mut out);

    let key = format!("{architecture}.context_length");
    out.extend_from_slice(&(key.len() as u64).to_le_bytes());
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(&TY_UINT32.to_le_bytes());
    out.extend_from_slice(&context_length.to_le_bytes());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    File::create(path)?.write_all(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_gguf_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("m.gguf");
        write_minimal_gguf(&path, "qwen2", 32768).unwrap();
        let header = is_loadable(&path).unwrap();
        assert_eq!(header.version, 3);
        assert_eq!(header.architecture.as_deref(), Some("qwen2"));
        assert_eq!(header.context_length, Some(32768));
    }

    #[test]
    fn a_file_that_is_not_gguf_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fake.gguf");
        std::fs::write(&path, b"esto no es un modelo, es un HTML de error").unwrap();
        assert!(matches!(read_header(&path), Err(GgufError::NotGguf)));
    }

    #[test]
    fn an_empty_file_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("empty.gguf");
        std::fs::write(&path, b"").unwrap();
        assert!(matches!(read_header(&path), Err(GgufError::NotGguf)));
    }

    #[test]
    fn a_truncated_header_is_malformed_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cut.gguf");
        write_minimal_gguf(&path, "qwen2", 8192).unwrap();
        let full = std::fs::read(&path).unwrap();
        std::fs::write(&path, &full[..full.len() - 6]).unwrap();
        assert!(matches!(read_header(&path), Err(GgufError::Malformed)));
    }

    #[test]
    fn an_unsupported_version_is_named_in_the_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("v9.gguf");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(
            read_header(&path),
            Err(GgufError::UnsupportedVersion(9))
        ));
    }

    #[test]
    fn a_gguf_without_tensors_is_not_loadable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("no-tensors.gguf");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes()); // sin tensores
        bytes.extend_from_slice(&0u64.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(read_header(&path).is_ok());
        assert!(matches!(is_loadable(&path), Err(GgufError::Malformed)));
    }

    #[test]
    fn an_absurd_kv_count_is_refused_before_allocating() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bomb.gguf");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(read_header(&path), Err(GgufError::Malformed)));
    }

    #[test]
    fn a_missing_file_reports_io_not_a_panic() {
        assert!(matches!(
            read_header(Path::new("/no/existe/x.gguf")),
            Err(GgufError::Io(_))
        ));
    }
}
