//! Enumeración de GPU y VRAM sin dependencias de vendor.
//!
//! Se intentan varias fuentes en orden y la primera que responde gana. Si ninguna
//! responde, la lista queda vacía: es un degradado válido, no un error — la
//! clasificación de modelos sigue funcionando con RAM y CPU.
//!
//! Los *parsers* están separados de la ejecución de los procesos para poder
//! probarlos con salidas reales fijadas, sin necesidad de la herramienta ni del
//! hardware.

use serde::{Deserialize, Serialize};

const MIB: u64 = 1024 * 1024;

/// `Win32_VideoController.AdapterRAM` es un `uint32`, así que una tarjeta de
/// 8 o 24 GB se reporta saturada. Windows devuelve `0xFFF0_0000` (4.095 MiB) en
/// ese caso: a partir de ahí el valor no dice nada sobre la VRAM real y se
/// descarta en lugar de mentir.
const ADAPTER_RAM_SATURATION: u64 = 0xFFF0_0000;

/// Tipo de memoria de la GPU, que decide cómo se cuenta la VRAM frente a la RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuKind {
    /// GPU dedicada con su propia memoria (NVIDIA/AMD/Intel Arc discretas).
    Discrete,
    /// GPU integrada que comparte la RAM del sistema.
    Integrated,
    /// Memoria unificada (Apple Silicon): VRAM y RAM son lo mismo.
    Unified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub vendor: String,
    pub name: String,
    /// `None` cuando la fuente no publica la cantidad de memoria.
    pub vram_bytes: Option<u64>,
    pub kind: GpuKind,
    /// De qué fuente salió el dato, para poder explicarlo en Diagnóstico.
    pub source: String,
}

impl GpuInfo {
    /// VRAM realmente utilizable para alojar pesos. Una GPU integrada no aporta
    /// presupuesto propio: su memoria ya está contada en la RAM del sistema.
    pub fn usable_vram_bytes(&self) -> u64 {
        match self.kind {
            GpuKind::Discrete | GpuKind::Unified => self.vram_bytes.unwrap_or(0),
            GpuKind::Integrated => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsers (probados sin la herramienta presente)
// ---------------------------------------------------------------------------

/// `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits`
///
/// La memoria viene en MiB.
pub fn parse_nvidia_smi(out: &str) -> Vec<GpuInfo> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, mem) = line.split_once(',')?;
            let mib: u64 = mem.trim().parse().ok()?;
            Some(GpuInfo {
                vendor: "NVIDIA".into(),
                name: name.trim().to_string(),
                vram_bytes: Some(mib * MIB),
                kind: GpuKind::Discrete,
                source: "nvidia-smi".into(),
            })
        })
        .collect()
}

/// `powershell -NoProfile -Command "Get-CimInstance Win32_VideoController |
///  Select-Object Name,AdapterRAM,AdapterCompatibility | ConvertTo-Json"`
///
/// `AdapterRAM` es un `uint32` y satura en 4 GiB: por encima de ese valor el dato
/// no es fiable, así que se descarta en lugar de mentir.
pub fn parse_windows_video_controllers(json: &str) -> Vec<GpuInfo> {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let rows = match &value {
        serde_json::Value::Array(rows) => rows.clone(),
        other => vec![other.clone()],
    };
    rows.iter()
        .filter_map(|row| {
            let name = row.get("Name")?.as_str()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let vendor = row
                .get("AdapterCompatibility")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| vendor_from_name(&name));
            let raw = row.get("AdapterRAM").and_then(|v| v.as_u64());
            let vram_bytes = match raw {
                Some(0) | None => None,
                Some(bytes) if bytes >= ADAPTER_RAM_SATURATION => None,
                Some(bytes) => Some(bytes),
            };
            Some(GpuInfo {
                kind: classify_by_name(&name, &vendor),
                vendor,
                name,
                vram_bytes,
                source: "Win32_VideoController".into(),
            })
        })
        .collect()
}

/// `system_profiler SPDisplaysDataType -json`
///
/// En Apple Silicon no hay VRAM propia: la memoria es unificada y el llamante
/// sustituye el valor por la RAM total del sistema.
pub fn parse_system_profiler(json: &str) -> Vec<GpuInfo> {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(items) = value.get("SPDisplaysDataType").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item
                .get("sppci_model")
                .or_else(|| item.get("_name"))
                .and_then(|v| v.as_str())?
                .trim()
                .to_string();
            let vendor = item
                .get("spdisplays_vendor")
                .and_then(|v| v.as_str())
                .map(clean_apple_vendor)
                .unwrap_or_else(|| vendor_from_name(&name));
            let unified = item.get("spdisplays_mtlgpufamilysupport").is_some()
                && name.to_ascii_lowercase().starts_with("apple");
            let vram_bytes = item
                .get("spdisplays_vram")
                .or_else(|| item.get("spdisplays_vram_shared"))
                .and_then(|v| v.as_str())
                .and_then(parse_apple_vram);
            Some(GpuInfo {
                vendor,
                kind: if unified {
                    GpuKind::Unified
                } else {
                    classify_by_name(&name, "")
                },
                name,
                vram_bytes,
                source: "system_profiler".into(),
            })
        })
        .collect()
}

/// `"8 GB"` / `"1536 MB"` tal y como los publica `system_profiler`.
fn parse_apple_vram(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    let (num, unit) = raw.split_once(' ')?;
    let num: f64 = num.trim().parse().ok()?;
    let factor = match unit.trim().to_ascii_uppercase().as_str() {
        "GB" => 1024.0 * MIB as f64,
        "MB" => MIB as f64,
        _ => return None,
    };
    Some((num * factor) as u64)
}

fn clean_apple_vendor(raw: &str) -> String {
    // "sppci_vendor_Apple" -> "Apple"
    raw.rsplit('_').next().unwrap_or(raw).to_string()
}

/// AMD en Linux: `/sys/class/drm/card*/device/mem_info_vram_total` (bytes).
pub fn parse_sysfs_vram(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok().filter(|b| *b > 0)
}

pub(crate) fn vendor_from_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for (needle, vendor) in [
        ("nvidia", "NVIDIA"),
        ("geforce", "NVIDIA"),
        ("quadro", "NVIDIA"),
        ("rtx", "NVIDIA"),
        ("radeon", "AMD"),
        ("amd", "AMD"),
        ("intel", "Intel"),
        ("arc", "Intel"),
        ("apple", "Apple"),
        ("qualcomm", "Qualcomm"),
        ("adreno", "Qualcomm"),
    ] {
        if lower.contains(needle) {
            return vendor.to_string();
        }
    }
    "Desconocido".to_string()
}

/// Integrada o discreta a partir del nombre comercial. Se usa solo cuando la
/// fuente no lo dice; el error habitual (marcar discreta una integrada) se evita
/// siendo conservador: ante la duda, integrada.
fn classify_by_name(name: &str, vendor: &str) -> GpuKind {
    let hay = format!("{name} {vendor}").to_ascii_lowercase();
    let integrated_markers = [
        "uhd graphics",
        "hd graphics",
        "iris",
        "vega 8",
        "vega 7",
        "radeon graphics",
        "microsoft basic",
        "adreno",
        "integrated",
    ];
    if integrated_markers.iter().any(|m| hay.contains(m)) {
        return GpuKind::Integrated;
    }
    let discrete_markers = [
        "geforce",
        "rtx",
        "gtx",
        "quadro",
        "tesla",
        "radeon rx",
        "radeon pro",
        "arc a",
        "arc b",
        "firepro",
    ];
    if discrete_markers.iter().any(|m| hay.contains(m)) {
        return GpuKind::Discrete;
    }
    GpuKind::Integrated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_two_cards() {
        let out = "NVIDIA GeForce RTX 4090, 24564\nNVIDIA RTX A2000, 6144\n";
        let gpus = parse_nvidia_smi(out);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(gpus[0].vram_bytes, Some(24564 * MIB));
        assert_eq!(gpus[0].kind, GpuKind::Discrete);
        assert_eq!(gpus[1].vram_bytes, Some(6144 * MIB));
    }

    #[test]
    fn nvidia_smi_empty_or_garbage_is_no_gpu() {
        assert!(parse_nvidia_smi("").is_empty());
        assert!(parse_nvidia_smi("no devices were found\n").is_empty());
    }

    #[test]
    fn windows_single_object_not_array() {
        let json = r#"{"Name":"Intel(R) UHD Graphics 620","AdapterRAM":1073741824,"AdapterCompatibility":"Intel Corporation"}"#;
        let gpus = parse_windows_video_controllers(json);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].kind, GpuKind::Integrated);
        assert_eq!(
            gpus[0].usable_vram_bytes(),
            0,
            "una integrada no aporta VRAM propia"
        );
    }

    #[test]
    fn windows_adapter_ram_saturates_at_4gib() {
        let json = r#"[{"Name":"NVIDIA GeForce RTX 4080","AdapterRAM":4293918720,"AdapterCompatibility":"NVIDIA"}]"#;
        let gpus = parse_windows_video_controllers(json);
        assert_eq!(gpus[0].kind, GpuKind::Discrete);
        assert_eq!(
            gpus[0].vram_bytes, None,
            "AdapterRAM satura el uint32: mejor sin dato que con un dato falso"
        );
    }

    #[test]
    fn macos_apple_silicon_is_unified() {
        let json = r#"{"SPDisplaysDataType":[{"_name":"Apple M3 Pro","sppci_model":"Apple M3 Pro","spdisplays_vendor":"sppci_vendor_Apple","spdisplays_mtlgpufamilysupport":"spdisplays_metal3"}]}"#;
        let gpus = parse_system_profiler(json);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].kind, GpuKind::Unified);
        assert_eq!(gpus[0].vendor, "Apple");
    }

    #[test]
    fn macos_discrete_amd_reports_vram() {
        let json = r#"{"SPDisplaysDataType":[{"sppci_model":"AMD Radeon Pro 5500M","spdisplays_vram":"8 GB"}]}"#;
        let gpus = parse_system_profiler(json);
        assert_eq!(gpus[0].vram_bytes, Some(8 * 1024 * MIB));
        assert_eq!(gpus[0].kind, GpuKind::Discrete);
    }

    #[test]
    fn sysfs_vram_rejects_zero() {
        assert_eq!(parse_sysfs_vram("8589934592\n"), Some(8589934592));
        assert_eq!(parse_sysfs_vram("0"), None);
        assert_eq!(parse_sysfs_vram("n/a"), None);
    }

    #[test]
    fn malformed_json_is_no_gpu_not_a_panic() {
        assert!(parse_windows_video_controllers("<not json>").is_empty());
        assert!(parse_system_profiler("{}").is_empty());
    }
}
