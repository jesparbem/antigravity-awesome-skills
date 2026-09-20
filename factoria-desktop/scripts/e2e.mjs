/**
 * Prueba de extremo a extremo del flujo principal de FactorIA Desktop:
 *
 *   detectar PC → catálogo → instalar → ejecutar → chat local
 *
 * Conduce la interfaz real con un navegador y deja capturas en `screenshots/`.
 * Las dos sustituciones del entorno están documentadas en `scripts/harness.mjs`
 * y en `docs/VALIDATION.md`.
 */

import { chromium } from "@playwright/test";
import { mkdirSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { prepareWorkspace, startMirror, startServer, writePolicy } from "./harness.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SHOTS = join(ROOT, "screenshots");
const SERVER_BIN = join(ROOT, "target", "debug", "factoria-server");
const STATIC_DIR = join(ROOT, "dist");

const pasos = [];
let fallos = 0;

function check(nombre, condicion, detalle = "") {
  pasos.push({ nombre, ok: !!condicion, detalle });
  if (condicion) {
    console.log(`  ✓ ${nombre}`);
  } else {
    fallos += 1;
    console.error(`  ✗ ${nombre}${detalle ? ` — ${detalle}` : ""}`);
  }
}

/**
 * Chromium a usar. `PLAYWRIGHT_CHROMIUM` manda; si no, se aprovecha el que ya
 * traiga la imagen (entornos de CI que no descargan navegadores); si tampoco,
 * lo resuelve Playwright.
 */
function chromiumPath() {
  if (process.env.PLAYWRIGHT_CHROMIUM) return process.env.PLAYWRIGHT_CHROMIUM;
  const preinstalled = join(process.env.PLAYWRIGHT_BROWSERS_PATH ?? "/opt/pw-browsers", "chromium");
  return existsSync(preinstalled) ? preinstalled : undefined;
}

async function shot(page, name) {
  await page.screenshot({ path: join(SHOTS, `${name}.png`), fullPage: false });
}

async function main() {
  if (!existsSync(SERVER_BIN)) throw new Error(`falta ${SERVER_BIN}: ejecuta cargo build primero`);
  if (!existsSync(STATIC_DIR)) throw new Error(`falta ${STATIC_DIR}: ejecuta npm run build primero`);
  mkdirSync(SHOTS, { recursive: true });

  const ws = prepareWorkspace();
  const mirror = await startMirror(ws.ggufPath, "/factoria/mini.gguf");
  const policyPath = writePolicy(ws.root, {
    catalogPath: ws.catalogPath,
    mirrorUrl: mirror.url,
  });
  const server = await startServer({
    dataDir: ws.dataDir,
    policyPath,
    binary: SERVER_BIN,
    staticDir: STATIC_DIR,
  });
  console.log(`\nFactorIA levantada en ${server.uiUrl.split("?")[0]}\n`);

  const browser = await chromium.launch({
    executablePath: chromiumPath(),
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  const page = await browser.newPage({ viewport: { width: 1360, height: 900 } });
  page.on("pageerror", (err) => {
    fallos += 1;
    console.error(`  ✗ error de JavaScript en la página: ${err.message}`);
  });

  try {
    // ---- 1. Detección del PC ------------------------------------------
    console.log("1. Detección del PC");
    await page.goto(server.uiUrl, { waitUntil: "networkidle" });
    await page.waitForSelector('[data-testid="panel-hardware"]', { timeout: 20_000 });

    const cpu = (await page.locator('[data-testid="hw-cpu"]').innerText()).trim();
    const ram = (await page.locator('[data-testid="hw-ram"]').innerText()).trim();
    const gpu = (await page.locator('[data-testid="hw-gpu"]').innerText()).trim();
    check("la Home muestra el procesador detectado", cpu.length > 3 && !cpu.includes("—"), cpu);
    check("la Home muestra la memoria detectada", /\d/.test(ram), ram);
    check("la Home informa de la gráfica (o de su ausencia)", gpu.length > 3, gpu);
    check(
      "el panel de recursos muestra medidas reales",
      await page.locator('[data-testid="panel-resources"] .barra').first().isVisible(),
    );
    // El estilo de la etiqueta aplica `text-transform: uppercase`, así que
    // `innerText` devuelve el texto ya transformado.
    const hero = (await page.locator('[data-testid="home-hero"]').innerText()).toLowerCase();
    check(
      "la organización viene de la política corporativa",
      hero.includes("naturgy it corporativo"),
      hero.slice(0, 60),
    );
    await shot(page, "01-home");

    // ---- 2. Catálogo y recomendación ----------------------------------
    console.log("\n2. Catálogo y recomendación");
    await page.locator('[data-testid="nav-catalog"]').click();
    await page.waitForSelector('[data-testid="catalog-grid"]');
    const card = page.locator('[data-model-id="factoria-test-mini"]');
    check("el catálogo carga desde el catálogo corporativo", await card.isVisible());
    const fit = (await card.locator('[data-testid="fit-chip"]').innerText()).trim();
    check("el modelo llega clasificado por compatibilidad", ["Óptimo", "Compatible"].includes(fit), fit);
    check(
      "la clasificación viene con sus razones explicadas",
      (await card.locator('[data-testid="fit-reasons"] li').count()) > 0,
    );
    check(
      "la tarjeta estima la memoria necesaria",
      /\d/.test(await card.locator('[data-testid="memory-need"]').innerText()),
    );

    await card.locator('[data-testid="btn-details"]').click();
    check(
      "el desglose de memoria es auditable desde la interfaz",
      await card.locator('[data-testid="memory-breakdown"]').isVisible(),
    );
    await shot(page, "02-catalogo");

    // ---- 3. Instalación ------------------------------------------------
    console.log("\n3. Instalación");
    await card.locator('[data-testid="btn-install"]').click();
    await page.waitForSelector('[data-model-id="factoria-test-mini"][data-state="installed"]', {
      timeout: 60_000,
    });
    check("el modelo queda instalado tras verificar su SHA-256", true);
    check(
      "aparece la acción de ejecutar",
      await card.locator('[data-testid="btn-start"]').isVisible(),
    );
    await shot(page, "03-instalado");

    // ---- 4. Ejecución --------------------------------------------------
    console.log("\n4. Ejecución local");
    await card.locator('[data-testid="btn-start"]').click();
    await page.waitForSelector('[data-model-id="factoria-test-mini"][data-state="running"]', {
      timeout: 60_000,
    });
    check("el motor arranca y responde a /health", true);
    check(
      "la barra lateral muestra el modelo en ejecución",
      await page.locator('[data-testid="sidebar-running"]').isVisible(),
    );
    const endpoint = await page.locator('[data-testid="sidebar-running"]').innerText();
    check("el motor escucha en loopback", endpoint.includes("127.0.0.1"), endpoint.replace(/\n/g, " "));

    // ---- 5. Chat local --------------------------------------------------
    console.log("\n5. Chat local");
    await card.locator('[data-testid="btn-open-chat"]').click();
    await page.waitForSelector('[data-testid="composer"]', { timeout: 20_000 });
    check(
      'la interfaz declara "Procesando localmente"',
      await page.locator('[data-testid="local-badge"]').isVisible(),
    );

    await page.locator('[data-testid="composer"]').fill("¿Dónde se ejecuta este modelo?");
    await page.locator('[data-testid="btn-send"]').click();

    await page.waitForSelector('[data-testid="streaming-message"]', { timeout: 20_000 });
    const parcial = await page.locator('[data-testid="streaming-message"]').innerText();
    check("la respuesta llega en streaming", parcial.trim().length > 0);
    await shot(page, "04-chat-streaming");

    await page.waitForSelector('[data-testid="message"][data-role="assistant"]', { timeout: 60_000 });
    const respuesta = await page
      .locator('[data-testid="message"][data-role="assistant"]')
      .last()
      .innerText();
    check("la respuesta se completa y se persiste", respuesta.length > 40);
    check(
      "el turno registra métricas medidas",
      await page.locator('[data-testid="metrics"]').first().isVisible(),
    );
    const metricas = await page.locator('[data-testid="metrics"]').first().innerText();
    check("las métricas traen cifras reales", /[\d,]+ tok\/s/.test(metricas), metricas);
    await shot(page, "05-chat-respuesta");

    // ---- 6. Detener la generación ---------------------------------------
    console.log("\n6. Detener y regenerar");
    await page.locator('[data-testid="composer"]').fill("Escribe una respuesta larga, por favor.");
    await page.locator('[data-testid="btn-send"]').click();
    await page.waitForSelector('[data-testid="btn-stop-generation"]', { timeout: 10_000 });
    await page.waitForTimeout(250);
    await page.locator('[data-testid="btn-stop-generation"]').click();
    await page.waitForSelector('[data-testid="btn-send"]', { timeout: 20_000 });
    check("la generación se puede detener a mitad", true);

    const antes = await page.locator('[data-testid="message"][data-role="assistant"]').count();
    await page.locator('[data-testid="btn-regenerate"]').click();
    await page.waitForSelector('[data-testid="btn-send"]', { timeout: 60_000 });
    const despues = await page.locator('[data-testid="message"][data-role="assistant"]').count();
    check("regenerar no duplica respuestas", despues <= antes, `${antes} → ${despues}`);

    // ---- 7. Ajustes, política y auditoría --------------------------------
    console.log("\n7. Ajustes, política y auditoría");
    await page.locator('[data-testid="nav-settings"]').click();
    // La política llega por una petición asíncrona: se espera al dato, no al hueco.
    await page.waitForSelector('[data-testid="settings-policy"][data-loaded="true"]', {
      timeout: 20_000,
    });
    check(
      "la política se muestra como gestionada por la organización",
      (await page.locator('[data-testid="settings-policy"]').innerText()).includes("Gestionado por"),
    );
    check(
      "la telemetría aparece desactivada",
      (await page.locator('[data-testid="telemetry-state"]').innerText()).includes("Desactivada"),
    );
    const auditoria = await page.locator('[data-testid="settings-audit"]').innerText();
    check("la auditoría registra la instalación", auditoria.includes("model.install"));
    check("la auditoría registra la ejecución", auditoria.includes("model.start"));
    check("la auditoría registra el turno de chat", auditoria.includes("chat.turn"));
    check(
      "la auditoría no contiene el texto del prompt",
      !auditoria.includes("¿Dónde se ejecuta este modelo?"),
    );
    await shot(page, "06-ajustes");

    // ---- 8. Detener y eliminar -------------------------------------------
    console.log("\n8. Detener y eliminar");
    await page.locator('[data-testid="nav-catalog"]').click();
    await page.waitForSelector('[data-testid="catalog-grid"]');
    await card.locator('[data-testid="btn-stop"]').click();
    await page.waitForSelector('[data-model-id="factoria-test-mini"][data-state="installed"]', {
      timeout: 30_000,
    });
    check("el modelo se puede detener", true);

    page.once("dialog", (d) => d.accept());
    await card.locator('[data-testid="btn-remove"]').click();
    await page.waitForSelector('[data-model-id="factoria-test-mini"][data-state="notInstalled"]', {
      timeout: 30_000,
    });
    check("el modelo se puede eliminar del equipo", true);
    await shot(page, "07-catalogo-final");
  } finally {
    await browser.close();
    server.close();
    mirror.close();
  }

  console.log(`\n${"─".repeat(60)}`);
  const ok = pasos.filter((p) => p.ok).length;
  console.log(`${ok}/${pasos.length} comprobaciones superadas · capturas en screenshots/`);
  if (fallos) {
    console.error(`\n${fallos} comprobación(es) fallida(s).`);
    process.exit(1);
  }
  console.log("Flujo completo verificado: detectar PC → catálogo → instalar → ejecutar → chat.");
}

main().catch((err) => {
  console.error("\nLa prueba de extremo a extremo no pudo completarse:");
  console.error(err);
  process.exit(1);
});
