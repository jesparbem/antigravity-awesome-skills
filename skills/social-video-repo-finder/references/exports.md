# Cómo exportar tus favoritos

## Instagram: "Guardados"
1. Instagram → Configuración → **Centro de cuentas** → *Tu información y permisos* → **Descargar tu información**.
2. Elige *Algunos de tus datos* → marca **Guardado** → formato **JSON** → crea los archivos.
3. En el ZIP que recibas, busca `your_instagram_activity/saved/saved_posts.json`
   (y `saved_collections.json` si usas colecciones).
4. `python3 scripts/extract.py -i saved_posts.json --cookies-from-browser chrome --whisper -o dossier.json`

El script recorre el JSON y extrae cualquier `href` que apunte a instagram.com, sin depender del esquema exacto, que Meta cambia a menudo.

## YouTube
- **Directo con cookies**: `https://www.youtube.com/playlist?list=LL` (Me gusta) o `list=WL` (Ver más tarde),
  con `--cookies-from-browser chrome`.
- **Google Takeout** (takeout.google.com → YouTube → *listas de reproducción* / *historial*):
  - `playlists/Liked videos.csv`, `playlists/Watch later.csv`: la primera columna es el *Video ID*.
  - `history/watch-history.json`: se usa el campo `titleUrl`.
- **Lista pública propia**: basta con la URL de la lista.

## Archivo de texto
Una URL por línea (YouTube, Shorts, Instagram reel/p/tv); puedes mezclar plataformas.
