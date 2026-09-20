//! Parámetros de arranque derivados del hardware.
//!
//! Este módulo es la razón por la que el empleado nunca ve un parámetro de
//! inferencia: todo lo que llama.cpp necesita en la línea de órdenes se deduce
//! de la máquina y del modelo. Criterios heredados de `engine/tune.rs` de
//! Rebost, reducidos a lo que el MVP usa y con los umbrales explicados.

use crate::catalog::ModelSpec;
use crate::hardware::{Accelerator, HardwareProfile};
use serde::{Deserialize, Serialize};

/// Ventana mínima: por debajo de 4k no cabe una conversación de trabajo.
pub const MIN_CONTEXT: u32 = 4_096;
/// Ventana máxima del MVP. Más contexto multiplica la KV cache sin que el
/// empleado lo note hasta que el equipo empieza a paginar.
pub const MAX_CONTEXT: u32 = 16_384;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tuning {
    pub context_tokens: u32,
    pub max_output_tokens: u32,
    /// Capas que se descargan a la GPU. `0` = todo en CPU;
    /// un número grande = "todas las que quepan" (convención de llama.cpp).
    pub gpu_layers: u32,
    pub threads: u32,
    pub batch_size: u32,
    /// Cuantización de la caché K/V (`q8_0` ahorra la mitad que `f16`).
    pub kv_cache_type: String,
    pub flash_attention: String,
    pub use_mmap: bool,
}

impl Tuning {
    /// Deriva los parámetros para este modelo en este equipo.
    pub fn derive(model: &ModelSpec, hw: &HardwareProfile) -> Self {
        let gpu_resident = gpu_can_hold(model, hw);
        let context_tokens = context_for(model, hw, gpu_resident);

        // Convención de llama.cpp: un `-ngl` alto significa "todas las capas".
        let gpu_layers = if hw.unified_memory {
            999
        } else if hw.usable_vram_bytes == 0 {
            0
        } else if gpu_resident {
            999
        } else {
            partial_layers(model, hw)
        };

        // Dejar un núcleo libre evita que la interfaz se congele mientras genera.
        let threads = hw.physical_cores.saturating_sub(1).clamp(1, 16);

        Self {
            context_tokens,
            max_output_tokens: output_cap_for(context_tokens),
            gpu_layers,
            threads,
            batch_size: if gpu_layers > 0 { 512 } else { 256 },
            // OpenCL (Adreno) se cuelga con la caché K/V cuantizada.
            kv_cache_type: match hw.accelerator {
                Accelerator::OpenCl => "f16".into(),
                _ => "q8_0".into(),
            },
            flash_attention: match hw.accelerator {
                Accelerator::Metal | Accelerator::Cuda | Accelerator::Vulkan => "on".into(),
                _ => "auto".into(),
            },
            // CUDA y Vulkan discretas copian los pesos a la GPU: mantener el
            // mmap solo duplica la presión de memoria.
            use_mmap: !matches!(hw.accelerator, Accelerator::Cuda | Accelerator::Vulkan),
        }
    }

    /// Argumentos de `llama-server` correspondientes.
    pub fn llama_server_args(&self, model_path: &str, port: u16) -> Vec<String> {
        let mut args = vec![
            "--model".into(),
            model_path.to_string(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "--ctx-size".into(),
            self.context_tokens.to_string(),
            "--n-predict".into(),
            self.max_output_tokens.to_string(),
            "--threads".into(),
            self.threads.to_string(),
            "--batch-size".into(),
            self.batch_size.to_string(),
            "--n-gpu-layers".into(),
            self.gpu_layers.to_string(),
            "--cache-type-k".into(),
            self.kv_cache_type.clone(),
            "--cache-type-v".into(),
            self.kv_cache_type.clone(),
            "--flash-attn".into(),
            self.flash_attention.clone(),
        ];
        if !self.use_mmap {
            args.push("--no-mmap".into());
        }
        args
    }
}

fn gpu_can_hold(model: &ModelSpec, hw: &HardwareProfile) -> bool {
    if hw.unified_memory {
        return true;
    }
    if hw.usable_vram_bytes == 0 {
        return false;
    }
    model.file_bytes + model.kv.kv_cache_bytes(MIN_CONTEXT) <= hw.vram_budget_bytes()
}

/// Cuántas capas caben en la gráfica cuando el modelo entero no cabe.
/// Aproximación por fracción de los pesos, redondeando a la baja.
fn partial_layers(model: &ModelSpec, hw: &HardwareProfile) -> u32 {
    if model.file_bytes == 0 || model.kv.layers == 0 {
        return 0;
    }
    let usable = hw
        .vram_budget_bytes()
        .saturating_sub(model.kv.kv_cache_bytes(MIN_CONTEXT));
    let fraction = usable as f64 / model.file_bytes as f64;
    ((model.kv.layers as f64) * fraction)
        .floor()
        .clamp(0.0, model.kv.layers as f64) as u32
}

/// Ventana de contexto: el techo lo pone el modelo; el suelo, la memoria del
/// equipo una vez descontados los pesos.
fn context_for(model: &ModelSpec, hw: &HardwareProfile, gpu_resident: bool) -> u32 {
    let ceiling = model.context_window.min(MAX_CONTEXT);
    if ceiling <= MIN_CONTEXT {
        return MIN_CONTEXT.min(model.context_window.max(MIN_CONTEXT));
    }

    let bytes_per_token = model.kv.bytes_per_token().max(1);
    let weights = (model.file_bytes as f64 * hw.accelerator.weight_copies()) as u64;
    let pool = if gpu_resident && !hw.unified_memory {
        hw.vram_budget_bytes()
    } else {
        hw.ram_budget_bytes()
    };
    let room = pool
        .saturating_sub(weights)
        .saturating_sub(1024 * 1024 * 1024);
    let affordable = (room / bytes_per_token) as u32;

    // Escalones fijos: dos equipos iguales deben arrancar igual.
    let steps = [MAX_CONTEXT, 12_288, 8_192, MIN_CONTEXT];
    for step in steps {
        if step <= ceiling && affordable >= step {
            return step;
        }
    }
    MIN_CONTEXT
}

/// Longitud máxima de respuesta. Una ventana estrecha no puede gastar la mitad
/// en la respuesta o no queda sitio para la conversación. Criterio de Rebost.
fn output_cap_for(context_tokens: u32) -> u32 {
    match context_tokens {
        c if c >= 16_384 => 2_048,
        c if c >= 8_192 => 1_536,
        _ => 768,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::hardware::{GpuInfo, GpuKind};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn hw(
        ram_gib: u64,
        cores: u32,
        vram_gib: u64,
        accel: Accelerator,
        unified: bool,
    ) -> HardwareProfile {
        let gpus = if vram_gib > 0 {
            vec![GpuInfo {
                vendor: if unified {
                    "Apple".into()
                } else {
                    "NVIDIA".into()
                },
                name: "GPU".into(),
                vram_bytes: Some(vram_gib * GIB),
                kind: if unified {
                    GpuKind::Unified
                } else {
                    GpuKind::Discrete
                },
                source: "test".into(),
            }]
        } else {
            vec![]
        };
        HardwareProfile {
            os: "Test".into(),
            os_version: "1".into(),
            hostname: "pc".into(),
            arch: "x86_64".into(),
            cpu_brand: "Test".into(),
            physical_cores: cores,
            logical_cores: cores * 2,
            total_ram_bytes: ram_gib * GIB,
            available_ram_bytes: ram_gib * GIB / 2,
            free_disk_bytes: 400 * GIB,
            total_disk_bytes: 512 * GIB,
            usable_vram_bytes: gpus
                .iter()
                .map(|g| g.usable_vram_bytes())
                .max()
                .unwrap_or(0),
            gpus,
            accelerator: accel,
            unified_memory: unified,
        }
    }

    fn model(id: &str) -> ModelSpec {
        Catalog::embedded().get(id).unwrap().clone()
    }

    #[test]
    fn cpu_only_machine_keeps_every_layer_on_the_processor() {
        let t = Tuning::derive(
            &model("qwen2.5-1.5b-instruct-q4km"),
            &hw(16, 8, 0, Accelerator::Cpu, false),
        );
        assert_eq!(t.gpu_layers, 0);
        assert!(t.use_mmap, "sin GPU, el mmap ahorra memoria");
        assert_eq!(t.flash_attention, "auto");
    }

    #[test]
    fn a_model_that_fits_in_vram_offloads_everything() {
        let t = Tuning::derive(
            &model("qwen2.5-7b-instruct-q4km"),
            &hw(32, 8, 16, Accelerator::Cuda, false),
        );
        assert_eq!(t.gpu_layers, 999);
        assert!(!t.use_mmap, "CUDA copia los pesos: el mmap sobra");
        assert_eq!(t.flash_attention, "on");
    }

    #[test]
    fn a_model_too_big_for_the_gpu_offloads_only_some_layers() {
        let m = model("qwen2.5-14b-instruct-q4km");
        let t = Tuning::derive(&m, &hw(64, 16, 6, Accelerator::Cuda, false));
        assert!(t.gpu_layers > 0, "6 GB alojan algunas capas");
        assert!(
            t.gpu_layers < m.kv.layers,
            "pero no las {} capas",
            m.kv.layers
        );
    }

    #[test]
    fn unified_memory_offloads_everything() {
        let t = Tuning::derive(
            &model("qwen2.5-14b-instruct-q4km"),
            &hw(36, 12, 36, Accelerator::Metal, true),
        );
        assert_eq!(t.gpu_layers, 999);
        assert!(t.use_mmap, "Metal mantiene el mmap");
    }

    #[test]
    fn opencl_avoids_the_quantised_kv_cache() {
        let t = Tuning::derive(
            &model("qwen2.5-3b-instruct-q4km"),
            &hw(16, 8, 0, Accelerator::OpenCl, false),
        );
        assert_eq!(t.kv_cache_type, "f16", "q8_0 cuelga en Adreno");
    }

    #[test]
    fn context_never_leaves_the_documented_range() {
        let cat = Catalog::embedded();
        for m in &cat.models {
            for machine in [
                hw(8, 4, 0, Accelerator::Cpu, false),
                hw(16, 8, 0, Accelerator::Cpu, false),
                hw(64, 16, 24, Accelerator::Cuda, false),
                hw(36, 12, 36, Accelerator::Metal, true),
            ] {
                let t = Tuning::derive(m, &machine);
                assert!(
                    (MIN_CONTEXT..=MAX_CONTEXT).contains(&t.context_tokens),
                    "{} dio un contexto de {}",
                    m.id,
                    t.context_tokens
                );
            }
        }
    }

    #[test]
    fn context_never_exceeds_the_model_window() {
        let m = model("gemma-2-9b-it-q4km"); // ventana de 8k
        let t = Tuning::derive(&m, &hw(64, 16, 24, Accelerator::Cuda, false));
        assert!(t.context_tokens <= m.context_window);
    }

    #[test]
    fn a_roomier_machine_gets_at_least_as_much_context() {
        let m = model("qwen2.5-7b-instruct-q4km");
        let small = Tuning::derive(&m, &hw(16, 8, 0, Accelerator::Cpu, false));
        let big = Tuning::derive(&m, &hw(64, 16, 0, Accelerator::Cpu, false));
        assert!(big.context_tokens >= small.context_tokens);
    }

    #[test]
    fn tuning_is_deterministic() {
        let m = model("qwen2.5-7b-instruct-q4km");
        let machine = hw(32, 8, 12, Accelerator::Cuda, false);
        assert_eq!(Tuning::derive(&m, &machine), Tuning::derive(&m, &machine));
    }

    #[test]
    fn threads_leave_one_core_for_the_interface() {
        let t = Tuning::derive(
            &model("qwen2.5-3b-instruct-q4km"),
            &hw(16, 8, 0, Accelerator::Cpu, false),
        );
        assert_eq!(t.threads, 7);
    }

    #[test]
    fn a_single_core_machine_still_gets_one_thread() {
        let t = Tuning::derive(
            &model("qwen2.5-0.5b-instruct-q4km"),
            &hw(4, 1, 0, Accelerator::Cpu, false),
        );
        assert_eq!(t.threads, 1);
    }

    #[test]
    fn output_cap_scales_with_the_window() {
        assert_eq!(output_cap_for(4_096), 768);
        assert_eq!(output_cap_for(8_192), 1_536);
        assert_eq!(output_cap_for(16_384), 2_048);
    }

    #[test]
    fn server_args_carry_everything_the_engine_needs() {
        let t = Tuning::derive(
            &model("qwen2.5-7b-instruct-q4km"),
            &hw(32, 8, 16, Accelerator::Cuda, false),
        );
        let args = t.llama_server_args("/data/m.gguf", 8123);
        let joined = args.join(" ");
        assert!(joined.contains("--model /data/m.gguf"));
        assert!(
            joined.contains("--host 127.0.0.1"),
            "el motor no puede escuchar fuera"
        );
        assert!(joined.contains("--port 8123"));
        assert!(joined.contains("--ctx-size"));
        assert!(joined.contains("--no-mmap"));
    }

    #[test]
    fn server_args_omit_no_mmap_when_mmap_is_wanted() {
        let t = Tuning::derive(
            &model("qwen2.5-1.5b-instruct-q4km"),
            &hw(16, 8, 0, Accelerator::Cpu, false),
        );
        assert!(!t
            .llama_server_args("/m.gguf", 1)
            .join(" ")
            .contains("--no-mmap"));
    }
}
