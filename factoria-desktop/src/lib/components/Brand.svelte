<script lang="ts">
  /**
   * Marca de FactorIA Desktop.
   *
   * El isotipo es una figura propia construida con los colores corporativos de
   * Naturgy (azul #004165, naranja #E57200). **No** reproduce el imagotipo
   * oficial: los activos de marca protegidos no se incorporan al repositorio.
   * Para usar el logotipo oficial, colócalo en `src/assets/brand/` y
   * sustitúyelo aquí — ver `docs/BRANDING.md`.
   */
  interface Props {
    size?: number;
    withWordmark?: boolean;
    onDark?: boolean;
  }
  let { size = 32, withWordmark = true, onDark = false }: Props = $props();
</script>

<div class="flex items-center gap-2.5" data-testid="brand">
  <svg width={size} height={size} viewBox="0 0 48 48" aria-hidden="true" class="shrink-0">
    <rect width="48" height="48" rx="12" fill="var(--color-azul)" />
    <path
      d="M24 10c-6 4.5-9 9-9 13.5 0 3.6 2.4 6.3 5.6 6.9-1.4 2.6-3.4 4.6-5.6 6.1h7.3c4.9 0 9.7-3.9 9.7-10.1 0-5.6-2.7-11.2-8-16.4Z"
      fill="var(--color-naranja)"
    />
    <path d="M17 33.5h9" stroke="#fff" stroke-width="2.4" stroke-linecap="round" />
  </svg>
  {#if withWordmark}
    <div class="leading-none">
      <div
        class="text-[17px] font-bold tracking-tight"
        style:color={onDark ? "#fff" : "var(--color-azul)"}
      >
        Factor<span style="color: var(--color-naranja)">IA</span>
      </div>
      <div
        class="mt-0.5 text-[10px] font-semibold uppercase tracking-[0.16em]"
        style:color={onDark ? "rgba(255,255,255,.66)" : "var(--texto-suave)"}
      >
        Desktop
      </div>
    </div>
  {/if}
</div>
