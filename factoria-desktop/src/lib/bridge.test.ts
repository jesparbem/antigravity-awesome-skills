import { beforeEach, describe, expect, it, vi } from "vitest";
import { detectHost, httpRequest, sessionToken, BridgeError } from "./bridge";

describe("detectHost", () => {
  beforeEach(() => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("usa HTTP cuando no está dentro de Tauri", () => {
    expect(detectHost()).toBe("http");
  });

  it("usa Tauri cuando el shell inyecta su puente", () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    expect(detectHost()).toBe("tauri");
  });
});

describe("sessionToken", () => {
  beforeEach(() => {
    window.sessionStorage.clear();
    window.history.replaceState({}, "", "/");
  });

  it("toma el token de la query y lo borra de la barra de direcciones", () => {
    window.history.replaceState({}, "", "/?token=abc123");
    expect(sessionToken()).toBe("abc123");
    expect(window.location.search).not.toContain("token");
  });

  it("lo recuerda en llamadas posteriores", () => {
    window.history.replaceState({}, "", "/?token=abc123");
    sessionToken();
    expect(sessionToken()).toBe("abc123");
  });

  it("devuelve cadena vacía cuando no hay token", () => {
    expect(sessionToken()).toBe("");
  });
});

describe("httpRequest", () => {
  it("envía el token en la cabecera", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ ok: true }),
    });
    vi.stubGlobal("fetch", fetchMock);
    await httpRequest("GET", "/api/home", undefined, "tok");
    expect(fetchMock.mock.calls[0][1].headers["x-factoria-token"]).toBe("tok");
    vi.unstubAllGlobals();
  });

  it("convierte el error del servidor en un mensaje legible", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 404,
        statusText: "Not Found",
        json: async () => ({ error: "no existe el modelo x" }),
      }),
    );
    await expect(httpRequest("GET", "/api/models/x")).rejects.toThrow("no existe el modelo x");
    vi.unstubAllGlobals();
  });

  it("cae al estado HTTP cuando la respuesta no trae JSON", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 401,
        statusText: "Unauthorized",
        json: async () => {
          throw new Error("sin cuerpo");
        },
      }),
    );
    await expect(httpRequest("GET", "/api/home")).rejects.toBeInstanceOf(BridgeError);
    vi.unstubAllGlobals();
  });
});
