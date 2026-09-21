/**
 * Puente único hacia el núcleo.
 *
 * La interfaz no sabe en qué host corre. Dentro de la aplicación de escritorio
 * llama a los comandos de Tauri; servida por el host HTTP local, usa `fetch`.
 * El resto del frontend solo ve `api.*`.
 */

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: { core?: { invoke?: Invoke }; event?: unknown };
  }
}

export type Host = "tauri" | "http";

export function detectHost(): Host {
  return typeof window !== "undefined" && window.__TAURI_INTERNALS__ !== undefined
    ? "tauri"
    : "http";
}

/**
 * Token de sesión del host HTTP.
 *
 * Llega en la query al abrir la interfaz. Se guarda en `sessionStorage` y se
 * limpia de la barra de direcciones para que no quede en el historial.
 */
export function sessionToken(): string {
  if (typeof window === "undefined") return "";
  const url = new URL(window.location.href);
  const fromQuery = url.searchParams.get("token");
  if (fromQuery) {
    try {
      window.sessionStorage.setItem("factoria.token", fromQuery);
    } catch {
      /* sessionStorage puede estar bloqueado; el token vive en memoria */
    }
    url.searchParams.delete("token");
    window.history.replaceState({}, "", url.toString());
    return fromQuery;
  }
  try {
    return window.sessionStorage.getItem("factoria.token") ?? "";
  } catch {
    return "";
  }
}

export class BridgeError extends Error {
  constructor(
    message: string,
    readonly status?: number,
  ) {
    super(message);
    this.name = "BridgeError";
  }
}

async function httpRequest<T>(
  method: string,
  path: string,
  body?: unknown,
  token = sessionToken(),
): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: {
      "content-type": "application/json",
      "x-factoria-token": token,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const payload = (await res.json()) as { error?: string };
      if (payload.error) message = payload.error;
    } catch {
      /* una respuesta sin cuerpo JSON deja el mensaje de estado */
    }
    throw new BridgeError(message, res.status);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) throw new BridgeError("el puente de Tauri no está disponible");
  try {
    return (await invoke(cmd, args)) as T;
  } catch (err) {
    throw new BridgeError(String(err));
  }
}

/** Una llamada que cada host resuelve a su manera. */
export interface Call {
  command: string;
  method: "GET" | "POST" | "DELETE";
  path: string;
  args?: Record<string, unknown>;
  body?: unknown;
}

export async function call<T>(c: Call): Promise<T> {
  return detectHost() === "tauri"
    ? tauriInvoke<T>(c.command, c.args)
    : httpRequest<T>(c.method, c.path, c.body);
}

export { httpRequest };
