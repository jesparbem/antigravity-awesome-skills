# Identidad visual

## Criterio

FactorIA Desktop usa los **colores y el tono** de la identidad corporativa de Naturgy,
pero **no incorpora al repositorio ningún activo de marca protegido**: ni el imagotipo
oficial, ni la tipografía FS Emeric, ni fotografía corporativa.

El motivo es práctico además de legal: el repositorio puede acabar en manos de
terceros (proveedores, auditores, un fork), y los activos de marca no deben viajar
con el código.

## Tokens (en `src/app.css`)

| Token | Valor | Uso |
|---|---|---|
| `--color-azul` | `#004165` | Color principal: navegación, cabeceras, botones primarios |
| `--color-azul-oscuro` | `#004571` | Estados *hover* del azul |
| `--color-azul-profundo` | `#002c45` | Degradado de la cabecera de la Home |
| `--color-naranja` | `#E57200` | Acento: recomendación, llamadas a la acción, isotipo |
| `--color-naranja-claro` | `#FBAE40` | Acento secundario |
| `--color-verde` | `#0F8A5F` | Estado correcto (Óptimo, En ejecución) |
| `--color-ambar` | `#B26A00` | Advertencia (Compatible) |
| `--color-rojo` | `#C8102E` | Error, acción destructiva |

Modo oscuro incluido: los tokens se redefinen bajo
`@media (prefers-color-scheme: dark)` y bajo `:root[data-theme="dark"]`.

## Tipografía

La tipografía oficial (**FS Emeric**) es propietaria y no se distribuye aquí. La
aplicación usa la pila del sistema (Segoe UI Variable / SF / Helvetica). Esto no es
solo una limitación: una fuente del sistema se dibuja igual **sin internet y detrás de
un proxy con inspección TLS**, que es el escenario del PC corporativo. No se carga
ninguna fuente desde una CDN.

Para usar FS Emeric en un despliegue interno con licencia:

1. Coloca los ficheros en `src/assets/fonts/`.
2. Declara las `@font-face` al principio de `src/app.css`.
3. Cambia `--font-display` y `--font-body`.

Nada más cambia: toda la interfaz usa esos dos tokens.

## Logotipo

`src/lib/components/Brand.svelte` dibuja un isotipo **propio**: un cuadrado azul
redondeado con una figura naranja. No reproduce el imagotipo de Naturgy.

Para usar el logotipo oficial en un despliegue interno:

1. Coloca los ficheros en `src/assets/brand/`.
2. Sustituye el `<svg>` de `Brand.svelte` por un `<img>` a ese activo.
3. Regenera los iconos del instalador en `src-tauri/icons/`.

## Tono

- Castellano corporativo, tuteo, frases cortas.
- Sin jerga técnica en la interfaz: no se dice "offload", "VRAM insuficiente" ni
  "quantización"; se dice "la gráfica alojará solo una parte del modelo".
- Sin reclamos de marketing repetidos. El concepto **"Tu IA. En tu PC."** aparece
  una vez, en la Home.
- **Ninguna afirmación que la arquitectura no pueda garantizar.** El indicador
  "Procesando localmente" se deriva del *endpoint* real que atiende la generación
  (`runtime::endpoint_is_loopback`), no de una constante.

## Capturas

Las de `docs/screenshots/` se generaron con `npm run test:e2e` sobre la aplicación
real; se regeneran en cada ejecución en `screenshots/` (no versionado).

| Captura | Pantalla |
|---|---|
| `01-home.png` | Inicio: Mi PC, recursos en vivo, recomendados y acciones rápidas |
| `02-catalogo.png` | Catálogo con filtros, aptitud razonada y desglose de memoria |
| `03-instalado.png` | Modelo instalado, listo para ejecutar |
| `04-chat-streaming.png` | Respuesta llegando token a token |
| `05-chat-respuesta.png` | Respuesta completa con métricas medidas |
| `06-ajustes.png` | Red, política corporativa, motores y auditoría |
| `07-catalogo-final.png` | Catálogo tras detener y eliminar |
