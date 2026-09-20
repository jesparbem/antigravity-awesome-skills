import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  // La interfaz se empaqueta entera: ni CDNs ni fuentes externas, para que
  // funcione sin internet y detrás de un proxy con inspección TLS.
  build: { target: "es2022", assetsInlineLimit: 4096, sourcemap: false },
  server: { port: 5273, strictPort: false },
});
