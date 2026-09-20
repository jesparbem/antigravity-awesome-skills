# Fase 3 — Plan del MVP

Tareas pequeñas y verificables. La columna **Verificación** es la prueba concreta que se ejecuta;
si no se puede ejecutar en este entorno, se indica por qué.

Estado: ✅ hecho · ⚠️ hecho con limitación documentada · ⛔ fuera del MVP

> Resultado: **248 comprobaciones automatizadas en verde** (192 tests de Rust,
> 26 de TypeScript, 30 de extremo a extremo en navegador). El detalle de qué se
> verificó y qué no está en [`VALIDATION.md`](VALIDATION.md).

---

## T0 · Andamiaje

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T0.1 | Workspace Cargo con `factoria-core`, `factoria-server`, `src-tauri` | `cargo metadata` resuelve | ✅ |
| T0.2 | Proyecto Vite + Svelte 5 + TS + Tailwind 4 | `npm run build` genera `dist/` | ✅ |
| T0.3 | `NOTICE.md` + `LICENSE` con atribución a Rebost | Revisión manual | ✅ |

## T1 · Detección de hardware

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T1.1 | `HardwareProfile` con SO, CPU, núcleos, arch, RAM, disco | `cargo test hardware::` | ✅ |
| T1.2 | Detección de GPU/VRAM por SO (nvidia-smi / WMI / system_profiler / sysfs) | Test de los *parsers* con salidas reales fijadas | ✅ |
| T1.3 | Acelerador efectivo + memoria unificada | `cargo test accelerator` | ✅ |
| T1.4 | Degradado correcto cuando no hay GPU detectable | Test con detector vacío | ✅ |

## T2 · Catálogo y aptitud

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T2.1 | `ModelSpec` con familia, parámetros, cuantización, contexto, licencia, SHA-256, runtimes | Deserialización del catálogo embebido | ✅ |
| T2.2 | `fit()` → Óptimo / Compatible / No recomendado + razones | Tests por banda de RAM (4/8/16/32/64 GB) y con/sin GPU | ✅ |
| T2.3 | `recommend()` determinista | Test: mismo perfil ⇒ misma recomendación | ✅ |
| T2.4 | Estimación de tok/s por acelerador | Test de monotonía (más VRAM ⇒ no menos tok/s) | ✅ |
| T2.5 | Allowlist/denylist de política aplicadas al catálogo | `cargo test policy::` | ✅ |

## T3 · Runtimes

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T3.1 | `trait LlmRuntime` + `RuntimeRegistry` | Compila; test con runtime falso | ✅ |
| T3.2 | Adaptador **llama.cpp**: *pins*, extracción, arranque, `/health`, parada | E2E contra un `llama-server` de prueba que habla el mismo protocolo | ⚠️ ver L1 |
| T3.8 | El modelo no se publica como "en ejecución" hasta que `/health` responde | E2E: enviar justo tras arrancar no falla | ✅ |
| T3.3 | *Flags* derivados del hardware (`-ngl`, `-c`, `-t`, `--cache-type-*`, `-fa`) | `cargo test tuning::` | ✅ |
| T3.4 | *Streaming* SSE compatible OpenAI + cancelación | Test de parseo + E2E con parada real | ✅ |
| T3.5 | Adaptador **Ollama** (probe, list, pull, chat) | Test de parseo de `/api/tags` y `/api/chat`; *probe* real si hay demonio | ⚠️ ver L2 |
| T3.6 | Descarga con progreso + verificación SHA-256 | E2E contra un *mirror* HTTP local | ⚠️ ver L3 |
| T3.7 | Lectura de cabecera GGUF | Test con GGUF sintético válido e inválido | ✅ |

## T4 · Chat

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T4.1 | Conversaciones persistidas (threads.json + JSONL) | `cargo test chat::` | ✅ |
| T4.2 | Envío con *streaming*, stop, regenerar, limpiar | E2E en navegador | ✅ |
| T4.3 | Indicador "Procesando localmente" derivado del *endpoint* real | Test: endpoint no-loopback ⇒ indicador apagado | ✅ |
| T4.4 | Métricas por respuesta (TTFT, tok/s, tokens) | E2E: la UI muestra cifras reales | ✅ |

## T5 · Política corporativa

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T5.1 | Carga de `policy.json` con precedencia gestionada > env > usuario > defecto | `cargo test policy::` | ✅ |
| T5.2 | Campos `locked` reflejados en la UI | E2E: ajuste bloqueado no editable | ✅ |
| T5.3 | Proxy, CA, `offline`, mirror aplicados al cliente HTTP | Test de construcción del cliente | ✅ |
| T5.4 | Auditoría local *append-only* sin contenido | Test: instalar + chatear ⇒ N líneas, ninguna con el prompt | ✅ |
| T5.5 | Telemetría apagada por defecto | Test: sin política ⇒ `enabled == false` | ✅ |

## T6 · Hosts

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T6.1 | `factoria-server` con `/api/*` + `/api/events` (SSE) + estáticos | `curl` a cada endpoint | ✅ |
| T6.2 | Token de sesión y comprobación de `Origin` | Test: petición sin token ⇒ 401 | ✅ |
| T6.3 | `src-tauri` con los mismos comandos | `cargo check -p factoria-desktop` + `clippy -D warnings` | ✅ compila; el *bundle* sigue sin generarse (L4) |

## T7 · Frontend

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T7.1 | `bridge.ts` Tauri/HTTP transparente | `vitest` | ✅ |
| T7.2 | *Design tokens* Naturgy + tipografía empaquetada | Revisión visual + *screenshots* | ✅ |
| T7.3 | **Home**: Mi PC, recomendados, instalados, recursos, acciones rápidas | E2E + *screenshot* | ✅ |
| T7.4 | **Catálogo**: tarjetas con metadatos, estado y aptitud; filtros | E2E + *screenshot* | ✅ |
| T7.5 | Acciones: instalar / ejecutar / detener / eliminar / abrir chat | E2E del ciclo completo | ✅ |
| T7.6 | **Chat**: selector de modelo, *streaming*, stop, regenerar, copiar, limpiar | E2E + *screenshot* | ✅ |
| T7.7 | **Ajustes**: red, política, auditoría, diagnóstico | E2E + *screenshot* | ✅ |

## T8 · Calidad

| # | Tarea | Verificación | Estado |
|---|-------|--------------|--------|
| T8.1 | `cargo fmt --check` + `cargo clippy -D warnings` | Comando | ✅ |
| T8.2 | `cargo test` del workspace | Comando | ✅ |
| T8.3 | `svelte-check` + `vitest` | Comando | ✅ |
| T8.4 | `npm run build` del frontend | Comando | ✅ |
| T8.5 | E2E Playwright del flujo completo + *screenshots* | `npm run test:e2e` | ✅ |
| T8.6 | CI en GitHub Actions | Workflow en el repo | ✅ |

## T9 · Fuera del MVP (arquitectura preparada)

⛔ RAG local · documentos · embeddings · MCP · skills · agentes · *tool calling* ·
SSO · catálogo centralizado remoto · multiidioma · firma y notarización de instaladores.

---

## Limitaciones conocidas de este entorno de validación

| # | Limitación | Consecuencia |
|---|------------|--------------|
| **L1** | La política de egreso de este contenedor **bloquea `huggingface.co`, `ollama.com` y `api.github.com`** (403 del proxy). | No se puede descargar aquí ni un binario real de `llama-server` ni pesos GGUF reales. El E2E usa un **`llama-server` de prueba** que implementa el mismo protocolo (`/health`, `/v1/chat/completions` con SSE) y un *mirror* HTTP local. Se ejercita **el código real** de FactorIA (arranque de proceso, sondeo de salud, descarga, SHA-256, parseo SSE, cancelación); lo único sustituido es llama.cpp y los pesos. |
| **L2** | No hay demonio Ollama en este contenedor. | El adaptador Ollama se valida con tests de parseo y un servidor de prueba, no contra Ollama real. |
| **L3** | Sin acceso a Hugging Face. | Las URLs y SHA-256 del catálogo embebido **no se han podido verificar en vivo**; están marcadas en el código y deben confirmarse antes de distribuir. |
| **L4** | Linux no es plataforma soportada por el *bundle*. | Instaladas las dependencias GTK/WebKit, el *crate* del shell **compila y pasa clippy**; lo que no se ha generado son los instaladores `.dmg`/NSIS, que exigen macOS y Windows. |
| **L5** | Sin GPU en el contenedor. | Las rutas CUDA/Metal/Vulkan se validan con tests de los *parsers* y de la lógica de clasificación, no con hardware real. |
