/**
 * Banco de pruebas de extremo a extremo.
 *
 * Levanta FactorIA con dos sustituciones, ambas documentadas en
 * `docs/VALIDATION.md`:
 *
 *  1. Un **mirror corporativo local** que sirve un GGUF de prueba, porque la
 *     política de salida de la red de este entorno bloquea Hugging Face.
 *  2. Un **`llama-server` de prueba** que habla el mismo protocolo, porque
 *     tampoco se puede descargar el binario real de llama.cpp.
 *
 * Todo lo demás es el código real: descarga con verificación SHA-256,
 * validación de la cabecera GGUF, lanzamiento del proceso hijo, sondeo de
 * salud, parseo SSE, cancelación y métricas.
 */

import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const STUB = join(ROOT, "tests", "fixtures", "llama-server-stub.py");

/** Escribe un GGUF mínimo pero válido (mismo formato que `gguf::write_minimal_gguf`). */
export function writeFixtureGguf(path, architecture = "qwen2", contextLength = 8192) {
  const parts = [];
  const u32 = (n) => {
    const b = Buffer.alloc(4);
    b.writeUInt32LE(n);
    return b;
  };
  const u64 = (n) => {
    const b = Buffer.alloc(8);
    b.writeBigUInt64LE(BigInt(n));
    return b;
  };
  const str = (s) => {
    const body = Buffer.from(s, "utf8");
    return Buffer.concat([u64(body.length), body]);
  };

  parts.push(Buffer.from("GGUF", "ascii"), u32(3), u64(1) /* tensores */, u64(2) /* claves */);
  // general.architecture (tipo 8 = string)
  parts.push(str("general.architecture"), u32(8), str(architecture));
  // <arch>.context_length (tipo 4 = uint32)
  parts.push(str(`${architecture}.context_length`), u32(4), u32(contextLength));

  const bytes = Buffer.concat(parts);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
  return { bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
}

/** Mirror HTTP local que sirve un único fichero. */
export function startMirror(filePath, servedPath) {
  return new Promise((ok) => {
    const body = readFileSync(filePath);
    const server = createServer((req, res) => {
      if (req.url === servedPath) {
        res.writeHead(200, {
          "content-type": "application/octet-stream",
          "content-length": String(body.length),
        });
        res.end(body);
      } else {
        res.writeHead(404).end();
      }
    });
    server.listen(0, "127.0.0.1", () =>
      ok({
        url: `http://127.0.0.1:${server.address().port}`,
        close: () => server.close(),
      }),
    );
  });
}

/** Prepara el directorio de datos, el catálogo corporativo y la política. */
export function prepareWorkspace() {
  const root = mkdtempSync(join(tmpdir(), "factoria-e2e-"));
  const dataDir = join(root, "data");
  const ggufPath = join(root, "mirror", "factoria", "mini.gguf");
  mkdirSync(dataDir, { recursive: true });

  const fixture = writeFixtureGguf(ggufPath);

  const catalog = [
    {
      id: "factoria-test-mini",
      name: "FactorIA Mini (prueba)",
      family: "Prueba",
      provider: "Naturgy IT",
      paramsB: 0.5,
      quantization: "Q4_K_M",
      fileBytes: fixture.bytes,
      contextWindow: 8192,
      license: "Apache-2.0",
      licenseUrl: null,
      released: "2026-09",
      blurb: "Modelo de validación del despliegue. No genera texto real.",
      runtimes: ["llamacpp"],
      source: {
        kind: "ggufUrl",
        url: "https://huggingface.co/naturgy/factoria-mini/resolve/main/mini.gguf",
        sha256: fixture.sha256,
        mirrorPath: "factoria/mini.gguf",
      },
      capability: 10,
      kv: { layers: 24, kvHeads: 2, headDim: 64 },
    },
  ];
  const catalogPath = join(root, "catalog.json");
  writeFileSync(catalogPath, JSON.stringify(catalog, null, 2));

  return { root, dataDir, ggufPath, catalogPath, fixture };
}

export function writePolicy(root, { catalogPath, mirrorUrl }) {
  const policyPath = join(root, "policy.json");
  writeFileSync(
    policyPath,
    JSON.stringify(
      {
        organization: "Naturgy IT Corporativo",
        locked: ["network.proxy", "catalog.denylist"],
        catalog: {
          source: catalogPath,
          mirrorBaseUrl: mirrorUrl,
          allowlist: [],
          denylist: [],
          allowUnverifiedDownloads: false,
        },
        telemetry: { enabled: false },
        audit: { enabled: true, retentionMonths: 12 },
        updates: { channel: "off" },
        chat: { systemPrompt: "Responde en castellano, de forma breve y profesional." },
      },
      null,
      2,
    ),
  );
  return policyPath;
}

/** Arranca `factoria-server` y espera a su línea de preparado. */
export function startServer({ dataDir, policyPath, binary, staticDir }) {
  return new Promise((ok, fail) => {
    const child = spawn(binary, ["--port", "0", "--static", staticDir], {
      env: {
        ...process.env,
        FACTORIA_DATA_DIR: dataDir,
        FACTORIA_POLICY_FILE: policyPath,
        FACTORIA_LLAMA_SERVER_BIN: STUB,
        FACTORIA_LOG: "factoria_server=info,factoria_core=info",
      },
      stdio: ["ignore", "pipe", "pipe"],
    });

    let out = "";
    let stderr = "";
    const timer = setTimeout(() => {
      child.kill();
      fail(new Error(`el servidor no arrancó a tiempo.\nstdout:\n${out}\nstderr:\n${stderr}`));
    }, 30_000);

    child.stdout.on("data", (chunk) => {
      out += chunk;
      const match = out.match(/FACTORIA_READY (\S+)/);
      if (match) {
        clearTimeout(timer);
        ok({ child, uiUrl: match[1], stderr: () => stderr, close: () => child.kill() });
      }
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      fail(new Error(`el servidor terminó con código ${code}\n${stderr}`));
    });
  });
}
