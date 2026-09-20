# Fase 2 — Arquitectura objetivo de FactorIA Desktop

> FactorIA Desktop · *Tu IA. En tu PC.*
> Obra derivada de [Rebost](https://github.com/Frontierz-AI/Rebost) (MIT). Ver `NOTICE.md`.

---

## 1. Principio rector

```
Instalar → detectar hardware → recomendar modelos → descargar → ejecutar → conversar
```

El empleado **no** ve Docker, Python, CUDA, llama.cpp, Ollama, terminales ni parámetros de inferencia.
Todo eso es responsabilidad del núcleo y se resuelve con valores derivados del hardware detectado.

Regla de diseño: *si una pantalla obliga a decidir algo que el sistema puede deducir del hardware,
la pantalla está mal diseñada.*

---

## 2. Vista general

```
┌──────────────────────────────────────────────────────────────────────┐
│  UI  ·  Svelte 5 + TypeScript + Tailwind 4  (Home · Catálogo · Chat · Ajustes) │
└───────────────────────────────┬──────────────────────────────────────┘
                                │  lib/bridge.ts  (una sola interfaz)
                 ┌──────────────┴──────────────┐
                 ▼                             ▼
      Tauri IPC (invoke + event)      HTTP + SSE (127.0.0.1)
                 │                             │
        ┌────────┴────────┐          ┌─────────┴─────────┐
        │   src-tauri     │          │  factoria-server  │
        │  (shell nativo) │          │  (axum, headless) │
        └────────┬────────┘          └─────────┬─────────┘
                 └──────────────┬──────────────┘
                                ▼
        ╔═══════════════════════════════════════════════════════╗
        ║                   factoria-core                       ║
        ║   crate Rust puro, sin dependencia de shell ni de UI   ║
        ╟───────────────────────────────────────────────────────╢
        ║ hardware │ catalog │ runtime │ chat │ policy │ audit   ║
        ║          │  + fit  │ registry│      │        │ metrics║
        ╚══════════╧═════════╧════╤════╧══════╧════════╧════════╝
                                  │  trait LlmRuntime
                 ┌────────────────┼────────────────┬──────────────┐
                 ▼                ▼                ▼              ▼
           llama.cpp          Ollama            MLX*          futuro*
       (proceso hijo,      (demonio local,   (Apple)       (vLLM, ONNX…)
        HTTP local)         HTTP local)
                                                       * no implementado en el MVP
```

### 2.1 Por qué un núcleo separado del shell

Rebost mete la lógica dentro del *crate* de Tauri. FactorIA la saca a `factoria-core` por tres razones
concretas, no por purismo:

1. **Verificabilidad.** El núcleo compila y se prueba en Linux/CI sin WebKit ni GUI. En Rebost, probar
   el catálogo obliga a compilar todo el *shell*.
2. **"APIs locales" del requisito 11.** El host HTTP no es un invento para el MVP: es la superficie que
   permitirá que otras herramientas internas de Naturgy hablen con la IA local del empleado.
3. **Futuro servicio/CLI.** Un despliegue masivo puede querer un *daemon* o una CLI de diagnóstico
   sin ventana. Con el núcleo separado es añadir un `main.rs`.

El coste es una capa de adaptadores duplicada (Tauri y HTTP), que es deliberadamente trivial:
ambos hosts solo traducen entrada/salida hacia el mismo `AppCore`.

---

## 3. Mapa de módulos

### `crates/factoria-core`

| Módulo | Responsabilidad |
|--------|-----------------|
| `hardware` | Detección de SO, CPU, núcleos, arquitectura, RAM, GPU, VRAM, disco y capacidades de inferencia. |
| `catalog` | Catálogo de modelos + `FitLevel` (Óptimo / Compatible / No recomendado) + recomendación. |
| `runtime` | `trait LlmRuntime`, `RuntimeRegistry` y adaptadores (`llamacpp`, `ollama`). |
| `runtime::llamacpp` | *Pins* de `llama-server`, arranque/parada del proceso hijo, `/health`, *flags* derivados del hardware. |
| `runtime::ollama` | Habla con el demonio local de Ollama si está presente. |
| `download` | Descarga con progreso, reanudación y verificación SHA-256. |
| `gguf` | Lectura mínima de la cabecera GGUF (`context_length`, arquitectura, validación). |
| `chat` | Conversaciones, persistencia JSONL, orquestación del *stream*, cancelación, regeneración. |
| `policy` | Política corporativa: proxy, CA, allow/denylist, telemetría, actualizaciones, catálogo autorizado. |
| `audit` | Registro local *append-only* de eventos (sin contenido de prompts). |
| `metrics` | TTFT, tokens/s, muestreo de CPU/RAM. |
| `store` | Ajustes, rutas de datos, estado de modelos instalados. |
| `events` | Bus de eventos interno (`tokio::broadcast`) que ambos hosts reexponen. |

### `crates/factoria-server`
Host HTTP local (axum). Sirve el frontend compilado y expone `/api/*` + `/api/events` (SSE).
Escucha **solo en `127.0.0.1`**, con puerto efímero por defecto.

### `src-tauri`
Shell de escritorio. Comandos `#[tauri::command]` que llaman a `AppCore` y reemiten los eventos del bus
como eventos Tauri `factoria://…`.

### `src`
Frontend Svelte 5. `lib/bridge.ts` detecta si está dentro de Tauri (`window.__TAURI_INTERNALS__`) y
elige IPC o `fetch`; el resto de la UI no sabe en qué host corre.

---

## 4. Abstracción de runtimes

```rust
#[async_trait]
pub trait LlmRuntime: Send + Sync {
    fn id(&self) -> RuntimeId;                  // "llamacpp" | "ollama" | …
    fn descriptor(&self) -> RuntimeDescriptor;  // nombre visible, capacidades
    async fn probe(&self) -> RuntimeStatus;     // disponible / no instalado / error
    async fn installed_models(&self) -> Result<Vec<InstalledModel>>;
    async fn install(&self, spec: &ModelSpec, progress: ProgressSink) -> Result<InstalledModel>;
    async fn remove(&self, model_id: &str) -> Result<()>;
    async fn start(&self, model_id: &str, tuning: &Tuning) -> Result<RunningModel>;
    async fn stop(&self, model_id: &str) -> Result<()>;
    async fn chat_stream(&self, req: ChatRequest, sink: TokenSink, cancel: CancelToken) -> Result<ChatOutcome>;
}
```

Notas de diseño:

- Ambos adaptadores actuales hablan **HTTP con un endpoint local**, lo que hace la abstracción barata:
  llama.cpp expone un API compatible con OpenAI y Ollama expone el suyo. La diferencia real está en el
  **ciclo de vida** (llama.cpp es un proceso que nosotros lanzamos; Ollama es un demonio que ya existe)
  y en la **instalación** (GGUF descargado vs `ollama pull`). Por eso el trait incluye `install`/`start`,
  no solo `chat`.
- `RuntimeRegistry` ordena los runtimes por prioridad y resuelve el primero disponible; un `ModelSpec`
  declara qué runtimes lo pueden ejecutar.
- Un runtime futuro (MLX en Apple Silicon, ONNX Runtime, vLLM…) implementa el trait y se registra.
  Nada fuera de `runtime/` cambia.

---

## 5. Detección de hardware

`hardware::detect()` produce un `HardwareProfile`:

| Campo | Fuente |
|-------|--------|
| `os`, `os_version`, `arch` | `sysinfo::System` + `std::env::consts` |
| `cpu_brand`, `physical_cores`, `logical_cores`, `cpu_mhz` | `sysinfo` |
| `total_ram_bytes`, `available_ram_bytes` | `sysinfo` |
| `free_disk_bytes` (del volumen de datos) | `sysinfo::Disks` filtrado por *mount point* |
| `gpus[]`: `vendor`, `name`, `vram_bytes`, `kind` (integrada/discreta/unificada) | ver §5.1 |
| `accelerator` | derivado (§5.2) |

### 5.1 Cómo se obtiene la GPU y la VRAM

Sin dependencias pesadas y sin librerías de vendor: se intentan en orden y el primero que responde gana.

| SO | Estrategia |
|----|-----------|
| **Windows** | `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits` → si no, WMI vía `Get-CimInstance Win32_VideoController` (`AdapterRAM`, con el límite conocido de 4 GiB de ese campo). |
| **macOS** | `system_profiler SPDisplaysDataType -json` (nombre, `sppci_model`, VRAM); en Apple Silicon la memoria es **unificada** → `vram = RAM total`, marcada como `Unified`. |
| **Linux** | `nvidia-smi`; si no, `/sys/class/drm/card*/device/mem_info_vram_total` (AMD) y `vendor`/`device` de PCI para el nombre. |

Si nada responde, `gpus` queda vacío y la clasificación funciona **solo con RAM y CPU** — es un
degradado correcto, no un error.

### 5.2 Acelerador efectivo

```
macOS + arm64                     → Metal (memoria unificada)
Windows/Linux + GPU NVIDIA        → CUDA
Windows + GPU AMD/Intel con Vulkan→ Vulkan
Windows ARM + Adreno              → OpenCL
resto                             → CPU
```

---

## 6. Criterios de compatibilidad (reproducibles)

Se calculan en `catalog::fit`. **Deterministas**: mismo hardware + mismo modelo ⇒ misma etiqueta.
Cada evaluación devuelve además las **razones**, que la UI muestra tal cual.

### 6.1 Memoria necesaria en ejecución

```
weights      = tamaño del fichero de pesos (bytes)
kv_cache     = ctx_tokens × kv_bytes_por_token(modelo)      (≈ 2 × n_layers × n_kv_heads × head_dim × bytes_por_elemento)
overhead     = 1.0 GiB   (proceso del runtime, grafo de cómputo, buffers)
copies       = 2.0 si el acelerador copia los pesos fuera de mmap (CUDA, Vulkan discreta)
             = 1.15 en el resto (Metal/unificada, CPU con mmap, OpenCL)

need_total   = weights × copies + kv_cache + overhead
```

> Herencia directa de Rebost (`file_bytes × copies + 2 GiB`), separando el término fijo en
> *KV cache* real + *overhead*, para que el contexto influya en la etiqueta.

### 6.2 Presupuestos de la máquina

```
budget_ram   = RAM_total × 0.65      (deja SO, navegador, ofimática y la propia app)
budget_vram  = VRAM × 0.90           (solo si hay GPU dedicada utilizable)
budget_disk  = disco_libre − 2 GiB   (margen de seguridad del volumen de datos)
```

### 6.3 Clasificación

| Etiqueta | Condición |
|----------|-----------|
| **Óptimo** | `need_total ≤ budget_ram` **y** (el modelo cabe entero en `budget_vram`, **o** el sistema es de memoria unificada con `need_total ≤ budget_ram`) **y** `weights ≤ budget_disk` |
| **Compatible** | `need_total ≤ budget_ram` **y** `weights ≤ budget_disk`, pero sin GPU capaz de alojarlo entero → se ejecutará en CPU o con descarga parcial: funciona, más lento |
| **No recomendado** | `need_total > budget_ram` **o** `weights > budget_disk` **o** el runtime requerido no está disponible en esta máquina |

Reglas adicionales, todas explicitadas al usuario como razones:

- `weights > budget_disk` → *No recomendado* con la razón "espacio en disco insuficiente" (gana sobre
  cualquier otra, porque ni siquiera se puede descargar).
- Sin GPU y `physical_cores < 4` → un modelo *Compatible* se degrada a *No recomendado* si
  `weights > 3 GiB` (el rendimiento en CPU sería inusable: por debajo de ~2 tok/s en la mayoría de equipos).
- Runtime no disponible (p. ej. modelo solo-Ollama y Ollama no instalado) → *No recomendado*,
  razón accionable ("requiere Ollama").

La estimación de velocidad que muestra la tarjeta (`~X tok/s`) es una **orientación**, derivada del
ancho de banda de memoria típico del acelerador, y se etiqueta como estimación en la UI. En cuanto se
ejecuta el modelo, la cifra real medida sustituye a la estimada.

### 6.4 Recomendación

`catalog::recommend(profile)` devuelve las filas ordenadas por `(fit_rank, capability_rank)` y marca
como **recomendado principal** la de mayor capacidad con etiqueta *Óptimo*; si no hay ninguna,
la de mayor capacidad *Compatible*. Es la misma política de Rebost ("la primera fila que cabe"),
explicitada y con la etiqueta de aptitud como criterio primario.

---

## 7. Datos en disco

```
<APP_DATA>/es.naturgy.factoria/
├── settings.json          ajustes del usuario
├── policy.json            (solo lectura) política corporativa, si la hay en local
├── models/                pesos GGUF descargados  +  models.json (registro de instalados)
├── engines/<runtime>/<release>-<accel>/   binarios de runtime extraídos
├── conversations/         threads.json + un .jsonl por conversación
├── logs/                  factoria.log, engine.log
└── audit/                 audit-YYYY-MM.jsonl  (append-only, sin contenido de prompts)
```

Rutas por SO: `%APPDATA%\es.naturgy.factoria\` · `~/Library/Application Support/es.naturgy.factoria/`
· `$XDG_DATA_HOME/es.naturgy.factoria/`.

---

## 8. Entorno corporativo

Diseñado desde el principio, implementado en el MVP como **capa de política efectiva**.

### 8.1 Orden de precedencia de la configuración

```
1. Política gestionada (la que manda)
     Windows : %PROGRAMDATA%\Naturgy\FactorIA\policy.json   (o HKLM\SOFTWARE\Policies\Naturgy\FactorIA)
     macOS   : /Library/Application Support/Naturgy/FactorIA/policy.json  (o perfil MDM)
     Linux   : /etc/naturgy/factoria/policy.json
2. Variable de entorno FACTORIA_POLICY_FILE   (pruebas y pilotos)
3. Ajustes del usuario (settings.json)        — solo donde la política lo permite
4. Valores por defecto del producto
```

Cada campo de la política lleva `locked: true|false`. Un campo bloqueado se muestra en la UI
en gris con un indicador **"Gestionado por Naturgy"** y no es editable. Esto es lo que permite un
despliegue masivo por GPO/Intune/Jamf **sin** un servicio de configuración en la nube.

### 8.2 Contenido de la política

| Clave | Efecto |
|-------|--------|
| `network.proxy` | Proxy HTTP(S) corporativo para descargas (`http_proxy`/`https_proxy` o explícito). |
| `network.ca_bundle` | Ruta al *bundle* de CAs corporativas (TLS con inspección). |
| `network.offline` | Modo aislado: ninguna salida de red; solo modelos ya presentes o del *mirror*. |
| `catalog.source` | `embedded` \| `file:` \| `https://` — catálogo de modelos **autorizado**. |
| `catalog.mirror_base_url` | *Mirror* interno de pesos y binarios de motor (sustituye a Hugging Face / GitHub). |
| `catalog.allowlist` / `catalog.denylist` | Patrones de id de modelo permitidos / prohibidos. La denylist gana. |
| `updates.channel` | `off` \| `manual` \| `auto`, y `updates.feed_url` para un feed interno. |
| `telemetry.enabled` | **`false` por defecto.** Si se activa, solo eventos agregados (§8.3). |
| `audit.enabled` | Registro local de acciones. Activado por defecto, **sin contenido**. |
| `chat.system_prompt` | Instrucciones corporativas inyectadas en todas las conversaciones. |

### 8.3 Qué sale del equipo, y qué no

**Nunca** salen del equipo, por diseño y sin opción de activarlo en el MVP:
prompts, respuestas, conversaciones, documentos, nombres de fichero, datos personales.

Salidas de red que **sí** existen y son visibles en Ajustes → Red:

| Destino | Cuándo | Contenido |
|---------|--------|-----------|
| Origen del catálogo (mirror corporativo o Hugging Face) | Al instalar un modelo | Petición del fichero de pesos |
| Origen del binario del runtime (mirror o GitHub Releases) | Primer arranque del motor | Petición del archivo |
| Feed de actualizaciones | Si `updates.channel ≠ off` | Versión instalada |
| Telemetría | Solo si `telemetry.enabled = true` | Contadores agregados: versión, SO, clase de hardware, modelos instalados, nº de conversaciones. **Sin texto.** |

La inferencia va a `127.0.0.1`. El indicador **"Procesando localmente"** de la UI no es un adorno:
lo emite el núcleo cuando el *endpoint* que atiende la generación resuelve a *loopback*, y se apagaría
si algún día existiera un runtime remoto.

### 8.4 Auditoría

`audit/audit-YYYY-MM.jsonl`, una línea por evento:
`{ts, event, actor:"local-user", model_id, runtime, bytes, duration_ms, outcome}`.
Eventos: instalación, borrado, arranque/parada de modelo, cambio de política, inicio/fin de conversación
(**sin el texto**), fallo de descarga. Pensado para que un agente de recolección corporativo lo recoja.

---

## 9. Preparado para el futuro (sin implementarlo ahora)

Decisiones tomadas hoy solo para no cerrar puertas mañana:

| Capacidad futura | Qué la habilita ya |
|---|---|
| **RAG local / Documentos / Embeddings** | El núcleo separado permite añadir `crates/factoria-rag` sin tocar los hosts. `ChatRequest` ya lleva `context_blocks: Vec<ContextBlock>` (hoy vacío) y la respuesta lleva `citations` (hoy vacío). |
| **Tool calling / MCP / Skills / Agentes** | `ChatRequest` lleva `tools: Vec<ToolSpec>` y el *stream* distingue `Delta::Text` de `Delta::ToolCall`. El adaptador llama.cpp ya usa el esquema OpenAI, que soporta herramientas. |
| **APIs locales** | `factoria-server` **es** esa API. Hoy sirve la UI; mañana, con autenticación por token local, sirve a otras herramientas internas. |
| **Modelos corporativos / catálogo centralizado** | `catalog.source` + `catalog.mirror_base_url` ya existen en la política. |
| **SSO / Integraciones Naturgy** | El núcleo no tiene concepto de usuario más allá de `actor` en auditoría; añadir un `IdentityProvider` no rompe nada porque nada asume "un solo usuario anónimo". |
| **Más runtimes (MLX, vLLM, ONNX)** | `trait LlmRuntime` + registro. |

Lo que **no** se hace ahora, conscientemente: no hay base de datos, ni servicio en la nube, ni cola de
mensajes, ni sistema de plugins, ni capa de abstracción de almacenamiento. Añadirlos hoy sería
*overengineering* según el punto 12 del encargo.

---

## 10. Seguridad

- Escucha **solo** en `127.0.0.1`; el host HTTP exige una cabecera con un token de sesión efímero
  generado al arrancar y comprueba `Origin` (defensa frente a *DNS rebinding*).
- Descargas verificadas por **SHA-256** declarado en el catálogo; un fichero que no cuadra se descarta.
- Descompresión de tar/zip a prueba de *path traversal*.
- Los pesos GGUF se validan (cabecera) antes de activarse.
- Capacidades de Tauri restringidas; CSP sin `unsafe-eval`; sin CDNs externas (fuentes y recursos
  empaquetados → funciona detrás de un proxy corporativo y sin internet).
- Markdown del chat renderizado y **saneado** antes de insertarse.
- Ningún secreto en el repositorio; la configuración sensible vive en la política gestionada.

---

## 11. Decisiones registradas

| # | Decisión | Alternativa descartada | Motivo |
|---|---|---|---|
| D1 | Tauri 2 + Svelte 5 (igual que Rebost) | Electron | Binario ~10× menor, sin Chromium ni Node en el PC del empleado; y mantiene abierta la reutilización del código de Rebost. |
| D2 | Núcleo en *crate* propio | Todo dentro de `src-tauri` como Rebost | Verificable en CI/Linux y reutilizable por el host HTTP. |
| D3 | Host HTTP local además del shell | Solo IPC de Tauri | Permite validar el producto de extremo a extremo sin GUI y es la futura "API local" del requisito 11. |
| D4 | llama.cpp como runtime primario | Ollama primario | Ollama exige una instalación previa que el empleado no puede hacer; llama.cpp se puede empaquetar y *pinnear*. Ollama queda como adaptador secundario para quien ya lo tenga. |
| D5 | Catálogo embebido + *override* por política | Catálogo siempre remoto | Funciona sin red y el control corporativo es explícito. |
| D6 | Telemetría **apagada** por defecto | Telemetría anónima activa | Requisito 10; y activarla debe ser una decisión de Naturgy, no del producto. |
| D7 | Fuentes y recursos empaquetados, sin CDN | Google Fonts | Funciona sin internet y tras un proxy con inspección TLS. |
| D8 | Sin RAG en el MVP | Heredar el pipeline de Rebost | Punto 12: primero algo que se pueda instalar, abrir y probar. |
