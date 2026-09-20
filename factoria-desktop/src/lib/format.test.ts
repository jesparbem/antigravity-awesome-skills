import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatContext,
  formatDuration,
  formatEta,
  formatPercent,
  reasonText,
} from "./format";

describe("formatBytes", () => {
  it("coincide con el formato del núcleo", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5 MB");
    expect(formatBytes(8_988 * 1024 * 1024)).toBe("8,78 GB");
    expect(formatBytes(24 * 1024 * 1024 * 1024)).toBe("24 GB");
  });

  it("no inventa cifras cuando no hay dato", () => {
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(undefined)).toBe("—");
    expect(formatBytes(Number.NaN)).toBe("—");
  });
});

describe("formatContext", () => {
  it("resume la ventana en miles de tokens", () => {
    expect(formatContext(32768)).toBe("32K");
    expect(formatContext(8192)).toBe("8K");
    expect(formatContext(512)).toBe("512");
  });
});

describe("formatDuration", () => {
  it("cambia de unidad según la magnitud", () => {
    expect(formatDuration(420)).toBe("420 ms");
    expect(formatDuration(2500)).toBe("2,5 s");
    expect(formatDuration(95_000)).toBe("1 min 35 s");
  });
});

describe("formatEta", () => {
  it("estima lo que queda", () => {
    expect(formatEta(0, 1000, 100)).toBe("10 s restantes");
    expect(formatEta(0, 600_000, 1000)).toBe("10 min restantes");
  });

  it("no muestra nada cuando no puede estimar", () => {
    expect(formatEta(500, 500, 100)).toBe("");
    expect(formatEta(0, 1000, 0)).toBe("");
    expect(formatEta(0, 0, 100)).toBe("");
  });
});

describe("formatPercent", () => {
  it("redondea", () => {
    expect(formatPercent(42.4)).toBe("42 %");
  });
});

describe("reasonText", () => {
  it("traduce las razones simples a lenguaje llano", () => {
    expect(reasonText("cpuOnly")).toContain("procesador");
    expect(reasonText("fitsInVram")).toContain("tarjeta gráfica");
    expect(reasonText({ unifiedMemory: null })).toContain("unificada");
  });

  it("explica la falta de memoria con las cifras reales", () => {
    const text = reasonText({
      exceedsRam: { needBytes: 12 * 1024 ** 3, budgetBytes: 5 * 1024 ** 3 },
    });
    expect(text).toContain("12 GB");
    expect(text).toContain("5,00 GB");
  });

  it("explica la falta de disco", () => {
    const text = reasonText({
      notEnoughDisk: { needBytes: 9 * 1024 ** 3, freeBytes: 1024 ** 3 },
    });
    expect(text).toContain("ocupa");
    expect(text).toContain("libres");
  });

  it("nombra el motor que falta", () => {
    expect(reasonText({ runtimeUnavailable: { runtime: "Ollama" } })).toContain("Ollama");
  });

  it("no rompe con una razón desconocida", () => {
    expect(reasonText({ algoNuevo: {} })).toBe("algoNuevo");
    expect(reasonText(null)).toBe("");
  });
});
