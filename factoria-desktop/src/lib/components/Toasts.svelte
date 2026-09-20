<script lang="ts">
  import { store } from "../state.svelte";
  import Icon from "./Icon.svelte";
</script>

<div class="pointer-events-none fixed bottom-5 right-5 z-50 flex w-[min(380px,calc(100vw-2rem))] flex-col gap-2">
  {#each store.toasts as toast (toast.id)}
    <div
      class="card pointer-events-auto flex items-start gap-3 px-4 py-3 text-sm"
      role="status"
      data-testid="toast"
      style:border-left="3px solid {toast.kind === 'error'
        ? 'var(--color-rojo)'
        : toast.kind === 'ok'
          ? 'var(--color-verde)'
          : 'var(--color-azul)'}"
    >
      <span class="flex-1">{toast.text}</span>
      <button
        class="opacity-60 hover:opacity-100"
        onclick={() => store.dismiss(toast.id)}
        aria-label="Cerrar aviso"
      >
        <Icon name="x" size={15} />
      </button>
    </div>
  {/each}
</div>
