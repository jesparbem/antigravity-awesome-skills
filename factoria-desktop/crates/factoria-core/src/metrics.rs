//! Métricas de generación y de recursos.
//!
//! Las cifras que la interfaz muestra tras cada respuesta salen de aquí y son
//! **medidas**, no estimadas. La estimación previa del catálogo vive en
//! `catalog::fit`.

use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationMetrics {
    /// Tiempo hasta el primer token, en milisegundos.
    pub time_to_first_token_ms: u64,
    pub total_ms: u64,
    pub output_tokens: u32,
    pub tokens_per_second: f32,
}

/// Cronómetro de una generación. Cuenta tokens por *deltas* recibidos, que es lo
/// que el runtime emite; para llama.cpp un delta es un token.
pub struct GenerationTimer {
    started: Instant,
    first_token: Option<Instant>,
    tokens: u32,
}

impl Default for GenerationTimer {
    fn default() -> Self {
        Self::start()
    }
}

impl GenerationTimer {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            first_token: None,
            tokens: 0,
        }
    }

    pub fn token(&mut self) {
        if self.first_token.is_none() {
            self.first_token = Some(Instant::now());
        }
        self.tokens += 1;
    }

    pub fn finish(self) -> GenerationMetrics {
        let now = Instant::now();
        let total_ms = now.duration_since(self.started).as_millis() as u64;
        let ttft = self
            .first_token
            .map(|t| t.duration_since(self.started).as_millis() as u64)
            .unwrap_or(total_ms);
        // La velocidad se mide desde el primer token: el tiempo de carga del
        // modelo no es velocidad de generación.
        let gen_ms = total_ms.saturating_sub(ttft).max(1);
        let tps = if self.tokens > 1 {
            (self.tokens - 1) as f32 * 1000.0 / gen_ms as f32
        } else {
            0.0
        };
        GenerationMetrics {
            time_to_first_token_ms: ttft,
            total_ms,
            output_tokens: self.tokens,
            tokens_per_second: tps,
        }
    }
}

/// Uso de recursos en un instante, para el panel de la Home.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSample {
    pub cpu_percent: f32,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub disk_free_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generation_with_no_tokens_reports_zero_speed() {
        let m = GenerationTimer::start().finish();
        assert_eq!(m.output_tokens, 0);
        assert_eq!(m.tokens_per_second, 0.0);
    }

    #[test]
    fn tokens_are_counted_and_speed_is_positive() {
        let mut t = GenerationTimer::start();
        for _ in 0..10 {
            t.token();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let m = t.finish();
        assert_eq!(m.output_tokens, 10);
        assert!(
            m.tokens_per_second > 0.0,
            "diez tokens en ~20 ms deben dar velocidad"
        );
        assert!(m.total_ms >= m.time_to_first_token_ms);
    }

    #[test]
    fn a_single_token_does_not_fabricate_a_rate() {
        let mut t = GenerationTimer::start();
        t.token();
        let m = t.finish();
        assert_eq!(m.output_tokens, 1);
        assert_eq!(
            m.tokens_per_second, 0.0,
            "con un solo token no hay intervalo que medir"
        );
    }
}
