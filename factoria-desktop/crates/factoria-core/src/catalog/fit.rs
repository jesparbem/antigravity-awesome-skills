//! Clasificación de aptitud de un modelo en un equipo concreto.
//!
//! Los criterios están descritos en `docs/ARCHITECTURE.md` §6 y son
//! **deterministas**: el mismo perfil de hardware y el mismo modelo dan siempre
//! la misma etiqueta. Cada veredicto lleva sus razones, que la interfaz muestra
//! literalmente — el empleado nunca ve solo un semáforo sin explicación.

use super::ModelSpec;
use crate::hardware::{Accelerator, HardwareProfile};
use serde::{Deserialize, Serialize};

const GIB: u64 = 1024 * 1024 * 1024;

/// Overhead fijo del proceso del runtime (grafo de cómputo, buffers, tokenizador).
const RUNTIME_OVERHEAD_BYTES: u64 = 1024 * 1024 * 1024;

/// Contexto con el que se dimensiona la aptitud. No es el máximo del modelo: es
/// lo que FactorIA arranca por defecto (ver `runtime::tuning`). Clasificar con el
/// contexto máximo declararía "no recomendado" modelos que funcionan de sobra.
pub const FIT_CONTEXT_TOKENS: u32 = 8_192;

/// Por debajo de este número de núcleos, y sin GPU, un modelo grande es
/// inutilizable en la práctica aunque la RAM dé la cifra.
const MIN_CORES_FOR_LARGE_CPU_ONLY: u32 = 4;
/// Umbral de "grande" para la regla anterior.
const LARGE_ON_CPU_BYTES: u64 = 3 * GIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FitLevel {
    /// Cabe en memoria y la GPU puede alojarlo entero (o el equipo es de memoria
    /// unificada). Se ejecutará a buena velocidad.
    Optimal,
    /// Cabe en memoria y en disco, pero se ejecutará en CPU o con descarga
    /// parcial a GPU. Funciona; irá más lento.
    Compatible,
    /// No se recomienda ejecutarlo en este equipo.
    NotRecommended,
}

impl FitLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Optimal => "Óptimo",
            Self::Compatible => "Compatible",
            Self::NotRecommended => "No recomendado",
        }
    }

    /// Orden para la lista: primero lo que mejor encaja.
    pub fn rank(self) -> u8 {
        match self {
            Self::Optimal => 0,
            Self::Compatible => 1,
            Self::NotRecommended => 2,
        }
    }
}

/// Motivo concreto, en un vocabulario cerrado para poder traducirlo y probarlo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FitReason {
    FitsInVram,
    UnifiedMemory,
    FitsInRam,
    ExceedsRam { need_bytes: u64, budget_bytes: u64 },
    NotEnoughDisk { need_bytes: u64, free_bytes: u64 },
    CpuOnly,
    PartialGpuOffload,
    TooSlowOnCpu,
    RuntimeUnavailable { runtime: String },
}

impl FitReason {
    /// Texto que ve el empleado. Sin jerga: ni "offload", ni "VRAM insuficiente".
    pub fn message(&self) -> String {
        match self {
            Self::FitsInVram => "Cabe entero en la memoria de la tarjeta gráfica.".into(),
            Self::UnifiedMemory => {
                "Memoria unificada: la gráfica usa la misma memoria del equipo.".into()
            }
            Self::FitsInRam => "Cabe en la memoria del equipo.".into(),
            Self::ExceedsRam {
                need_bytes,
                budget_bytes,
            } => format!(
                "Necesita unos {} y este equipo puede dedicar {}.",
                crate::format_bytes(*need_bytes),
                crate::format_bytes(*budget_bytes)
            ),
            Self::NotEnoughDisk {
                need_bytes,
                free_bytes,
            } => format!(
                "La descarga ocupa {} y quedan {} libres.",
                crate::format_bytes(*need_bytes),
                crate::format_bytes(*free_bytes)
            ),
            Self::CpuOnly => "Se ejecutará con el procesador; irá más lento.".into(),
            Self::PartialGpuOffload => "La gráfica alojará solo una parte del modelo.".into(),
            Self::TooSlowOnCpu => {
                "Sin gráfica y con pocos núcleos, la respuesta sería demasiado lenta.".into()
            }
            Self::RuntimeUnavailable { runtime } => {
                format!("Requiere {runtime}, que no está disponible en este equipo.")
            }
        }
    }
}

/// Desglose de memoria, para que la cifra sea auditable desde la interfaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEstimate {
    pub weights_bytes: u64,
    pub weight_copies_pct: u32,
    pub kv_cache_bytes: u64,
    pub overhead_bytes: u64,
    pub total_bytes: u64,
    pub context_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FitVerdict {
    pub level: FitLevel,
    pub reasons: Vec<FitReason>,
    pub memory: MemoryEstimate,
    /// Estimación orientativa de velocidad, antes de ejecutar. Se sustituye por
    /// la medición real en cuanto el modelo responde una vez.
    pub estimated_tokens_per_second: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFit {
    pub model: ModelSpec,
    pub verdict: FitVerdict,
}

/// Memoria que necesita este modelo en este equipo, con el contexto de arranque.
pub fn memory_estimate(model: &ModelSpec, hw: &HardwareProfile) -> MemoryEstimate {
    let context_tokens = FIT_CONTEXT_TOKENS.min(model.context_window);
    let copies = hw.accelerator.weight_copies();
    let weights = (model.file_bytes as f64 * copies) as u64;
    let kv = model.kv.kv_cache_bytes(context_tokens);
    MemoryEstimate {
        weights_bytes: weights,
        weight_copies_pct: (copies * 100.0) as u32,
        kv_cache_bytes: kv,
        overhead_bytes: RUNTIME_OVERHEAD_BYTES,
        total_bytes: weights + kv + RUNTIME_OVERHEAD_BYTES,
        context_tokens,
    }
}

/// Tokens/s orientativos: los pesos se recorren una vez por token, así que el
/// ancho de banda de memoria domina. Es una cota superior optimista dividida por
/// un factor de eficiencia; sirve para ordenar y ambientar, no para prometer.
fn estimate_tokens_per_second(model: &ModelSpec, hw: &HardwareProfile, gpu_resident: bool) -> f32 {
    let bandwidth_gbs = if gpu_resident {
        hw.accelerator.typical_bandwidth_gbs()
    } else {
        Accelerator::Cpu.typical_bandwidth_gbs()
    };
    let weights_gb = model.file_bytes as f64 / GIB as f64;
    if weights_gb <= 0.0 {
        return 0.0;
    }
    let efficiency = 0.45;
    let tps = bandwidth_gbs * efficiency / weights_gb;
    // Un equipo con pocos núcleos no alcanza el ancho de banda teórico en CPU.
    let core_factor = if gpu_resident {
        1.0
    } else {
        (hw.physical_cores as f64 / 8.0).clamp(0.25, 1.0)
    };
    ((tps * core_factor) as f32).clamp(0.1, 400.0)
}

/// Clasifica un modelo en un equipo. `available_runtimes` son los identificadores
/// de runtime que ahora mismo se pueden usar (`["llamacpp"]`, `["llamacpp","ollama"]`…).
pub fn fit(model: &ModelSpec, hw: &HardwareProfile, available_runtimes: &[String]) -> FitVerdict {
    let memory = memory_estimate(model, hw);
    let mut reasons = Vec::new();

    // 1. Disco: si no cabe la descarga, no hay nada más que discutir.
    if model.file_bytes > hw.disk_budget_bytes() {
        reasons.push(FitReason::NotEnoughDisk {
            need_bytes: model.file_bytes,
            free_bytes: hw.free_disk_bytes,
        });
        return FitVerdict {
            level: FitLevel::NotRecommended,
            reasons,
            memory,
            estimated_tokens_per_second: 0.0,
        };
    }

    // 2. Runtime: un modelo que ningún motor disponible puede ejecutar no es
    //    "lento", es imposible. La razón que se muestra es accionable.
    let runtime_ok = model
        .runtimes
        .iter()
        .any(|r| available_runtimes.iter().any(|a| a == r));
    if !runtime_ok {
        let needed = model.runtimes.first().cloned().unwrap_or_default();
        reasons.push(FitReason::RuntimeUnavailable {
            runtime: runtime_label(&needed),
        });
        return FitVerdict {
            level: FitLevel::NotRecommended,
            reasons,
            memory,
            estimated_tokens_per_second: 0.0,
        };
    }

    // 3. Memoria del sistema.
    let ram_budget = hw.ram_budget_bytes();
    if memory.total_bytes > ram_budget {
        reasons.push(FitReason::ExceedsRam {
            need_bytes: memory.total_bytes,
            budget_bytes: ram_budget,
        });
        return FitVerdict {
            level: FitLevel::NotRecommended,
            reasons,
            memory,
            estimated_tokens_per_second: 0.0,
        };
    }
    reasons.push(FitReason::FitsInRam);

    // 4. ¿Puede vivir entero en la gráfica?
    let gpu_resident = if hw.unified_memory {
        reasons.push(FitReason::UnifiedMemory);
        true
    } else if hw.usable_vram_bytes > 0
        && memory.weights_bytes + memory.kv_cache_bytes <= hw.vram_budget_bytes()
    {
        reasons.push(FitReason::FitsInVram);
        true
    } else if hw.usable_vram_bytes > 0 {
        reasons.push(FitReason::PartialGpuOffload);
        false
    } else {
        reasons.push(FitReason::CpuOnly);
        false
    };

    let tps = estimate_tokens_per_second(model, hw, gpu_resident);

    // 5. Degradado por CPU insuficiente: la RAM da el número, pero el equipo no
    //    daría una experiencia utilizable.
    if !gpu_resident
        && hw.usable_vram_bytes == 0
        && hw.physical_cores < MIN_CORES_FOR_LARGE_CPU_ONLY
        && model.file_bytes > LARGE_ON_CPU_BYTES
    {
        reasons.push(FitReason::TooSlowOnCpu);
        return FitVerdict {
            level: FitLevel::NotRecommended,
            reasons,
            memory,
            estimated_tokens_per_second: tps,
        };
    }

    FitVerdict {
        level: if gpu_resident {
            FitLevel::Optimal
        } else {
            FitLevel::Compatible
        },
        reasons,
        memory,
        estimated_tokens_per_second: tps,
    }
}

fn runtime_label(id: &str) -> String {
    match id {
        "llamacpp" => "el motor local de FactorIA".into(),
        "ollama" => "Ollama".into(),
        other => other.to_string(),
    }
}

/// Clasifica el catálogo entero y lo ordena: primero lo que mejor encaja, y
/// dentro de cada nivel, lo más capaz.
pub fn classify_all(
    models: &[ModelSpec],
    hw: &HardwareProfile,
    available_runtimes: &[String],
) -> Vec<ModelFit> {
    let mut out: Vec<ModelFit> = models
        .iter()
        .map(|m| ModelFit {
            verdict: fit(m, hw, available_runtimes),
            model: m.clone(),
        })
        .collect();
    out.sort_by(|a, b| {
        a.verdict
            .level
            .rank()
            .cmp(&b.verdict.level.rank())
            .then(b.model.capability.cmp(&a.model.capability))
    });
    out
}

/// El modelo recomendado: el más capaz que sea *Óptimo*; si ninguno lo es, el
/// más capaz *Compatible*. Nunca devuelve un *No recomendado*.
pub fn recommend(
    models: &[ModelSpec],
    hw: &HardwareProfile,
    available_runtimes: &[String],
) -> Option<ModelFit> {
    let classified = classify_all(models, hw, available_runtimes);
    classified
        .iter()
        .find(|f| f.verdict.level == FitLevel::Optimal)
        .or_else(|| {
            classified
                .iter()
                .find(|f| f.verdict.level == FitLevel::Compatible)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::hardware::{GpuInfo, GpuKind};

    fn hw(ram_gib: u64, cores: u32, gpus: Vec<GpuInfo>, accel: Accelerator) -> HardwareProfile {
        let unified = gpus.iter().any(|g| g.kind == GpuKind::Unified);
        let usable = gpus
            .iter()
            .map(|g| g.usable_vram_bytes())
            .max()
            .unwrap_or(0);
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
            gpus,
            accelerator: accel,
            unified_memory: unified,
            usable_vram_bytes: usable,
        }
    }

    fn nvidia(vram_gib: u64) -> Vec<GpuInfo> {
        vec![GpuInfo {
            vendor: "NVIDIA".into(),
            name: format!("NVIDIA GeForce RTX ({vram_gib} GB)"),
            vram_bytes: Some(vram_gib * GIB),
            kind: GpuKind::Discrete,
            source: "test".into(),
        }]
    }

    fn apple(ram_gib: u64) -> Vec<GpuInfo> {
        vec![GpuInfo {
            vendor: "Apple".into(),
            name: "Apple M3".into(),
            vram_bytes: Some(ram_gib * GIB),
            kind: GpuKind::Unified,
            source: "test".into(),
        }]
    }

    fn runtimes() -> Vec<String> {
        vec!["llamacpp".to_string()]
    }

    fn model(id: &str) -> ModelSpec {
        Catalog::embedded()
            .get(id)
            .expect("modelo del catálogo")
            .clone()
    }

    #[test]
    fn small_model_on_a_modest_office_laptop_is_compatible() {
        // 8 GB, sin GPU, 4 núcleos: el portátil corporativo de gama baja.
        let hw = hw(8, 4, vec![], Accelerator::Cpu);
        let v = fit(&model("qwen2.5-1.5b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::Compatible);
        assert!(v.reasons.contains(&FitReason::CpuOnly));
    }

    #[test]
    fn large_model_on_8gb_is_not_recommended_with_a_memory_reason() {
        let hw = hw(8, 4, vec![], Accelerator::Cpu);
        let v = fit(&model("qwen2.5-14b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::NotRecommended);
        assert!(matches!(v.reasons[0], FitReason::ExceedsRam { .. }));
        assert!(v.reasons[0].message().contains("Necesita"));
    }

    #[test]
    fn model_fully_resident_in_vram_is_optimal() {
        let hw = hw(32, 8, nvidia(12), Accelerator::Cuda);
        let v = fit(&model("qwen2.5-7b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::Optimal);
        assert!(v.reasons.contains(&FitReason::FitsInVram));
    }

    #[test]
    fn model_too_big_for_the_gpu_but_fitting_in_ram_is_compatible() {
        // 6 GB de VRAM no alojan un 14B; 64 GB de RAM sí lo sostienen.
        let hw = hw(64, 8, nvidia(6), Accelerator::Cuda);
        let v = fit(&model("qwen2.5-14b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::Compatible);
        assert!(v.reasons.contains(&FitReason::PartialGpuOffload));
    }

    #[test]
    fn unified_memory_makes_it_optimal_without_a_discrete_gpu() {
        let hw = hw(36, 12, apple(36), Accelerator::Metal);
        let v = fit(&model("qwen2.5-14b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::Optimal);
        assert!(v.reasons.contains(&FitReason::UnifiedMemory));
    }

    #[test]
    fn no_disk_space_beats_every_other_criterion() {
        let mut hw = hw(64, 16, nvidia(24), Accelerator::Cuda);
        hw.free_disk_bytes = 3 * GIB; // menos que el modelo + la reserva
        let v = fit(&model("qwen2.5-14b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::NotRecommended);
        assert!(matches!(v.reasons[0], FitReason::NotEnoughDisk { .. }));
    }

    #[test]
    fn a_model_whose_runtime_is_missing_is_not_recommended_with_an_actionable_reason() {
        let hw = hw(32, 8, nvidia(16), Accelerator::Cuda);
        let mut m = model("qwen2.5-7b-instruct-q4km");
        m.runtimes = vec!["ollama".into()];
        let v = fit(&m, &hw, &runtimes());
        assert_eq!(v.level, FitLevel::NotRecommended);
        assert!(v.reasons[0].message().contains("Ollama"));
    }

    #[test]
    fn weak_cpu_downgrades_a_large_model_that_ram_would_allow() {
        // 16 GB da la cifra para un 7B, pero 2 núcleos y sin GPU no.
        let hw = hw(16, 2, vec![], Accelerator::Cpu);
        let v = fit(&model("qwen2.5-7b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::NotRecommended);
        assert!(v.reasons.contains(&FitReason::TooSlowOnCpu));
    }

    #[test]
    fn weak_cpu_still_allows_a_small_model() {
        let hw = hw(16, 2, vec![], Accelerator::Cpu);
        let v = fit(&model("qwen2.5-1.5b-instruct-q4km"), &hw, &runtimes());
        assert_eq!(v.level, FitLevel::Compatible);
    }

    #[test]
    fn classification_is_deterministic() {
        let hw = hw(16, 8, vec![], Accelerator::Cpu);
        let cat = Catalog::embedded();
        let a = classify_all(&cat.models, &hw, &runtimes());
        let b = classify_all(&cat.models, &hw, &runtimes());
        assert_eq!(a, b);
    }

    #[test]
    fn every_covered_ram_band_gets_a_recommendation() {
        let cat = Catalog::embedded();
        for &band in crate::catalog::data::COVERED_RAM_BANDS_GIB {
            let hw = hw(band, 8, vec![], Accelerator::Cpu);
            let rec = recommend(&cat.models, &hw, &runtimes());
            assert!(rec.is_some(), "sin recomendación con {band} GB de RAM");
            assert_ne!(rec.unwrap().verdict.level, FitLevel::NotRecommended);
        }
    }

    #[test]
    fn a_bigger_machine_never_gets_a_weaker_recommendation() {
        let cat = Catalog::embedded();
        let small = recommend(
            &cat.models,
            &hw(8, 8, vec![], Accelerator::Cpu),
            &runtimes(),
        )
        .unwrap();
        let big = recommend(
            &cat.models,
            &hw(64, 16, nvidia(24), Accelerator::Cuda),
            &runtimes(),
        )
        .unwrap();
        assert!(big.model.capability >= small.model.capability);
    }

    #[test]
    fn a_machine_that_cannot_run_anything_gets_no_recommendation() {
        let mut hw = hw(2, 2, vec![], Accelerator::Cpu);
        hw.free_disk_bytes = GIB;
        assert!(recommend(&Catalog::embedded().models, &hw, &runtimes()).is_none());
    }

    #[test]
    fn classification_is_sorted_best_first() {
        let hw = hw(32, 8, nvidia(12), Accelerator::Cuda);
        let list = classify_all(&Catalog::embedded().models, &hw, &runtimes());
        let ranks: Vec<u8> = list.iter().map(|f| f.verdict.level.rank()).collect();
        let mut sorted = ranks.clone();
        sorted.sort_unstable();
        assert_eq!(ranks, sorted);
    }

    #[test]
    fn memory_estimate_accounts_for_copies_kv_and_overhead() {
        let cuda = hw(64, 16, nvidia(24), Accelerator::Cuda);
        let cpu = hw(64, 16, vec![], Accelerator::Cpu);
        let m = model("qwen2.5-7b-instruct-q4km");
        let on_cuda = memory_estimate(&m, &cuda);
        let on_cpu = memory_estimate(&m, &cpu);
        assert!(
            on_cuda.weights_bytes > on_cpu.weights_bytes,
            "CUDA presupuesta dos copias de los pesos"
        );
        assert_eq!(on_cpu.overhead_bytes, RUNTIME_OVERHEAD_BYTES);
        assert_eq!(on_cpu.context_tokens, FIT_CONTEXT_TOKENS);
        assert_eq!(
            on_cpu.total_bytes,
            on_cpu.weights_bytes + on_cpu.kv_cache_bytes + on_cpu.overhead_bytes
        );
    }

    #[test]
    fn fit_context_never_exceeds_the_model_window() {
        let mut m = model("qwen2.5-7b-instruct-q4km");
        m.context_window = 4096;
        let e = memory_estimate(&m, &hw(16, 8, vec![], Accelerator::Cpu));
        assert_eq!(e.context_tokens, 4096);
    }

    #[test]
    fn gpu_resident_is_estimated_faster_than_cpu() {
        let m = model("qwen2.5-7b-instruct-q4km");
        let gpu = fit(&m, &hw(32, 8, nvidia(16), Accelerator::Cuda), &runtimes());
        let cpu = fit(&m, &hw(32, 8, vec![], Accelerator::Cpu), &runtimes());
        assert!(gpu.estimated_tokens_per_second > cpu.estimated_tokens_per_second);
        assert!(cpu.estimated_tokens_per_second > 0.0);
    }

    #[test]
    fn a_smaller_model_is_estimated_faster_than_a_bigger_one() {
        let hw = hw(64, 16, nvidia(24), Accelerator::Cuda);
        let small = fit(&model("qwen2.5-1.5b-instruct-q4km"), &hw, &runtimes());
        let big = fit(&model("qwen2.5-14b-instruct-q4km"), &hw, &runtimes());
        assert!(small.estimated_tokens_per_second > big.estimated_tokens_per_second);
    }
}
