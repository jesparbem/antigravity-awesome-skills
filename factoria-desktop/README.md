<div align="center">

# FactorIA Desktop

**Tu IA. En tu PC.**

Aplicación de escritorio que ejecuta modelos de lenguaje **en el propio equipo** del
empleado. Detecta el hardware, recomienda lo que encaja, lo instala y permite
conversar — sin que los prompts salgan del PC.

</div>

---

## Qué es

Un empleado instala FactorIA, la abre, y la aplicación:

1. **Analiza su equipo** — sistema, procesador, núcleos, memoria, gráfica, VRAM y disco.
2. **Le dice qué modelos puede ejecutar**, clasificados en *Óptimo*, *Compatible* o
   *No recomendado*, con el motivo explicado en castellano llano.
3. **Instala el que elija** con un clic, verificando su integridad.
4. **Lo arranca** con los parámetros deducidos de su hardware.
5. **Le deja conversar**, con la respuesta generada en su propio equipo.

En ningún momento ve Docker, Python, CUDA, llama.cpp, Ollama, una terminal ni un
parámetro de inferencia. Ese es el criterio de diseño: *si una pantalla obliga a
decidir algo que el sistema puede deducir del hardware, la pantalla está mal diseñada*.

FactorIA Desktop es una **obra derivada de [Rebost](https://github.com/Frontierz-AI/Rebost)**
(MIT), reorientada a un despliegue corporativo. Ver [`NOTICE.md`](NOTICE.md) y
[`docs/REBOST_ANALYSIS.md`](docs/REBOST_ANALYSIS.md).

---

## Arquitectura

```
UI · Svelte 5 + TypeScript + Tailwind 4   (Inicio · Catálogo · Chat · Ajustes)
                    │  lib/bridge.ts — una sola interfaz
         ┌──────────┴──────────┐
         ▼                     ▼
   Tauri IPC            HTTP + SSE (127.0.0.1)
   src-tauri            factoria-server
         └──────────┬──────────┘
                    ▼
        ┌───────────────────────────────────┐
        │          factoria-core            │
        │  crate Rust, sin shell ni UI      │
        ├───────────────────────────────────┤
        │ hardware · catalog+fit · runtime  │
        │ chat · policy · audit · metrics   │
        └───────────────┬───────────────────┘
                        │  trait LlmRuntime
            ┌───────────┼───────────┬──────────┐
            ▼           ▼           ▼          ▼
        llama.cpp    Ollama      MLX*      futuro*
                                        * no implementado
```

Tres decisiones que explican el resto:

- **El núcleo no depende del shell.** Toda la lógica vive en `factoria-core`, que
  alojan tanto la aplicación Tauri como un servidor HTTP local. Eso hace el producto
  verificable sin abrir una ventana, y deja lista la *API local* para que otras
  herramientas internas hablen con la IA del empleado.
- **Los motores están detrás de un `trait`.** Rebost está acoplado a llama.cpp;
  FactorIA no. Añadir MLX, ONNX Runtime o vLLM es implementar `LlmRuntime` y
  registrarlo.
- **La política corporativa es una capa de primera clase**, no un añadido: decide el
  catálogo, el proxy, las CAs, el *mirror*, las actualizaciones y la telemetría.

Detalle completo en [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## Requisitos

### Para usarlo

| | |
|---|---|
| **Sistema** | Windows 10/11 (x64 o ARM64) · macOS 12+ (Apple Silicon o Intel) |
| **Memoria** | 8 GB mínimo · 16 GB recomendado |
| **Disco** | 2 GB para la aplicación y el motor, más el tamaño del modelo (0,4–9 GB) |
| **Gráfica** | Opcional. Con GPU dedicada va mucho más rápido; sin ella funciona en CPU |
| **Red** | Solo para descargar el modelo y el motor la primera vez. Después, opcional |

Windows 10 necesita WebView2 (el instalador lo añade si falta). Windows 11 ya lo trae.

### Para desarrollar

| | |
|---|---|
| **Rust** | 1.82+ (`rustup`) |
| **Node** | 20+ |
| **Linux (solo para compilar)** | `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev` |

---

## Instalación

Los instaladores se generan con `npm run tauri build` en cada plataforma:
`.dmg` en macOS, NSIS en Windows.

> **Estado actual:** todavía no hay *release* publicada. Los instaladores aún no se
> han generado ni firmado — ver [Limitaciones](#limitaciones-actuales).

---

## Desarrollo

```bash
git clone https://github.com/jesparbem/antigravity-awesome-skills.git
cd antigravity-awesome-skills/factoria-desktop
npm install
```

### Aplicación de escritorio

```bash
npm run tauri dev
```

### Sin ventana (host HTTP local)

Útil para desarrollar la interfaz, para CI y para probar en un servidor sin escritorio:

```bash
npm run build
cargo run -p factoria-server -- --static dist
# FACTORIA_READY http://127.0.0.1:<puerto>/?token=<token>
```

Abre esa URL. El token de sesión es obligatorio: evita que cualquier pestaña del
navegador use la IA local del equipo.

### Comprobaciones

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # 192 tests
npm run check                   # svelte-check
npm test                        # 26 tests
npm run build
npm run test:e2e                # 30 comprobaciones en un navegador real
```

### Variables de entorno

| Variable | Para qué |
|---|---|
| `FACTORIA_DATA_DIR` | Directorio de datos (pruebas, pilotos, perfiles portables) |
| `FACTORIA_POLICY_FILE` | Política corporativa a usar (pilotos y pruebas) |
| `FACTORIA_LLAMA_SERVER_BIN` | `llama-server` ya presente (despliegue aislado, pruebas) |
| `FACTORIA_LOG` | Nivel de traza, p. ej. `factoria_core=debug` |
| `OLLAMA_HOST` | Dónde escucha Ollama, si no es el puerto por defecto |

---

## Build

```bash
npm run tauri build
```

Genera el instalador de la plataforma anfitriona en `src-tauri/target/release/bundle/`.
No hay compilación cruzada: cada plataforma se construye en la suya.

Firma y notarización (certificados de Naturgy) son un paso posterior del *pipeline*
de publicación y todavía no están configurados.

---

## Modelos compatibles

Cualquier **GGUF de fichero único** que llama.cpp pueda cargar, y cualquier etiqueta
de Ollama si el empleado ya lo tiene instalado. El catálogo de serie
(`crates/factoria-core/src/catalog/data.rs`) cubre de 8 a 64 GB de RAM:

| Modelo | Tamaño | Descarga | Contexto | Licencia |
|---|---|---|---|---|
| Qwen2.5 14B Instruct | 14B | 8,78 GB | 32K | Apache-2.0 |
| Gemma 2 9B Instruct | 9B | 5,63 GB | 8K | Gemma Terms |
| Llama 3.1 8B Instruct | 8B | 4,80 GB | 128K | Llama 3.1 Community |
| Qwen2.5 7B Instruct | 7B | 4,57 GB | 32K | Apache-2.0 |
| Mistral 7B Instruct v0.3 | 7,2B | 4,27 GB | 32K | Apache-2.0 |
| Phi-3.5 Mini Instruct | 3,8B | 2,34 GB | 128K | MIT |
| Qwen2.5 3B Instruct | 3B | 1,88 GB | 32K | Qwen Research |
| Qwen2.5 1.5B Instruct | 1,5B | 1,09 GB | 32K | Apache-2.0 |
| Qwen2.5 0.5B Instruct | 0,5B | 398 MB | 32K | Apache-2.0 |

**La licencia MIT de la aplicación no cubre los pesos.** Cada modelo lleva la suya y
la interfaz la muestra antes de descargar. Ver [`NOTICE.md`](NOTICE.md).

### Cómo se decide qué es "Óptimo"

Criterios deterministas y documentados ([`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §6):

```
memoria_necesaria = pesos × copias + caché_KV(8K tokens) + 1 GiB de proceso
   copias = 2,0 si el acelerador copia los pesos (CUDA, Vulkan discreta)
          = 1,15 si mantiene el mmap (Metal, CPU, OpenCL)

presupuesto_RAM  = RAM_total × 0,65
presupuesto_VRAM = VRAM × 0,90
presupuesto_disco = disco_libre − 2 GiB
```

| Etiqueta | Cuándo |
|---|---|
| **Óptimo** | Cabe en RAM **y** la gráfica lo aloja entero (o el equipo es de memoria unificada) |
| **Compatible** | Cabe en RAM y en disco, pero se ejecutará en CPU o con descarga parcial |
| **No recomendado** | No cabe en memoria, no cabe en disco, o el motor que necesita no está disponible |

Cada veredicto llega con sus razones, y la interfaz las muestra literalmente.

---

## Seguridad

- **Los prompts, las respuestas y las conversaciones no salen del equipo.** No hay
  ninguna ruta del producto que los envíe a ningún sitio.
- **El indicador "Procesando localmente" es verificable**: se deriva del *endpoint*
  que realmente atiende la generación (`runtime::endpoint_is_loopback`), no de una
  constante. Si algún día hubiera un motor remoto, se apagaría solo.
- **Telemetría apagada por defecto.** Activarla es una decisión de Naturgy, y aun así
  solo enviaría contadores agregados, nunca texto.
- **Integridad de las descargas**: SHA-256 anclado en el catálogo o publicado por el
  origen. Sin ninguna de las dos, la instalación se rechaza salvo que la política lo
  autorice explícitamente.
- **Descompresión a prueba de *path traversal***; cabecera GGUF validada antes de
  activar un modelo.
- **El host HTTP escucha solo en `127.0.0.1`** y exige un token de sesión efímero.
- **Sin CDNs**: fuentes y recursos empaquetados. Funciona sin internet y detrás de un
  proxy con inspección TLS.
- **Auditoría local** *append-only* con lo que se ha hecho, **nunca con lo que se ha
  escrito** (hay un test que lo comprueba).

Reportar un problema de seguridad: a través del canal interno de Naturgy, no en un
issue público.

---

## Entorno corporativo

Un fichero `policy.json` colocado por GPO, Intune o Jamf manda sobre los ajustes del
usuario. Sin ningún servicio en la nube.

```
Windows : %PROGRAMDATA%\Naturgy\FactorIA\policy.json
macOS   : /Library/Application Support/Naturgy/FactorIA/policy.json
Linux   : /etc/naturgy/factoria/policy.json
```

```jsonc
{
  "organization": "Naturgy",
  "locked": ["network.proxy", "catalog.denylist"],   // no editables por el usuario
  "network": {
    "proxy": "http://proxy.naturgy.com:8080",
    "caBundle": "C:/ProgramData/Naturgy/ca-bundle.pem",
    "offline": false
  },
  "catalog": {
    "source": "C:/ProgramData/Naturgy/FactorIA/catalogo-autorizado.json",
    "mirrorBaseUrl": "https://artifacts.naturgy.com/ia",
    "allowlist": ["qwen2.5-*"],
    "denylist": ["*-uncensored"],
    "allowUnverifiedDownloads": false
  },
  "telemetry": { "enabled": false },
  "audit": { "enabled": true, "retentionMonths": 12 },
  "updates": { "channel": "off" },
  "chat": { "systemPrompt": "No compartas datos de clientes ni información confidencial." }
}
```

Lo que la política controla: proxy, certificados corporativos, modo aislado, catálogo
autorizado, *mirror* interno de pesos y motores, listas de permitidos/prohibidos,
canal de actualizaciones, telemetría, auditoría e instrucciones corporativas.

---

## Troubleshooting

| Síntoma | Causa probable | Qué hacer |
|---|---|---|
| **"El motor no llegó a estar listo a tiempo"** | Modelo grande en disco lento, o la gráfica no arranca el motor | Revisar `logs/` en el directorio de datos. Probar un modelo más pequeño |
| **"La integridad del fichero no se puede verificar"** | El origen no publica el SHA-256 y el catálogo no lo ancla | Anclar el digest en el catálogo corporativo, o autorizarlo con `allowUnverifiedDownloads` |
| **"El fichero descargado no coincide con el SHA-256"** | Descarga corrupta, o un proxy devolviendo una página de error | Reintentar. Si persiste, revisar si el proxy intercepta la descarga |
| **"El proxy debe incluir el esquema"** | `network.proxy` sin `http://` | Corregir la política |
| **La descarga falla tras un proxy corporativo** | Inspección TLS sin la CA configurada | Apuntar `network.caBundle` al *bundle* de Naturgy |
| **Todo sale "No recomendado"** | Equipo por debajo del mínimo, o disco lleno | La tarjeta dice el motivo exacto. Liberar disco, o usar un modelo más pequeño |
| **No se detecta la gráfica** | Sin `nvidia-smi` ni WMI accesible | No es un error: la clasificación sigue con RAM y CPU. Ajustes → Diagnóstico dice de qué fuente salió cada dato |
| **Ollama sale "No disponible"** | El demonio no está en marcha | Arrancarlo, o usar el motor propio de FactorIA (no requiere instalar nada) |
| **El chat dice que no hay modelo en marcha** | Ninguno arrancado | Catálogo → Ejecutar |

Directorio de datos (visible en Ajustes → Configuración corporativa):
`%APPDATA%\es.naturgy.factoria\` · `~/Library/Application Support/es.naturgy.factoria/`

---

## Limitaciones actuales

Esto es una **V0**. Lo que todavía no hay, dicho sin rodeos:

- **Sin instaladores publicados.** El *crate* del shell compila, pero los `.dmg` y NSIS
  no se han generado ni firmado.
- **Las URLs y los SHA-256 del catálogo de serie no están confirmados en vivo**: el
  entorno de desarrollo tenía bloqueado el acceso a Hugging Face y a GitHub Releases.
  El *pipeline* de publicación debe verificarlos antes de distribuir.
- **Sin inferencia real probada.** Todo el camino de FactorIA está verificado de
  extremo a extremo, pero contra un motor de prueba que habla el mismo protocolo. Ver
  [`docs/VALIDATION.md`](docs/VALIDATION.md) §3 y §4.
- **Sin RAG**: no hay documentos, ni búsqueda, ni citas. Deliberado (punto 12 del
  encargo: primero algo instalable y probable).
- **Sin *tool calling*, MCP, skills ni agentes.** Los tipos existen y el flujo los
  contempla, pero no se ejecutan.
- **Un modelo a la vez.** Correcto en un PC, pero es una limitación real.
- **Solo castellano** en la interfaz.
- **Sin actualizador in-app.** El canal existe en la política; el mecanismo, no.
- **El catálogo remoto (`catalog.source` con `https://`) no está implementado**: cae al
  embebido y lo registra en el log.
- **Sin SSO ni identidad**: la auditoría registra `local-user`.
- **Las rutas de GPU (CUDA/Metal/Vulkan/OpenCL) no se han probado sobre hardware**,
  solo su lógica y sus *parsers*.

---

## Roadmap

**Siguiente (cierre de la V1)**
1. Verificar y anclar URLs y SHA-256 del catálogo y de los motores en el *pipeline*.
2. Generar, firmar y notarizar los instaladores.
3. Probar con pesos reales sobre hardware con GPU (NVIDIA, Apple Silicon).
4. Piloto con un grupo reducido y política gestionada real.

**Después**
5. RAG local: documentos, embeddings y citas (el hueco ya está en `ChatRequest`).
6. *Tool calling* y MCP.
7. Catálogo centralizado con refresco periódico y actualizador in-app.
8. API local documentada para otras herramientas internas.
9. Runtime MLX en Apple Silicon.
10. SSO y auditoría con identidad corporativa.

---

## Documentación

| Documento | Contenido |
|---|---|
| [`docs/REBOST_ANALYSIS.md`](docs/REBOST_ANALYSIS.md) | Análisis de Rebost: arquitectura, qué se reutiliza, riesgos, licencia |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Arquitectura objetivo, criterios de compatibilidad, entorno corporativo |
| [`docs/MVP_PLAN.md`](docs/MVP_PLAN.md) | Plan del MVP con tareas verificables y su estado |
| [`docs/VALIDATION.md`](docs/VALIDATION.md) | Qué se ha verificado, cómo, y qué no |
| [`docs/BRANDING.md`](docs/BRANDING.md) | Identidad visual, tokens, tipografía, tono |
| [`NOTICE.md`](NOTICE.md) | Atribución a Rebost, terceros y licencias de los modelos |

---

## Licencia

MIT. Ver [`LICENSE`](LICENSE) y [`NOTICE.md`](NOTICE.md).

La licencia cubre el código, **no** los pesos de los modelos ni la marca Naturgy.
