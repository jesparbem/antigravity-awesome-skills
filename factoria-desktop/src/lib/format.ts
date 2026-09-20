/** Formateo para personas. Mismo criterio que `factoria_core::format_bytes`. */

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || Number.isNaN(bytes)) return "—";
  const KB = 1024;
  const MB = KB * 1024;
  const GB = MB * 1024;
  if (bytes >= GB) {
    const v = bytes / GB;
    return v >= 10 ? `${Math.round(v)} GB` : `${v.toFixed(2).replace(".", ",")} GB`;
  }
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
  if (bytes >= KB) return `${Math.round(bytes / KB)} KB`;
  return `${bytes} B`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

/** Ventana de contexto en miles de tokens: `32768` → `32K`. */
export function formatContext(tokens: number): string {
  if (tokens >= 1024) return `${Math.round(tokens / 1024)}K`;
  return String(tokens);
}

export function formatPercent(value: number): string {
  return `${Math.round(value)} %`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1).replace(".", ",")} s`;
  const minutes = Math.floor(ms / 60_000);
  return `${minutes} min ${Math.round((ms % 60_000) / 1000)} s`;
}

/** Tiempo restante de una descarga. */
export function formatEta(receivedBytes: number, totalBytes: number, speed: number): string {
  if (!totalBytes || speed <= 0 || receivedBytes >= totalBytes) return "";
  const seconds = (totalBytes - receivedBytes) / speed;
  if (seconds < 60) return `${Math.ceil(seconds)} s restantes`;
  return `${Math.ceil(seconds / 60)} min restantes`;
}

export function formatDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat("es-ES", {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export const FIT_LABELS: Record<string, string> = {
  optimal: "Óptimo",
  compatible: "Compatible",
  notRecommended: "No recomendado",
};

export const STATE_LABELS: Record<string, string> = {
  notInstalled: "No instalado",
  installing: "Instalando",
  installed: "Instalado",
  running: "En ejecución",
};

export const ACCELERATOR_LABELS: Record<string, string> = {
  metal: "Metal",
  cuda: "CUDA",
  vulkan: "Vulkan",
  openCl: "OpenCL",
  cpu: "CPU",
};

/**
 * Texto de una razón de aptitud.
 *
 * El núcleo envía un vocabulario cerrado; aquí se traduce a la frase que ve el
 * empleado, sin jerga técnica.
 */
export function reasonText(reason: unknown): string {
  if (typeof reason === "string") return REASON_TEXTS[reason] ?? reason;
  if (reason && typeof reason === "object") {
    const [key, value] = Object.entries(reason as Record<string, unknown>)[0] ?? [];
    if (!key) return "";
    const data = (value ?? {}) as Record<string, number>;
    switch (key) {
      case "exceedsRam":
        return `Necesita unos ${formatBytes(data.needBytes)} y este equipo puede dedicar ${formatBytes(data.budgetBytes)}.`;
      case "notEnoughDisk":
        return `La descarga ocupa ${formatBytes(data.needBytes)} y quedan ${formatBytes(data.freeBytes)} libres.`;
      case "runtimeUnavailable":
        return `Requiere ${(value as unknown as { runtime: string }).runtime}, que no está disponible en este equipo.`;
      default:
        return REASON_TEXTS[key] ?? key;
    }
  }
  return "";
}

const REASON_TEXTS: Record<string, string> = {
  fitsInVram: "Cabe entero en la memoria de la tarjeta gráfica.",
  unifiedMemory: "Memoria unificada: la gráfica usa la misma memoria del equipo.",
  fitsInRam: "Cabe en la memoria del equipo.",
  cpuOnly: "Se ejecutará con el procesador; irá más lento.",
  partialGpuOffload: "La gráfica alojará solo una parte del modelo.",
  tooSlowOnCpu: "Sin gráfica y con pocos núcleos, la respuesta sería demasiado lenta.",
};
