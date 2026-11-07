---
name: social-video-repo-finder
description: "Summarize saved YouTube/Instagram videos (favoritos), detect any code repository they mention and say exactly where the code lives; explainer videos get a structured summary."
category: content
risk: safe
source: community
date_added: "2026-11-07"
author: jesparbem
tags: [youtube, instagram, reels, summarization, github, repository, transcript, favoritos]
tools: [claude, claude-code, antigravity, cursor, gemini-cli, codex-cli]
---

# Social Video Repo Finder

## Overview

Esta skill convierte un favorito de **YouTube** o **Instagram** (un enlace suelto, una lista
de reproducción o la exportación completa de "Guardados") en un informe accionable:

1. **De qué habla el vídeo**: un resumen, que es detallado si se trata de un vídeo explicativo o tutorial.
2. **Si hay código**: qué repositorio es, dónde aparece (descripción, comentario fijado,
   enlace en la bio o mencionado de viva voz), si existe de verdad y **la carpeta exacta en la que hay que buscar**.
3. **Qué hacer después**: `git clone`, el archivo por el que conviene empezar y los enlaces que faltan.

El trabajo mecánico (metadatos, transcripción, extracción y verificación de enlaces) lo hace
`scripts/extract.py`; el agente solo lee el informe en JSON y redacta el resumen.

## When to Use This Skill

- El usuario pega un enlace de YouTube o Instagram y pregunta "¿de qué va esto?" o "¿dónde está el código?".
- El usuario tiene vídeos guardados ("me lo guardé para luego") y quiere un resumen de todos.
- Un reel dice "link en la bio" o "el repo en la descripción" y hay que encontrarlo.
- El usuario quiere decidir si merece la pena ver un tutorial largo.

## How It Works

### Step 1: Preparar el entorno (una sola vez)

```bash
pip install -r scripts/requirements.txt          # yt-dlp + youtube-transcript-api
pip install faster-whisper                       # opcional: reels sin subtítulos
export GITHUB_TOKEN=...                          # opcional: evita el límite de 60 peticiones/h
```

Instagram casi siempre exige una sesión iniciada. Hay que pasar las cookies del navegador con
`--cookies-from-browser chrome` (o `firefox`, `edge`, `safari`) o con `--cookies cookies.txt`.

### Step 2: Ejecutar el extractor

```bash
# Un solo vídeo
python3 scripts/extract.py "https://www.youtube.com/watch?v=VIDEO_ID" -o dossier.json

# Reel de Instagram (con cookies y transcripción local, porque los reels no suelen tener subtítulos)
python3 scripts/extract.py "https://www.instagram.com/reel/XXXX/" \
  --cookies-from-browser chrome --whisper -o dossier.json

# Todos los favoritos de una vez
python3 scripts/extract.py --input saved_posts.json --cookies-from-browser chrome --whisper -o dossier.json
python3 scripts/extract.py --input "Liked videos.csv" -o dossier.json            # Google Takeout
python3 scripts/extract.py "https://www.youtube.com/playlist?list=WL" \
  --cookies-from-browser chrome --limit 20 -o dossier.json                       # Ver más tarde

# Buscar enlaces también en el comentario fijado y en los comentarios más votados
python3 scripts/extract.py URL --with-comments -o dossier.json
```

Entradas admitidas: URL sueltas, `.txt` con una URL por línea, la exportación de Instagram
(`saved_posts.json` y `saved_collections.json`, que se obtienen en *Centro de cuentas → Descargar tu información*),
los CSV de listas de Google Takeout y `watch-history.json`. Los detalles están en
`references/exports.md`.

### Step 3: Leer el informe

Cada elemento de `items[]` contiene:

| Campo | Uso |
|---|---|
| `title`, `uploader`, `duration_s`, `chapters` | Contexto y estructura del vídeo |
| `description`, `transcript` (`transcript_source`) | Material para el resumen |
| `repositories[]` | Repositorios detectados: `url`, `path` (rama y carpeta si el enlace apunta dentro), `sources` (de dónde salió) y `github` (existe, estrellas, lenguaje, `top_level`, `readme_excerpt`, `clone`) |
| `code_links[]` | Otros sitios con código: Colab, Gist, Replit, CodeSandbox, Kaggle, PyPI, npm, etc. |
| `spoken_mentions[]` | Repositorios dictados en voz ("github punto com barra…") |
| `link_hints[]` | Frases como "link en la bio" que indican que falta un enlace |
| `signals` | Palabras clave de **tutorial** (`explainer`) y de **repositorio** (`repo`) |
| `github_search_suggestions[]` | Candidatos de GitHub encontrados por el título cuando no hay enlace |
| `ok` / `error` | Si falló, se explica el motivo y la solución (normalmente, faltan las cookies) |

### Step 4: Clasificar cada vídeo

- **Explicativo / tutorial**: `signals.explainer` no está vacío, hay capítulos, la duración
  supera los 3 minutos o la transcripción enseña algo paso a paso. En ese caso hay que hacer el **resumen detallado**.
- **Escaparate de proyecto**: muestra una herramienta o un repositorio sin explicarlo a fondo. Basta con un resumen corto y el foco en el código.
- **Otro** (humor, noticia u opinión): se resume en 2-3 frases.

### Step 5: Redactar la respuesta (en el idioma del usuario)

Para cada vídeo, se sigue esta plantilla:

```markdown
## 🎬 <título> — <autor> (<duración>, <plataforma>)
**Tipo:** Tutorial | Escaparate | Otro  ·  **Enlace:** <url>

### Resumen
<Tutorial: 5-10 viñetas con los conceptos clave en orden, más los pasos y los comandos que se mencionan.
 Otros: 2-3 frases.>

### 💻 Dónde está el código
| Repositorio | Encontrado en | Estado | Dónde mirar |
|---|---|---|---|
| owner/repo (`url`) | descripción / comentario fijado / bio / dicho en el vídeo | ✅ existe · ⭐ N · lenguaje | `carpeta/` (de `path` o deducida de `top_level` + README) |

`git clone https://github.com/owner/repo.git` → empieza por `<archivo o carpeta>`.

<Si no hay repositorio: dilo claramente. Si hay `link_hints`, indica dónde mirar (bio del perfil
 `uploader_url`, comentario fijado). Si hay `github_search_suggestions`, ofrécelas como
 "posibles candidatos (sin confirmar)".>

### ⏱️ Momentos clave (solo si hay capítulos o es un tutorial largo)
- 00:00 …
```

Con varios favoritos, se empieza con una **tabla índice** (título · tipo · ¿tiene código? · repositorio)
y después va el detalle de cada uno.

### Step 6: Decidir "dónde mirar" en el repositorio

1. Si `path` está relleno, esa es la carpeta exacta: el autor enlazó directamente a ella.
2. Si no, hay que cruzar lo que dice el vídeo con `top_level` y `readme_excerpt`: por ejemplo, si el
   vídeo trata de "RAG" y existe `examples/rag/`, se apunta ahí. En un monorepo de cursos, se busca la carpeta del
   capítulo o la lección que coincida con el título.
3. Si `exists: false`, se avisa de que el enlace está roto o el repositorio es privado y se ofrecen las sugerencias de búsqueda.

## Examples

### Example 1: Reel con "link en la bio"

```bash
python3 scripts/extract.py "https://www.instagram.com/reel/C9abc/" --cookies-from-browser chrome --whisper -o d.json
```
El JSON tiene `repositories: []` y `link_hints: ["link en la bio"]`, y `--with-comments` no aporta nada. El agente
abre `uploader_url`, ve que la bio apunta a `linktr.ee/...` y lo vuelve a ejecutar con ese enlace (el script expande
Linktree, Beacons y lnk.bio automáticamente cuando aparecen en el texto).

### Example 2: Tutorial largo de YouTube

`signals.explainer = ["paso a paso", "tutorial"]`, 14 capítulos y `repositories[0].path = {"ref":"main","path":"lesson-07"}`.
El agente hace el resumen detallado por capítulos y responde: *"El código de este vídeo está en
`owner/curso`, carpeta `lesson-07/`"*.

### Example 3: Exportación completa de guardados

```bash
python3 scripts/extract.py -i saved_posts.json --cookies-from-browser chrome --whisper --limit 30 -o d.json
```
El agente entrega una tabla índice de 30 filas y, después, el detalle solo de los vídeos con código o de tipo tutorial.

## Best Practices

- ✅ Hay que mencionar siempre de dónde salió cada repositorio (`sources`) para que el usuario pueda comprobarlo.
- ✅ Los repositorios confirmados por la API (`github.exists`) se distinguen de las sugerencias de búsqueda.
- ✅ El resumen se basa en `transcript`; si no hay transcripción, se avisa de que procede solo de la descripción.
- ✅ Con listas grandes, se usa `--limit` y se procesa por lotes.
- ❌ No hay que inventar URL de repositorios ni carpetas que no aparezcan en `top_level` o en el README.
- ❌ No hay que descargar vídeos completos; el script solo baja el audio cuando se usa `--whisper`.

## Common Pitfalls

- **Problema:** Instagram devuelve `login required` o el elemento sale vacío.
  **Solución:** Hay que usar `--cookies-from-browser <navegador>` con una sesión iniciada en ese navegador.
- **Problema:** Un reel no tiene transcripción.
  **Solución:** Hay que añadir `--whisper` (requiere `faster-whisper` y `ffmpeg`).
- **Problema:** Aparece `github.error: 403`.
  **Solución:** Se ha superado el límite de peticiones; hay que exportar `GITHUB_TOKEN`.
- **Problema:** La lista "Me gusta" (`list=LL`) o "Ver más tarde" (`list=WL`) sale vacía.
  **Solución:** Son listas privadas, así que necesitan cookies. Otra opción es usar el CSV de Google Takeout.

## Related Skills

- `@youtube-summarizer` - Resúmenes muy extensos de un único vídeo de YouTube.
- `@instagram` - Publicación y analítica mediante Graph API (no sirve para leer los guardados).
- `@address-github-comments` - Para trabajar después con un repositorio de GitHub.
