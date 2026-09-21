/** Tipos espejo de las estructuras de `factoria-core`. */

export type FitLevel = "optimal" | "compatible" | "notRecommended";
export type ModelState = "notInstalled" | "installing" | "installed" | "starting" | "running";
export type GpuKind = "discrete" | "integrated" | "unified";
export type Accelerator = "metal" | "cuda" | "vulkan" | "openCl" | "cpu";

export interface GpuInfo {
  vendor: string;
  name: string;
  vramBytes: number | null;
  kind: GpuKind;
  source: string;
}

export interface HardwareProfile {
  os: string;
  osVersion: string;
  hostname: string;
  arch: string;
  cpuBrand: string;
  physicalCores: number;
  logicalCores: number;
  totalRamBytes: number;
  availableRamBytes: number;
  freeDiskBytes: number;
  totalDiskBytes: number;
  gpus: GpuInfo[];
  accelerator: Accelerator;
  unifiedMemory: boolean;
  usableVramBytes: number;
}

export interface MemoryEstimate {
  weightsBytes: number;
  weightCopiesPct: number;
  kvCacheBytes: number;
  overheadBytes: number;
  totalBytes: number;
  contextTokens: number;
}

/** Las razones llegan como objetos etiquetados; la UI solo necesita el texto. */
export type FitReason = Record<string, unknown> | string;

export interface FitVerdict {
  level: FitLevel;
  reasons: FitReason[];
  memory: MemoryEstimate;
  estimatedTokensPerSecond: number;
}

export interface ModelSpec {
  id: string;
  name: string;
  family: string;
  provider: string;
  paramsB: number | null;
  quantization: string;
  fileBytes: number;
  contextWindow: number;
  license: string;
  licenseUrl: string | null;
  released: string;
  blurb: string;
  runtimes: string[];
  capability: number;
}

export interface InstalledModel {
  modelId: string;
  runtime: string;
  path: string | null;
  bytes: number;
  installedAt: string;
  sha256: string | null;
  integrity: string;
  contextWindow: number;
}

export interface RunningModel {
  modelId: string;
  runtime: string;
  endpoint: string;
  contextTokens: number;
  startedAt: string;
}

export interface ModelCard {
  model: ModelSpec;
  verdict: FitVerdict;
  state: ModelState;
  installed: InstalledModel | null;
  recommended: boolean;
}

export interface RuntimeReport {
  id: string;
  name: string;
  description: string;
  selfInstallable: boolean;
  state: "ready" | "needsSetup" | "unavailable";
  detail?: string;
}

export interface ResourceSample {
  cpuPercent: number;
  ramUsedBytes: number;
  ramTotalBytes: number;
  diskFreeBytes: number;
}

export interface HomeSnapshot {
  hardware: HardwareProfile;
  hardwareLabel: string;
  resources: ResourceSample;
  recommended: ModelCard[];
  installed: ModelCard[];
  running: RunningModel | null;
  runtimes: RuntimeReport[];
  policyOrigin: string;
  policyManaged: boolean;
  organization: string;
  onboarded: boolean;
  catalogSource: string;
  version: string;
}

export interface GenerationMetrics {
  timeToFirstTokenMs: number;
  totalMs: number;
  outputTokens: number;
  tokensPerSecond: number;
}

export interface Message {
  id: string;
  role: "system" | "user" | "assistant";
  content: string;
  createdAt: string;
  modelId?: string;
  metrics?: GenerationMetrics;
  local: boolean;
}

export interface Thread {
  id: string;
  title: string;
  modelId: string | null;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
}

export interface Settings {
  activeModelId: string | null;
  systemPrompt: string;
  temperature: number;
  autostartLastModel: boolean;
  onboarded: boolean;
}

export interface PolicyView {
  organization?: string | null;
  locked: string[];
  network: { proxy?: string | null; caBundle?: string | null; offline: boolean };
  catalog: {
    source?: string | null;
    mirrorBaseUrl?: string | null;
    allowlist: string[];
    denylist: string[];
    allowUnverifiedDownloads: boolean;
  };
  telemetry: { enabled: boolean; endpoint?: string | null };
  audit: { enabled: boolean; retentionMonths: number };
  updates: { channel: "off" | "manual" | "auto"; feedUrl?: string | null };
  chat: { systemPrompt?: string | null };
  origin: string;
  managed: boolean;
  runtimes: RuntimeReport[];
  catalogSource: string;
  dataDir: string;
}

export interface AuditEntry {
  ts: string;
  event: string;
  actor: string;
  modelId?: string;
  runtime?: string;
  bytes?: number;
  durationMs?: number;
  outcome: string;
}

/** Eventos del núcleo, tal y como llegan por SSE o por el bus de Tauri. */
export type AppEvent =
  | {
      type: "downloadProgress";
      modelId: string;
      receivedBytes: number;
      totalBytes: number | null;
      bytesPerSecond: number;
    }
  | { type: "downloadFinished"; modelId: string; ok: boolean; message: string | null }
  | { type: "modelStateChanged"; modelId: string; state: string }
  | { type: "runtimeStatus"; runtime: string; state: string; detail: string | null }
  | { type: "chatDelta"; threadId: string; messageId: string; text: string }
  | {
      type: "chatDone";
      threadId: string;
      messageId: string;
      stopped: boolean;
      metrics: GenerationMetrics | null;
    }
  | { type: "chatError"; threadId: string; message: string };
