/** Superficie tipada del núcleo. Es lo único que las vistas usan. */

import { call, detectHost, sessionToken } from "./bridge";
import type {
  AppEvent,
  AuditEntry,
  HardwareProfile,
  HomeSnapshot,
  InstalledModel,
  Message,
  ModelCard,
  PolicyView,
  ResourceSample,
  RunningModel,
  Settings,
  Thread,
} from "./types";

export const api = {
  home: () => call<HomeSnapshot>({ command: "home", method: "GET", path: "/api/home" }),

  hardware: () =>
    call<HardwareProfile>({ command: "hardware", method: "GET", path: "/api/hardware" }),

  resources: () =>
    call<ResourceSample>({ command: "resources", method: "GET", path: "/api/resources" }),

  models: () => call<ModelCard[]>({ command: "models", method: "GET", path: "/api/models" }),

  installModel: (id: string) =>
    call<InstalledModel>({
      command: "install_model",
      method: "POST",
      path: `/api/models/${encodeURIComponent(id)}/install`,
      args: { id },
    }),

  startModel: (id: string) =>
    call<RunningModel>({
      command: "start_model",
      method: "POST",
      path: `/api/models/${encodeURIComponent(id)}/start`,
      args: { id },
    }),

  stopModel: (id: string) =>
    call<{ ok: boolean }>({
      command: "stop_model",
      method: "POST",
      path: `/api/models/${encodeURIComponent(id)}/stop`,
      args: { id },
    }),

  removeModel: (id: string) =>
    call<{ ok: boolean }>({
      command: "remove_model",
      method: "DELETE",
      path: `/api/models/${encodeURIComponent(id)}`,
      args: { id },
    }),

  threads: () => call<Thread[]>({ command: "threads", method: "GET", path: "/api/threads" }),

  createThread: () =>
    call<Thread>({ command: "create_thread", method: "POST", path: "/api/threads" }),

  deleteThread: (id: string) =>
    call<{ ok: boolean }>({
      command: "delete_thread",
      method: "DELETE",
      path: `/api/threads/${encodeURIComponent(id)}`,
      args: { id },
    }),

  clearThread: (id: string) =>
    call<{ ok: boolean }>({
      command: "clear_thread",
      method: "POST",
      path: `/api/threads/${encodeURIComponent(id)}/clear`,
      args: { id },
    }),

  messages: (id: string) =>
    call<Message[]>({
      command: "messages",
      method: "GET",
      path: `/api/threads/${encodeURIComponent(id)}/messages`,
      args: { id },
    }),

  send: (id: string, text: string, regenerate = false) =>
    call<Message>({
      command: "send_message",
      method: "POST",
      path: `/api/threads/${encodeURIComponent(id)}/send`,
      args: { id, text, regenerate },
      body: { text, regenerate },
    }),

  stopGeneration: () =>
    call<{ ok: boolean }>({
      command: "stop_generation",
      method: "POST",
      path: "/api/generation/stop",
    }),

  settings: () => call<Settings>({ command: "settings", method: "GET", path: "/api/settings" }),

  saveSettings: (settings: Settings) =>
    call<Settings>({
      command: "save_settings",
      method: "POST",
      path: "/api/settings",
      args: { settings },
      body: settings,
    }),

  policy: () => call<PolicyView>({ command: "policy", method: "GET", path: "/api/policy" }),

  audit: (limit = 50) =>
    call<AuditEntry[]>({
      command: "audit",
      method: "GET",
      path: `/api/audit?limit=${limit}`,
      args: { limit },
    }),
};

/**
 * Suscripción al flujo de eventos del núcleo.
 *
 * Devuelve la función para cancelar. En Tauri se escucha el evento
 * `factoria://event`; con el host HTTP, un `EventSource`.
 */
export function subscribe(onEvent: (event: AppEvent) => void): () => void {
  if (detectHost() === "tauri") {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<AppEvent>("factoria://event", (e) => onEvent(e.payload)).then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      }),
    );
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }

  const source = new EventSource(`/api/events?token=${encodeURIComponent(sessionToken())}`);
  source.onmessage = (event) => {
    try {
      onEvent(JSON.parse(event.data) as AppEvent);
    } catch {
      /* una trama ilegible no debe romper el flujo */
    }
  };
  return () => source.close();
}
