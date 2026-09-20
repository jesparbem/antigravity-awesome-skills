/**
 * Estado compartido de la aplicación.
 *
 * Un único sitio recibe los eventos del núcleo y mantiene la vista actualizada,
 * para que ninguna pantalla tenga que sondear.
 */

import { api, subscribe } from "./api";
import type { AppEvent, HomeSnapshot, ModelCard, Thread } from "./types";

export interface DownloadState {
  receivedBytes: number;
  totalBytes: number | null;
  bytesPerSecond: number;
}

export interface Toast {
  id: number;
  kind: "ok" | "error" | "info";
  text: string;
}

class AppStore {
  home = $state<HomeSnapshot | null>(null);
  models = $state<ModelCard[]>([]);
  threads = $state<Thread[]>([]);
  downloads = $state<Record<string, DownloadState>>({});
  toasts = $state<Toast[]>([]);
  loading = $state(true);
  fatal = $state<string | null>(null);

  /** Texto que llega por streaming, por identificador de mensaje. */
  streaming = $state<Record<string, string>>({});
  generating = $state(false);

  #toastSeq = 0;
  #unsubscribe: (() => void) | null = null;

  async init() {
    this.#unsubscribe?.();
    this.#unsubscribe = subscribe((event) => this.#onEvent(event));
    await this.refresh();
    this.loading = false;
  }

  dispose() {
    this.#unsubscribe?.();
    this.#unsubscribe = null;
  }

  async refresh() {
    try {
      const [home, models, threads] = await Promise.all([
        api.home(),
        api.models(),
        api.threads(),
      ]);
      this.home = home;
      this.models = models;
      this.threads = threads;
      this.fatal = null;
    } catch (err) {
      this.fatal = err instanceof Error ? err.message : String(err);
    }
  }

  /** Solo el catálogo y la Home: evita recargar conversaciones al instalar. */
  async refreshModels() {
    try {
      const [home, models] = await Promise.all([api.home(), api.models()]);
      this.home = home;
      this.models = models;
    } catch (err) {
      this.notify("error", err instanceof Error ? err.message : String(err));
    }
  }

  notify(kind: Toast["kind"], text: string) {
    const toast: Toast = { id: ++this.#toastSeq, kind, text };
    this.toasts = [...this.toasts, toast];
    setTimeout(() => {
      this.toasts = this.toasts.filter((t) => t.id !== toast.id);
    }, 6000);
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  #onEvent(event: AppEvent) {
    switch (event.type) {
      case "downloadProgress":
        this.downloads = {
          ...this.downloads,
          [event.modelId]: {
            receivedBytes: event.receivedBytes,
            totalBytes: event.totalBytes,
            bytesPerSecond: event.bytesPerSecond,
          },
        };
        break;
      case "downloadFinished": {
        const { [event.modelId]: _removed, ...rest } = this.downloads;
        this.downloads = rest;
        if (!event.ok) this.notify("error", event.message ?? "La descarga no se completó.");
        void this.refreshModels();
        break;
      }
      case "modelStateChanged":
        void this.refreshModels();
        break;
      case "chatDelta":
        this.streaming = {
          ...this.streaming,
          [event.messageId]: (this.streaming[event.messageId] ?? "") + event.text,
        };
        break;
      case "chatDone":
        this.generating = false;
        break;
      case "chatError":
        this.generating = false;
        this.notify("error", event.message);
        break;
      default:
        break;
    }
  }

  clearStream(messageId: string) {
    const { [messageId]: _removed, ...rest } = this.streaming;
    this.streaming = rest;
  }
}

export const store = new AppStore();
