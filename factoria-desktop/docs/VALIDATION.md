# Fase 6 — Validación

Qué se ha comprobado de verdad, cómo, y qué **no** se ha podido comprobar en este
entorno. Nada de lo que aparece aquí como verificado está solo "implementado".

---

## 1. Resumen

| Comprobación | Resultado |
|---|---|
| `cargo fmt --all --check` | ✅ limpio |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ sin avisos |
| `cargo test --workspace` | ✅ **192 tests** (167 unitarios del núcleo + 9 de integridad de descarga + 13 del servidor + 3 del shell Tauri) |
| `cargo check -p factoria-desktop` (shell Tauri) | ✅ compila |
| `npm run check` (svelte-check) | ✅ 0 errores, 0 avisos, 131 ficheros |
| `npm test` (vitest) | ✅ **26 tests** |
| `npm run build` (Vite) | ✅ `dist/` — 163 kB JS (58 kB gzip), 18 kB CSS |
| `npm run test:e2e` (navegador real) | ✅ **30/30 comprobaciones** |

Total: **248 comprobaciones automatizadas**.

---

## 2. Flujo principal, verificado de extremo a extremo

`npm run test:e2e` levanta FactorIA, abre Chromium contra la interfaz real y recorre:

```
detectar PC → catálogo → recomendar → instalar → ejecutar → chat local
```

Las 30 comprobaciones que ejecuta:

**1. Detección del PC** — la Home muestra el procesador, la memoria y la gráfica
realmente detectados; el panel de recursos muestra medidas en vivo; la organización
sale de la política corporativa.

**2. Catálogo** — carga desde el catálogo corporativo indicado por la política;
cada modelo llega clasificado (Óptimo / Compatible / No recomendado) con sus razones
en lenguaje llano; la tarjeta estima la memoria necesaria y el desglose
(pesos + caché KV + proceso) es auditable desde la propia interfaz.

**3. Instalación** — la descarga se verifica contra el **SHA-256** anclado en el
catálogo y el fichero se valida como GGUF antes de registrarse.

**4. Ejecución** — el motor se lanza como proceso hijo, se espera a `/health`, y la
interfaz confirma que escucha en `127.0.0.1`.

**5. Chat local** — el indicador "Procesando localmente" aparece; la respuesta llega
**en streaming**; se completa y se persiste; el turno registra **métricas medidas**
(tok/s y tiempo hasta el primer token).

**6. Detener y regenerar** — la generación se corta a mitad; regenerar no duplica
respuestas.

**7. Política y auditoría** — la política se muestra como gestionada por la
organización; la telemetría aparece desactivada; la auditoría registra
`model.install`, `model.start` y `chat.turn`, y **no contiene el texto del prompt**
(se comprueba explícitamente).

**8. Detener y eliminar** — el modelo se detiene y se elimina del equipo.

Capturas en `screenshots/`, regeneradas en cada ejecución.

---

## 3. Qué se sustituye en este entorno, y por qué

La política de salida de red de este contenedor **bloquea `huggingface.co`,
`ollama.com` y `api.github.com`** (el proxy responde 403). Sin acceso a esos
orígenes no se puede descargar ni un binario real de `llama.cpp` ni pesos reales.

El banco de pruebas resuelve eso con dos sustituciones, y **solo** dos:

| Sustituido | Por qué | Qué se sigue ejercitando de verdad |
|---|---|---|
| El origen de los pesos (Hugging Face) | Bloqueado por la política de egreso | Un *mirror* HTTP local sirve un GGUF válido. Se ejercita el código real de descarga: reescritura al *mirror*, streaming a `.part`, cálculo de SHA-256, comparación con el digest anclado, renombrado atómico y validación de la cabecera GGUF. |
| El binario `llama-server` | Bloqueado por la política de egreso | `tests/fixtures/llama-server-stub.py` implementa el **mismo contrato** (`/health`, `/v1/chat/completions` con SSE estilo OpenAI). Se ejercita el código real: cálculo de argumentos según hardware, lanzamiento del proceso hijo, sondeo de `/health` con espera, parseo del SSE, cancelación, cronometraje y parada del proceso. |

Lo único que no ocurre es la inferencia en sí: el sustituto devuelve un texto fijo
token a token en lugar de generarlo. **Todo el camino de FactorIA es el de producción.**

El binario real se enchufa con `FACTORIA_LLAMA_SERVER_BIN`, la misma variable que usa
la prueba — así que un despliegue aislado con el motor ya colocado por el paquete
corporativo funciona por el mismo camino.

---

## 4. Lo que NO se ha podido verificar aquí

Dicho explícitamente, como pide el encargo:

| # | Sin verificar | Motivo | Cómo verificarlo |
|---|---|---|---|
| **V1** | **Inferencia real con pesos reales** | Sin acceso a Hugging Face ni a las *releases* de llama.cpp | Ejecutar `npm run tauri dev` en un PC con red: descargará el motor y el modelo recomendado |
| **V2** | **Las URLs y tamaños del catálogo embebido** (`catalog/data.rs`) | Sin acceso a Hugging Face para comprobarlos | Un paso del *pipeline* de publicación debe hacer un `HEAD` a cada URL y anclar el SHA-256 que devuelve `X-Linked-Etag`. Hasta entonces, **el catálogo de serie no está confirmado en vivo** |
| **V3** | **Las URLs del motor** (`runtime::llamacpp::engine_pins`) | Ídem | Mismo paso del *pipeline*; los tests ya comprueban que son HTTPS y que corresponden a la versión fijada |
| **V4** | **Rutas CUDA / Metal / Vulkan / OpenCL** | El contenedor no tiene GPU | Los *parsers* de detección y la lógica de clasificación sí están probados con salidas reales fijadas de `nvidia-smi`, `Win32_VideoController` y `system_profiler`. Falta probarlo sobre hardware |
| **V5** | **El adaptador de Ollama contra Ollama real** | No hay demonio Ollama y `ollama.com` está bloqueado | Probado el parseo de `/api/tags` y `/api/pull` y el sondeo contra un puerto cerrado. Falta un equipo con Ollama instalado |
| **V6** | **Los instaladores `.dmg` y NSIS** | El *bundler* de Tauri solo genera el instalador de la plataforma anfitriona, y Linux no es un objetivo del producto | `npm run tauri build` en macOS y en Windows. El *crate* del shell sí compila y pasa clippy |
| **V7** | **Firma y notarización** | Requiere certificados de Naturgy | Paso posterior del *pipeline* de publicación |
| **V8** | **Proxy y CA corporativos reales** | No hay un proxy de Naturgy disponible aquí | Probada la construcción del cliente: se rechaza un proxy sin esquema, se acepta uno bien formado, y un *bundle* de CAs ausente se reporta con su ruta. Falta probarlo contra la red corporativa |
| **V9** | **La ventana de Tauri en ejecución** | Requiere un escritorio gráfico; Linux no es plataforma soportada | La misma interfaz, el mismo núcleo y la misma forma de eventos se validan por el host HTTP; lo que no se ha ejecutado es la ventana nativa |

---

## 5. Reproducir la validación

```bash
cd factoria-desktop

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

npm ci
npm run check
npm test
npm run build

cargo build -p factoria-server
npm run test:e2e          # deja las capturas en screenshots/
```

En Linux, el *crate* del shell Tauri necesita `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libayatana-appindicator3-dev`, `librsvg2-dev` y `libsoup-3.0-dev` para compilar.

Para probar a mano sin navegador automatizado:

```bash
cargo run -p factoria-server -- --static dist
# imprime: FACTORIA_READY http://127.0.0.1:<puerto>/?token=<token>
```
