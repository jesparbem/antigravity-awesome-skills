# Avisos de terceros y atribución

FactorIA Desktop se distribuye bajo licencia **MIT** (ver `LICENSE`).

---

## Obra original: Rebost

FactorIA Desktop es una **obra derivada** de
[Rebost](https://github.com/Frontierz-AI/Rebost), *Private AI that works with your files*,
publicada bajo licencia MIT.

```
MIT License
Copyright (c) 2026 Frontierz
```

El aviso de copyright y el texto de la licencia de Rebost se conservan en `LICENSE`,
como exige la licencia MIT.

### Qué se ha tomado de Rebost

Lo que FactorIA hereda es **diseño y criterio**, no código copiado literalmente.
El detalle está en `docs/REBOST_ANALYSIS.md` §3. En resumen:

| De Rebost | En FactorIA |
|---|---|
| Arquitectura: capa IPC fina + módulos Rust desacoplados del shell | Adoptada y llevada más lejos: el núcleo es un *crate* propio (`factoria-core`) |
| Stack Tauri 2 + Svelte 5 + Tailwind 4 + Vite | Adoptado |
| Estrategia de *pin* del motor llama.cpp (release + build + URL + SHA-256 por SO/arquitectura) | Adoptada y generalizada en `runtime::llamacpp`, con *mirror* corporativo |
| Heurística de memoria `bytes × copias + margen` y el factor de RAM 0,65 | Adoptada como base y ampliada con VRAM, disco y KV cache (`catalog::fit`) |
| Criterios de arranque de `llama-server` según hardware (`engine/tune.rs`) | Reimplementados, reducidos a lo que usa el MVP (`runtime::tuning`) |
| Lectura de la cabecera GGUF (`engine/gguf.rs`) | Reimplementada, más pequeña (`gguf`) |
| Descarga con progreso y verificación SHA-256 | Rediseñada (`download`), añadiendo la resolución del digest contra el origen |
| Catálogo curado y fijado en código, nunca obtenido de la red | Adoptado, con origen sustituible por política corporativa |
| Persistencia de conversaciones: índice JSON + un JSONL por hilo | Adoptada |
| Detección de acelerador por presencia de bibliotecas del sistema | Adoptada y ampliada con enumeración real de GPU y VRAM |

**No** se ha reutilizado ningún activo de marca de Rebost (logotipo, iconos,
avatares) ni el pipeline de RAG documental (ingest, Tantivy, OCR, Privacy Lens).

---

## Dependencias en tiempo de ejecución

| Componente | Licencia | Cómo se distribuye |
|---|---|---|
| [llama.cpp](https://github.com/ggml-org/llama.cpp) (`llama-server`) | MIT | **No se empaqueta**: se descarga en el primer arranque desde la *release* fijada (o desde el *mirror* corporativo) y se verifica |
| [Tauri](https://tauri.app) 2 | MIT / Apache-2.0 | Enlazado |
| [Svelte](https://svelte.dev) 5 | MIT | Empaquetado en la interfaz |
| [Tailwind CSS](https://tailwindcss.com) 4 | MIT | Empaquetado en la interfaz |
| [axum](https://github.com/tokio-rs/axum), [tokio](https://tokio.rs), [reqwest](https://github.com/seanmonstar/reqwest), [sysinfo](https://github.com/GuillaumeGomez/sysinfo) | MIT / Apache-2.0 | Enlazados |
| [marked](https://marked.js.org) + [DOMPurify](https://github.com/cure53/DOMPurify) | MIT / (MPL-2.0 o Apache-2.0) | Empaquetados en la interfaz |

`cargo tree --format "{p} {l}"` y `npm ls --all` dan la lista completa y vigente.

---

## Pesos de los modelos

**La licencia MIT de esta aplicación no cubre los pesos de los modelos.** Cada modelo
lleva la suya, y algunas imponen restricciones de uso:

| Modelo del catálogo de serie | Licencia |
|---|---|
| Qwen2.5 (14B / 7B / 1.5B / 0.5B) | Apache-2.0 |
| Qwen2.5 3B | Qwen Research License (restricciones de uso comercial) |
| Llama 3.1 8B | Llama 3.1 Community License |
| Gemma 2 9B | Gemma Terms of Use |
| Mistral 7B v0.3 | Apache-2.0 |
| Phi-3.5 Mini | MIT |

La interfaz muestra la licencia de cada modelo **antes** de descargarlo. Un despliegue
corporativo debe revisar estas licencias antes de autorizar un modelo en el catálogo.

---

## Marca

`FactorIA Desktop` y la identidad visual de Naturgy son marcas de Naturgy Energy Group, S.A.
La licencia MIT cubre el código, **no** los derechos de marca.

Este repositorio **no incluye activos de marca protegidos**. El isotipo que dibuja
`src/lib/components/Brand.svelte` es una figura propia construida con los colores
corporativos, no una reproducción del imagotipo oficial. Ver `docs/BRANDING.md`.
