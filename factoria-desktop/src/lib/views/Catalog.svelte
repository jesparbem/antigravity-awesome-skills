<script lang="ts">
  import { store } from "../state.svelte";
  import { FIT_LABELS } from "../format";
  import Icon from "../components/Icon.svelte";
  import ModelCardView from "../components/ModelCard.svelte";
  import type { FitLevel, ModelState } from "../types";

  interface Props {
    onNavigate: (view: "home" | "chat" | "settings") => void;
  }
  let { onNavigate }: Props = $props();

  let fitFilter = $state<FitLevel | "all">("all");
  let stateFilter = $state<ModelState | "all">("all");
  let query = $state("");

  const filtered = $derived(
    store.models.filter((card) => {
      if (fitFilter !== "all" && card.verdict.level !== fitFilter) return false;
      if (stateFilter !== "all" && card.state !== stateFilter) return false;
      if (query.trim()) {
        const needle = query.trim().toLowerCase();
        const hay = `${card.model.name} ${card.model.family} ${card.model.provider}`.toLowerCase();
        if (!hay.includes(needle)) return false;
      }
      return true;
    }),
  );

  const counts = $derived({
    optimal: store.models.filter((c) => c.verdict.level === "optimal").length,
    compatible: store.models.filter((c) => c.verdict.level === "compatible").length,
    notRecommended: store.models.filter((c) => c.verdict.level === "notRecommended").length,
  });
</script>

<div class="flex flex-col gap-5">
  <header class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-[22px] font-extrabold">Catálogo de modelos</h1>
      <p class="mt-1 text-[13.5px]" style="color:var(--texto-suave)">
        {store.models.length} modelos autorizados · {counts.optimal} óptimos, {counts.compatible} compatibles,
        {counts.notRecommended} no recomendados para este equipo.
      </p>
    </div>
    <button class="btn btn-ghost btn-sm" onclick={() => store.refreshModels()}>
      <Icon name="refresh" size={15} /> Actualizar
    </button>
  </header>

  <div class="card flex flex-wrap items-center gap-3 p-3.5" data-testid="catalog-filters">
    <label class="min-w-[180px] flex-1">
      <span class="sr-only">Buscar modelo</span>
      <input
        class="campo"
        type="search"
        placeholder="Buscar por nombre, familia o proveedor…"
        bind:value={query}
        data-testid="catalog-search"
      />
    </label>
    <div class="flex flex-wrap gap-1.5" role="group" aria-label="Filtrar por compatibilidad">
      <button
        class="chip"
        class:chip-optimal={fitFilter === "all"}
        onclick={() => (fitFilter = "all")}
        data-testid="filter-all"
      >
        Todos
      </button>
      {#each ["optimal", "compatible", "notRecommended"] as const as level}
        <button
          class="chip"
          class:chip-optimal={fitFilter === level}
          onclick={() => (fitFilter = fitFilter === level ? "all" : level)}
          data-testid="filter-{level}"
        >
          {FIT_LABELS[level]}
        </button>
      {/each}
    </div>
    <select class="campo w-auto" bind:value={stateFilter} aria-label="Filtrar por estado">
      <option value="all">Cualquier estado</option>
      <option value="notInstalled">No instalados</option>
      <option value="installed">Instalados</option>
      <option value="running">En ejecución</option>
    </select>
  </div>

  {#if filtered.length}
    <div class="grid gap-4 xl:grid-cols-2" data-testid="catalog-grid">
      {#each filtered as card (card.model.id)}
        <ModelCardView {card} onOpenChat={() => onNavigate("chat")} />
      {/each}
    </div>
  {:else}
    <p class="card p-6 text-center text-[13.5px]" style="color:var(--texto-suave)">
      Ningún modelo coincide con el filtro.
    </p>
  {/if}
</div>
