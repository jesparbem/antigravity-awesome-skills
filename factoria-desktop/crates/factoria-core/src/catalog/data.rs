//! Catálogo embebido: los modelos que FactorIA Desktop ofrece de serie.
//!
//! Las filas están **fijadas en código**, nunca se obtienen de la red. Ese es el
//! criterio de Rebost y encaja con el requisito corporativo: lo que un empleado
//! puede instalar es una decisión tomada antes de la distribución, no en tiempo
//! de ejecución. Una política corporativa puede sustituir este catálogo entero
//! (`catalog.source`) o filtrarlo con allowlist/denylist.
//!
//! **Orden**: de mayor a menor `capability`. La recomendación elige la fila más
//! capaz que resulte *Óptima* en el equipo; si no hay ninguna, la más capaz
//! *Compatible*. `capability` se fija en la máquina del mantenedor a partir de
//! benchmarks públicos, nunca se calcula aquí.
//!
//! **Tamaños**: los `file_bytes` son los de la cuantización indicada, tomados de
//! la ficha del repositorio. Sirven para dimensionar; el tamaño real se confirma
//! al descargar.
//!
//! **Integridad**: `sha256` va sin anclar en el catálogo de serie. La descarga
//! resuelve el digest contra el propio origen (`X-Linked-Etag` en Hugging Face)
//! y verifica el fichero. Un despliegue corporativo debería anclar los digests
//! en su propio catálogo apuntando a un *mirror* interno (ver
//! `docs/ARCHITECTURE.md` §8).

use super::{KvGeometry, ModelSource, ModelSpec};

const MIB: u64 = 1024 * 1024;

fn hf(repo: &str, file: &str) -> ModelSource {
    ModelSource::GgufUrl {
        url: format!("https://huggingface.co/{repo}/resolve/main/{file}"),
        sha256: None,
        mirror_path: Some(format!("{repo}/{file}")),
    }
}

pub fn embedded_models() -> Vec<ModelSpec> {
    vec![
        ModelSpec {
            id: "qwen2.5-14b-instruct-q4km".into(),
            name: "Qwen2.5 14B Instruct".into(),
            family: "Qwen2.5".into(),
            provider: "Alibaba".into(),
            params_b: Some(14.0),
            quantization: "Q4_K_M".into(),
            file_bytes: 8_988 * MIB,
            context_window: 32_768,
            license: "Apache-2.0".into(),
            license_url: Some("https://huggingface.co/Qwen/Qwen2.5-14B-Instruct".into()),
            released: "2024-09".into(),
            blurb: "El más capaz del catálogo. Multilingüe y sólido con documentos largos. Pide un equipo con memoria holgada.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Qwen2.5-14B-Instruct-GGUF", "Qwen2.5-14B-Instruct-Q4_K_M.gguf"),
            capability: 74,
            kv: KvGeometry { layers: 48, kv_heads: 8, head_dim: 128 },
        },
        ModelSpec {
            id: "gemma-2-9b-it-q4km".into(),
            name: "Gemma 2 9B Instruct".into(),
            family: "Gemma 2".into(),
            provider: "Google".into(),
            params_b: Some(9.0),
            quantization: "Q4_K_M".into(),
            file_bytes: 5_761 * MIB,
            context_window: 8_192,
            license: "Gemma Terms of Use".into(),
            license_url: Some("https://ai.google.dev/gemma/terms".into()),
            released: "2024-06".into(),
            blurb: "Muy buena redacción en castellano. Ventana de contexto más corta que el resto.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/gemma-2-9b-it-GGUF", "gemma-2-9b-it-Q4_K_M.gguf"),
            capability: 66,
            kv: KvGeometry { layers: 42, kv_heads: 8, head_dim: 256 },
        },
        ModelSpec {
            id: "llama-3.1-8b-instruct-q4km".into(),
            name: "Llama 3.1 8B Instruct".into(),
            family: "Llama 3.1".into(),
            provider: "Meta".into(),
            params_b: Some(8.0),
            quantization: "Q4_K_M".into(),
            file_bytes: 4_920 * MIB,
            context_window: 131_072,
            license: "Llama 3.1 Community License".into(),
            license_url: Some("https://www.llama.com/llama3_1/license/".into()),
            released: "2024-07".into(),
            blurb: "Equilibrado y muy contrastado. Contexto amplio para documentos extensos.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Meta-Llama-3.1-8B-Instruct-GGUF", "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf"),
            capability: 62,
            kv: KvGeometry { layers: 32, kv_heads: 8, head_dim: 128 },
        },
        ModelSpec {
            id: "qwen2.5-7b-instruct-q4km".into(),
            name: "Qwen2.5 7B Instruct".into(),
            family: "Qwen2.5".into(),
            provider: "Alibaba".into(),
            params_b: Some(7.0),
            quantization: "Q4_K_M".into(),
            file_bytes: 4_683 * MIB,
            context_window: 32_768,
            license: "Apache-2.0".into(),
            license_url: Some("https://huggingface.co/Qwen/Qwen2.5-7B-Instruct".into()),
            released: "2024-09".into(),
            blurb: "La opción por defecto en un portátil corporativo típico de 16 GB.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Qwen2.5-7B-Instruct-GGUF", "Qwen2.5-7B-Instruct-Q4_K_M.gguf"),
            capability: 60,
            kv: KvGeometry { layers: 28, kv_heads: 4, head_dim: 128 },
        },
        ModelSpec {
            id: "mistral-7b-instruct-v0.3-q4km".into(),
            name: "Mistral 7B Instruct v0.3".into(),
            family: "Mistral".into(),
            provider: "Mistral AI".into(),
            params_b: Some(7.2),
            quantization: "Q4_K_M".into(),
            file_bytes: 4_372 * MIB,
            context_window: 32_768,
            license: "Apache-2.0".into(),
            license_url: Some("https://huggingface.co/mistralai/Mistral-7B-Instruct-v0.3".into()),
            released: "2024-05".into(),
            blurb: "Rápido y sobrio. Buena alternativa europea cuando prima la latencia.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Mistral-7B-Instruct-v0.3-GGUF", "Mistral-7B-Instruct-v0.3-Q4_K_M.gguf"),
            capability: 54,
            kv: KvGeometry { layers: 32, kv_heads: 8, head_dim: 128 },
        },
        ModelSpec {
            id: "qwen2.5-3b-instruct-q4km".into(),
            name: "Qwen2.5 3B Instruct".into(),
            family: "Qwen2.5".into(),
            provider: "Alibaba".into(),
            params_b: Some(3.0),
            quantization: "Q4_K_M".into(),
            file_bytes: 1_929 * MIB,
            context_window: 32_768,
            license: "Qwen Research License".into(),
            license_url: Some("https://huggingface.co/Qwen/Qwen2.5-3B-Instruct".into()),
            released: "2024-09".into(),
            blurb: "Para equipos de 8 GB. Responde bien a tareas cortas y resúmenes.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Qwen2.5-3B-Instruct-GGUF", "Qwen2.5-3B-Instruct-Q4_K_M.gguf"),
            capability: 45,
            kv: KvGeometry { layers: 36, kv_heads: 2, head_dim: 128 },
        },
        ModelSpec {
            id: "phi-3.5-mini-instruct-q4km".into(),
            name: "Phi-3.5 Mini Instruct".into(),
            family: "Phi-3.5".into(),
            provider: "Microsoft".into(),
            params_b: Some(3.8),
            quantization: "Q4_K_M".into(),
            file_bytes: 2_393 * MIB,
            context_window: 131_072,
            license: "MIT".into(),
            license_url: Some("https://huggingface.co/microsoft/Phi-3.5-mini-instruct".into()),
            released: "2024-08".into(),
            blurb: "Pequeño con contexto muy amplio. Rinde por encima de su tamaño en razonamiento.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Phi-3.5-mini-instruct-GGUF", "Phi-3.5-mini-instruct-Q4_K_M.gguf"),
            capability: 43,
            kv: KvGeometry { layers: 32, kv_heads: 32, head_dim: 96 },
        },
        ModelSpec {
            id: "qwen2.5-1.5b-instruct-q4km".into(),
            name: "Qwen2.5 1.5B Instruct".into(),
            family: "Qwen2.5".into(),
            provider: "Alibaba".into(),
            params_b: Some(1.5),
            quantization: "Q4_K_M".into(),
            file_bytes: 1_117 * MIB,
            context_window: 32_768,
            license: "Apache-2.0".into(),
            license_url: Some("https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct".into()),
            released: "2024-09".into(),
            blurb: "Arranca en cualquier equipo. Útil para borradores y reformulación.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Qwen2.5-1.5B-Instruct-GGUF", "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf"),
            capability: 30,
            kv: KvGeometry { layers: 28, kv_heads: 2, head_dim: 128 },
        },
        ModelSpec {
            id: "qwen2.5-0.5b-instruct-q4km".into(),
            name: "Qwen2.5 0.5B Instruct".into(),
            family: "Qwen2.5".into(),
            provider: "Alibaba".into(),
            params_b: Some(0.5),
            quantization: "Q4_K_M".into(),
            file_bytes: 398 * MIB,
            context_window: 32_768,
            license: "Apache-2.0".into(),
            license_url: Some("https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct".into()),
            released: "2024-09".into(),
            blurb: "El más ligero. Pensado para validar la instalación y para equipos muy justos.".into(),
            runtimes: vec!["llamacpp".into(), "ollama".into()],
            source: hf("bartowski/Qwen2.5-0.5B-Instruct-GGUF", "Qwen2.5-0.5B-Instruct-Q4_K_M.gguf"),
            capability: 18,
            kv: KvGeometry { layers: 24, kv_heads: 2, head_dim: 64 },
        },
    ]
}

/// Cobertura de bandas de memoria: se comprueba en los tests de `fit`.
pub const COVERED_RAM_BANDS_GIB: &[u64] = &[8, 16, 32, 64];
