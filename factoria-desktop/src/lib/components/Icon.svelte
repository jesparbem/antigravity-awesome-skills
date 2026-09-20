<script lang="ts">
  /**
   * Iconos en SVG inline.
   *
   * Sin librería de iconos: son doce trazos, y así la interfaz no depende de
   * ningún paquete externo ni descarga nada en tiempo de ejecución.
   */
  interface Props {
    name: keyof typeof PATHS;
    size?: number;
    class?: string;
  }
  let { name, size = 18, class: cls = "" }: Props = $props();

  const PATHS = {
    home: "M3 10.5 12 3l9 7.5M5 9.5V21h14V9.5",
    catalog: "M4 5h16M4 12h16M4 19h10",
    chat: "M21 12a8 8 0 0 1-8 8H7l-4 3V12a8 8 0 0 1 8-8h2a8 8 0 0 1 8 8Z",
    settings:
      "M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7ZM19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-2.9 1.2V21a2 2 0 1 1-4 0v-.1A1.7 1.7 0 0 0 7 19.4a1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0-1.2-2.9H1a2 2 0 1 1 0-4h.1A1.7 1.7 0 0 0 2.6 7a1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 2.9-1.2V1a2 2 0 1 1 4 0v.1A1.7 1.7 0 0 0 17 2.6",
    cpu: "M9 3v2m6-2v2M9 19v2m6-2v2M3 9h2m-2 6h2m14-6h2m-2 6h2M7 7h10v10H7z",
    gpu: "M2 7h18a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H2zM6 11h4m4-1a2 2 0 1 0 0 4 2 2 0 0 0 0-4Z",
    disk: "M4 6h16v12H4zM7 10h.01M7 14h6",
    download: "M12 3v12m0 0 4-4m-4 4-4-4M4 19h16",
    play: "M7 4.5v15l12-7.5-12-7.5Z",
    stop: "M6 6h12v12H6z",
    trash: "M4 7h16M9 7V5h6v2m-8 0 1 13h8l1-13",
    copy: "M9 9h11v11H9zM5 15H4V4h11v1",
    refresh: "M3 12a9 9 0 0 1 15.3-6.4L21 8M21 4v4h-4M21 12a9 9 0 0 1-15.3 6.4L3 16M3 20v-4h4",
    plus: "M12 5v14M5 12h14",
    check: "m4 12 5 5L20 6",
    warning: "M12 4 2.5 20h19L12 4Zm0 6v4m0 3h.01",
    lock: "M6 11h12v9H6zM9 11V7a3 3 0 0 1 6 0v4",
    shield: "M12 3 4 6v6c0 5 3.4 8.3 8 9 4.6-.7 8-4 8-9V6l-8-3Z",
    send: "M4 12 20 4l-4 16-4-7-8-1Z",
    sparkles: "M12 3.5 13.7 9l5.5 1.7-5.5 1.8L12 18l-1.7-5.5L4.8 10.7 10.3 9 12 3.5ZM19 3v3m1.5-1.5h-3",
    arrowRight: "M5 12h14m-5-5 5 5-5 5",
    x: "M6 6l12 12M18 6 6 18",
  } as const;
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.7"
  stroke-linecap="round"
  stroke-linejoin="round"
  class={cls}
  aria-hidden="true"
>
  <path d={PATHS[name]} />
</svg>
