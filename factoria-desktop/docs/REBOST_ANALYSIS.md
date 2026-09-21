# Fase 1 — Discovery: análisis de Rebost

> Fuente analizada: `https://github.com/Frontierz-AI/Rebost` (tag de trabajo: v0.9.3, clon del 2026-09-20)
> y su documentación pública en `rebost.ai`.

Este documento es el resultado del estudio previo a cualquier modificación. Recoge lo que Rebost
resuelve bien (y por tanto **no** se reinventa), lo que no encaja con el caso corporativo de Naturgy,
y las decisiones que se derivan para FactorIA Desktop.

---

## 1. Qué es Rebost

Aplicación de escritorio (macOS y Windows) que ejecuta un LLM **en local** y responde preguntas sobre
documentos del propio equipo. Producto vertical de RAG documental local:

- **Chat** — conversación con el modelo local.
- **Shelf (estantería)** — una carpeta del disco que el chat puede consultar; el contenido se indexa localmente.
- **Recipes** — prompts guardados y reutilizables.
- **House rules** — instrucciones permanentes inyectadas en el *system prompt*.
- **Privacy Lens** — recuento de PII detectada en los documentos.

Licencia **MIT** (`Copyright (c) 2026 Frontierz`). Sin cuenta, sin *tier* de pago, sin telemetría de producto.

---

## 2. Arquitectura

```
Svelte 5 (webview)  ──invoke + eventos rebost://…──►  Comandos Tauri (adaptadores finos)
                                                             │
        ┌──────────┬───────────┬──────────┬──────────┬───────┴──────┐
        ▼          ▼           ▼          ▼          ▼              ▼
      shelf      ingest      search      chat      engine          pii
     watcher   extract/OCR  tantivy    prompts   llama-server    privacy lens
```

Patrón muy claro y sano: **la capa IPC es un adaptador fino** y toda la lógica vive en módulos Rust
desacoplados del *shell*. Esto es exactamente lo que permite reutilizar el núcleo en otro *host*.

### 2.1 Stack tecnológico

| Capa | Tecnología |
|------|-----------|
| Shell de escritorio | Tauri 2.11 (Rust) |
| UI | Svelte 5 + TypeScript + Tailwind CSS 4 + Vite 8 |
| Iconos | `@lucide/svelte` |
| Markdown | `marked` + `dompurify` |
| Core | Rust 1.98, edition 2021 |
| Inferencia | `llama-server` de **llama.cpp** (proceso hijo en `127.0.0.1`) |
| Búsqueda | Tantivy 0.26 + `tantivy-stemmers` |
| Extracción documental | `xberg` (PDF, Office, email, texto) + OCR Tesseract vendorizado |
| PII | `pii-vault` |
| HTTP | `reqwest` 0.13 |
| Hardware | `sysinfo` 0.39 |
| i18n | `rust-i18n` + catálogos JSON en `locales/` |
| Tests | `vitest` (TS), `cargo test` (Rust), tests de *retrieval* y *experience quality* |
| Lint/format | `oxlint`, `prettier`, `rustfmt`, `cargo deny`, `cargo audit` |
| Package manager | `pnpm` 11 |

### 2.2 Runtime de inferencia

Rebost está **acoplado a un único runtime**: llama.cpp.

- No usa el mecanismo `externalBin` de Tauri; empaqueta un **archivo comprimido *pinneado*** de
  `llama-server` como recurso del instalador y lo descomprime en el directorio de datos de la app.
- `engine/pin.rs` fija `ENGINE_RELEASE` (semver oficial), `ENGINE_BUILD` (tag de GitHub que aloja los
  binarios) y un mapa `ENGINE_PINS` con **URL + SHA-256 por SO/arquitectura**.
- `ENGINE_OPTIONAL_PINS`: *builds* con aceleración mejor (CUDA en Windows+NVIDIA, OpenCL Adreno en
  Windows ARM) que se descargan en el primer arranque si el hardware encaja. Si fallan, cae al *pin*
  empaquetado (Vulkan o CPU) y no vuelve a intentarlo en esa sesión.
- `engine/tune.rs` (808 líneas) calcula *flags* de arranque según máquina y modelo: contexto 4k–16k,
  `--cache-type-k/v q8_0`, `-fa on|auto`, `-ngl 0` en CPU, `--no-mmap` en Vulkan/CUDA discreta…
- `engine/process.rs` / `ready.rs`: ciclo de vida del proceso hijo, `/health`, *warmup*, sustitución
  del modelo sin cortar el chat, limpieza de PIDs huérfanos.
- `engine/stream.rs` (615 líneas): consumo del *stream* y separación de razonamiento/respuesta.
- `engine/bench.rs`: calibración de velocidad ligada a modelo + build + acelerador + contexto.

**Conclusión:** el *know-how* de llama.cpp está muy bien resuelto, pero está **incrustado** en el
módulo `engine`. No existe abstracción de runtimes. Ollama aparece solo como *fuente de catálogo*
(`engine/models/ollama.rs`, 146 líneas: búsqueda en la librería de Ollama), **no** como motor de ejecución.

### 2.3 Gestión y descarga de modelos

- **Catálogo curado y *hardcodeado***: `engine/catalog.rs` (679 líneas). Filas ordenadas por capacidad,
  nunca se obtienen de red. Cada fila: nombre, familia, proveedor, repo HF, bytes aproximados, licencia,
  fecha, descripción y un `CatalogStanding` (`Scored(index)` de Artificial Analysis, o `BenchLead { above }`).
- **Recomendación** (`recommend()`): elige **la primera fila del catálogo que quepa**. Criterio:

  ```
  runtime_need = file_bytes * copies + 2 GiB        (copies = 2.0 en Vulkan/CUDA, 1.15 en el resto)
  cabe si:      runtime_need <= RAM_total * 0.65
  ```

- **Explore other AIs**: modal que busca en Hugging Face (GGUF `text-generation`) y mezcla Ollama cuando
  hay consulta. Marca **Official** solo para *namespaces* de laboratorio original (`ORIGINAL_MAKERS`).
  Oculta especialistas (OCR, layout, coder) salvo búsqueda explícita.
- **Descarga** (`engine/download.rs`, 1.528 líneas): resolución de GGUF de fichero único, exigencia de
  SHA-256, progreso por eventos, sustitución de pesos solo cuando el nuevo proceso está *Ready*.
- **Validación GGUF** (`engine/gguf.rs`): lee la cabecera GGUF (magic, versión, KV) para obtener
  `context_length` y comprobar que los tipos de tensor los soporta el build *pinneado*.

### 2.4 Detección de hardware

`catalog.rs::MachineProfile::detect()` es **deliberadamente mínimo**:

| Detecta | Cómo |
|---------|------|
| RAM total | `sysinfo` |
| CPU (marca) | `sysinfo`, primer core |
| Disco libre | `sysinfo::Disks`, filtrado por el *mount point* del directorio de datos |
| Arquitectura de proceso y de SO | `std::env::consts::ARCH` + `IsWow64Process2` en Windows |
| "Acelerador" | **derivado del *pin* de motor elegido**, no de una consulta real a la GPU |

`engine/gpu.rs` (134 líneas) **no enumera GPUs**: solo comprueba la presencia de `nvcuda.dll`/`nvml.dll`
(Windows x64) o DLLs Adreno / CPU Snapdragon (Windows ARM) para decidir qué archivo de motor bajar.

> **No hay detección de VRAM, ni de modelo de GPU, ni de número de núcleos, ni de nivel de
> instrucciones, ni de clasificación de aptitud por modelo.** Esto es exactamente el hueco que
> FactorIA Desktop tiene que llenar (requisitos 5 y 6 del encargo).

### 2.5 Configuración y persistencia

Directorio de datos del SO (`src-tauri/src/paths.rs`):
`~/Library/Application Support/io.rebost.desktop/` (macOS) · `%APPDATA%\io.rebost.desktop\` (Windows).

```
library/                  carpetas gestionadas de Shelf
shelves/<id>/{cards,extracted,documents.json}
search/tantivy/
conversations/            threads.json + un JSONL por hilo
models/                   ficheros GGUF
engine/<release>-<accel>/ llama-server
recipes.json · settings.json · instance.lock · logs/engine.log
```

Configuración = `settings.json` **local y solo local**. No hay configuración centralizada, ni política
corporativa, ni *allowlist*, ni canal de actualización gestionado.

### 2.6 Instalación y *packaging*

- Tauri bundler: `.dmg` (macOS, Apple Silicon + Intel) y NSIS (Windows x64 + ARM64). **Sin instalador Linux.**
- Windows 10 obtiene WebView2 desde el instalador; Windows 11 ya lo trae.
- El instalador incluye el archivo del motor: `pnpm tauri build` ejecuta antes `scripts/fetch-engine.mjs`.
- Firma: *codesign* de cada Mach-O dentro del archivo + notarización en macOS.
- Actualizador: `tauri-plugin-updater` contra `latest.json` en GitHub Releases, instalación in-app.
- Instancia única: `instance.lock` con `fs4`; una segunda copia enfoca la primera ventana.

### 2.7 APIs

**No expone ninguna API de red.** El contrato es:

- **Comandos Tauri** (`src-tauri/src/commands/{chat,model,recipes,settings,shelves}.rs`), tipados en
  `src/lib/api.ts` en el frontend.
- **Eventos** `rebost://engine|download|ingest|shelf-stats|shelves|chat|update|update-progress`.
- Internamente, HTTP a `127.0.0.1:<puerto>` contra `llama-server` (protocolo compatible OpenAI).

`src-tauri/capabilities/default.json` restringe los permisos del webview.

### 2.8 Seguridad y privacidad

Bien planteado para consumo, con matices:

- Los documentos no salen del equipo. La red se usa para: buscar/instalar modelos (Hugging Face, Ollama),
  comprobar actualizaciones (GitHub) y, opcionalmente, *Online* (Wikipedia / DuckDuckGo / You.com), que
  está **apagado por defecto** y pide aprobación por consulta.
- Verificación SHA-256 de motor y de pesos. Descompresión "*path-safe*" de tar/zip.
- `SECURITY.md` con canal de *disclosure*. `cargo deny` + `cargo audit` en CI.
- **Privacy Lens son recuentos, no cifrado en reposo**: el texto extraído vive en claro en
  `extracted/*.md`, en el índice y en el JSONL de conversaciones. La documentación de Rebost lo admite
  explícitamente. Para Naturgy esto es un punto a tratar (ver §5).

### 2.9 CI/CD

`.github/workflows/`: `ci.yml` (lint + typecheck + tests TS y Rust + HEAD-check de las URLs de los *pins*),
workflows de release por plataforma (`release-windows.yml` con runners x64 y `windows-11-arm`), firma y
notarización en macOS. `justfile` con tareas de desarrollo.

---

## 3. Componentes reutilizables para FactorIA Desktop

| Componente de Rebost | Reutilización | Justificación |
|---|---|---|
| **Patrón arquitectónico** (IPC fina + módulos Rust) | **Adoptado y reforzado** | Se lleva un paso más allá: el núcleo pasa a ser un *crate* propio (`factoria-core`) para poder alojarlo en Tauri **y** en un servidor HTTP local. |
| **Stack** Tauri 2 + Svelte 5 + Tailwind 4 + Vite | **Adoptado** | Binario pequeño, sin Chromium empaquetado, sin Node en el equipo del empleado. Encaja con "el empleado no debe saber nada". |
| **Estrategia de *pin* del motor** (release + build + URL + SHA-256 por SO/arch, opcionales por GPU) | **Adoptado, generalizado** | Es el mecanismo correcto para que el empleado no instale llama.cpp a mano. En FactorIA se convierte en `EnginePin` dentro del adaptador `llamacpp` y admite un **mirror corporativo**. |
| **Cálculo de necesidad de RAM** (`file_bytes * copies + 2 GiB`) | **Adoptado como base**, ampliado | Heurística sensata y reproducible. FactorIA la extiende con VRAM, disco y contexto (ver `ARCHITECTURE.md` §6). |
| **Catálogo curado y *hardcodeado*** | **Adoptado, cambiando el origen** | En Naturgy el catálogo debe ser **autorizado**: fila embebida + *override* por fichero de política corporativa. |
| **Lectura de cabecera GGUF** | **Adoptado (reimplementado, más pequeño)** | Necesario para leer `context_length` y validar el fichero descargado. |
| **Descarga con progreso + verificación SHA-256** | **Adoptado (reimplementado)** | Las 1.528 líneas de `download.rs` cubren casos que el MVP no necesita; se toma el diseño, no el código. |
| **Detección de acelerador por presencia de DLL** | **Adoptado y ampliado** | Se añade enumeración real de GPU y VRAM en los tres SO. |
| Ingest + Tantivy + OCR + PII (RAG documental) | **No en el MVP** | Es la mitad del producto Rebost, pero el encargo pide primero *detectar → catálogo → instalar → ejecutar → chat*. La arquitectura deja el hueco (`docs/ARCHITECTURE.md` §9). |
| Shelves / Recipes / House rules | **No en el MVP** | Ídem. *House rules* se reduce a "instrucciones del sistema" en ajustes. |
| Avatares de verduras, marca Rebost | **Descartado** | Identidad de producto de consumo, incompatible con el tono corporativo. |

---

## 4. Riesgos detectados

| # | Riesgo | Impacto | Mitigación en FactorIA |
|---|--------|---------|------------------------|
| R1 | **Acoplamiento total a llama.cpp** | Alto | Trait `LlmRuntime` + registro de runtimes desde el día 1. |
| R2 | **Dependencia de descargas desde Hugging Face y GitHub Releases** | Alto en corporativo (proxy/egress) | `SourceResolver` con *mirror* corporativo y `allowlist`; proxy y CA de empresa configurables. |
| R3 | **Los pesos y el texto se guardan en claro** | Medio/Alto | Por defecto no se indexan documentos en el MVP; el registro de auditoría no guarda contenido; queda documentado como limitación conocida. |
| R4 | **Sin configuración centralizada** | Alto para despliegue masivo | `policy.json` leído de una ruta gestionada (GPO/Intune/Jamf) que **prevalece** sobre los ajustes locales. |
| R5 | **Sin Linux** | Bajo/Medio | El núcleo compila y se prueba en Linux; el *bundle* de escritorio sigue siendo macOS/Windows. |
| R6 | **Detección de hardware insuficiente** para recomendar | Alto (es el requisito 5) | Módulo `hardware` propio con GPU/VRAM y clasificación reproducible. |
| R7 | **`xberg`, `pii-vault`, Tesseract vendorizado** pesan mucho en el *build*| Medio | Fuera del MVP; se añadirán cuando entre RAG. |
| R8 | **Un modelo activo a la vez**, generaciones serializadas | Bajo | Se mantiene (es correcto en un PC), pero el `RuntimeRegistry` permite varios runtimes en paralelo. |
| R9 | Comprobación de actualizaciones contra GitHub por defecto | Medio en corporativo | Canal de actualización configurable y **desactivable** por política. |

---

## 5. Licencia y atribución

- Rebost es **MIT** — `Copyright (c) 2026 Frontierz`. Permite uso, modificación y redistribución,
  incluso comercial y cerrada, **siempre que se conserven el aviso de copyright y el texto de la licencia**.
- FactorIA Desktop se publica también bajo **MIT**, conservando el copyright de Frontierz junto al de
  Naturgy, y con un fichero `NOTICE.md` que declara la obra derivada y qué se ha tomado.
- **Los pesos de los modelos no están cubiertos por la licencia de la aplicación.** Cada modelo lleva la
  suya (Apache-2.0, MIT, Gemma Terms, Llama Community License…). La UI muestra la licencia **antes** de descargar
  y el catálogo corporativo debe registrarla. Es un requisito legal, no cosmético.
- Los binarios de `llama.cpp` (MIT) se descargan y se verifican por SHA-256; su aviso va en `NOTICE.md`.
- **Marca**: no se reutiliza ningún activo de marca de Rebost. Tampoco se incorporan al repositorio
  activos de marca protegidos de Naturgy: la aplicación define *design tokens* corporativos y deja un
  hueco (`src/assets/brand/`) documentado para que el equipo de marca coloque el logotipo oficial.

---

## 6. Cambios necesarios (resumen ejecutivo)

1. **Extraer el núcleo a un *crate* propio** desacoplado de Tauri → permite *host* Tauri y *host* HTTP local
   (y, más adelante, un servicio o una CLI).
2. **Introducir una abstracción de runtimes** (`LlmRuntime`) con adaptadores `llamacpp` y `ollama`.
3. **Construir un módulo de hardware real** (SO, CPU, núcleos, arquitectura, RAM, GPU, VRAM, disco) con
   clasificación **Óptimo / Compatible / No recomendado** basada en criterios documentados.
4. **Catálogo visual** con metadatos completos y estados por modelo, alimentado por catálogo embebido +
   política corporativa.
5. **Nueva Home** ("Mi PC", recomendados, instalados, recursos, acciones rápidas).
6. **Chat propio** con *streaming*, parada, regeneración e indicador de ejecución local **verificable**.
7. **Capa de política corporativa**: proxy, CA, allow/denylist, telemetría apagada, auditoría local,
   actualizaciones controladas, configuración centralizada.
8. **Re-branding completo** a la identidad Naturgy.
9. Retirar del MVP: ingest, Tantivy, OCR, PII, Shelves, Recipes, i18n multi-idioma (se queda en `es`).
