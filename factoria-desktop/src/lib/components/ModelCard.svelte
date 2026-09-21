<script lang="ts">
  import { api } from "../api";
  import { store } from "../state.svelte";
  import {
    FIT_LABELS,
    formatBytes,
    formatContext,
    formatEta,
    formatSpeed,
    reasonText,
  } from "../format";
  import type { ModelCard } from "../types";
  import Icon from "./Icon.svelte";

  interface Props {
    card: ModelCard;
    compact?: boolean;
    onOpenChat?: (modelId: string) => void;
  }
  let { card, compact = false, onOpenChat }: Props = $props();

  let busy = $state(false);
  let expanded = $state(false);

  const download = $derived(store.downloads[card.model.id]);
  const progress = $derived(
    download?.totalBytes ? Math.min(100, (download.receivedBytes / download.totalBytes) * 100) : 0,
  );
  const blocked = $derived(card.verdict.level === "notRecommended");

  async function run(action: () => Promise<unknown>, okText: string) {
    busy = true;
    try {
      await action();
      store.notify("ok", okText);
      await store.refreshModels();
    } catch (err) {
      store.notify("error", err instanceof Error ? err.message : String(err));
    } finally {
      busy = false;
    }
  }

  const install = () =>
    run(() => api.installModel(card.model.id), `${card.model.name} instalado.`);
  const start = () => run(() => api.startModel(card.model.id), `${card.model.name} en marcha.`);
  const stop = () => run(() => api.stopModel(card.model.id), `${card.model.name} detenido.`);

  async function remove() {
    if (!confirm(`¿Eliminar ${card.model.name} de este equipo? Se borrarán los ficheros descargados.`))
      return;
    await run(() => api.removeModel(card.model.id), `${card.model.name} eliminado.`);
  }
</script>

<article
  class="card flex flex-col gap-3 p-4"
  data-testid="model-card"
  data-model-id={card.model.id}
  data-state={card.state}
  data-fit={card.verdict.level}
>
  <header class="flex items-start gap-3">
    <div class="min-w-0 flex-1">
      <div class="flex flex-wrap items-center gap-2">
        <h3 class="text-[15px] font-bold">{card.model.name}</h3>
        {#if card.recommended}
          <span class="chip" style="color:var(--color-naranja);border-color:var(--color-naranja);background:var(--color-naranja-pale)">
            <Icon name="sparkles" size={12} /> Recomendado
          </span>
        {/if}
        {#if card.state === "running"}
          <span class="chip chip-running" data-testid="chip-running">● En ejecución</span>
        {:else if card.state === "starting"}
          <span class="chip chip-compatible latido">Preparando…</span>
        {/if}
      </div>
      <p class="mt-0.5 text-[13px]" style="color:var(--texto-suave)">
        {card.model.provider} · {card.model.family} · {card.model.released}
      </p>
    </div>
    <span class="chip chip-{card.verdict.level}" data-testid="fit-chip">
      {FIT_LABELS[card.verdict.level]}
    </span>
  </header>

  {#if !compact}
    <p class="text-[13.5px]" style="color:var(--texto-suave)">{card.model.blurb}</p>
  {/if}

  <dl class="grid grid-cols-2 gap-x-4 gap-y-1.5 text-[12.5px] sm:grid-cols-4">
    <div>
      <dt class="etiqueta">Tamaño</dt>
      <dd class="font-semibold">{card.model.paramsB ? `${card.model.paramsB}B` : "MoE"}</dd>
    </div>
    <div>
      <dt class="etiqueta">Cuantización</dt>
      <dd class="font-semibold">{card.model.quantization}</dd>
    </div>
    <div>
      <dt class="etiqueta">Descarga</dt>
      <dd class="font-semibold">{formatBytes(card.model.fileBytes)}</dd>
    </div>
    <div>
      <dt class="etiqueta">Contexto</dt>
      <dd class="font-semibold">{formatContext(card.model.contextWindow)}</dd>
    </div>
    <div>
      <dt class="etiqueta">Memoria estimada</dt>
      <dd class="font-semibold" data-testid="memory-need">
        {formatBytes(card.verdict.memory.totalBytes)}
      </dd>
    </div>
    <div>
      <dt class="etiqueta">Velocidad estimada</dt>
      <dd class="font-semibold">
        ~{card.verdict.estimatedTokensPerSecond.toFixed(0)} tok/s
      </dd>
    </div>
    <div>
      <dt class="etiqueta">Motor</dt>
      <dd class="font-semibold">{card.model.runtimes.join(" · ")}</dd>
    </div>
    <div>
      <dt class="etiqueta">Licencia</dt>
      <dd class="truncate font-semibold" title={card.model.license}>{card.model.license}</dd>
    </div>
  </dl>

  {#if card.verdict.reasons.length}
    <ul class="flex flex-col gap-1 text-[12.5px]" style="color:var(--texto-suave)" data-testid="fit-reasons">
      {#each card.verdict.reasons as reason}
        <li class="flex items-start gap-1.5">
          <span aria-hidden="true" style="color:var(--color-naranja)">·</span>
          {reasonText(reason)}
        </li>
      {/each}
    </ul>
  {/if}

  {#if download}
    <div class="flex flex-col gap-1.5" data-testid="download-progress">
      <div class="barra acento"><span style:width="{progress}%"></span></div>
      <div class="flex justify-between text-[12px]" style="color:var(--texto-suave)">
        <span>
          {formatBytes(download.receivedBytes)}
          {#if download.totalBytes}/ {formatBytes(download.totalBytes)}{/if}
          · {formatSpeed(download.bytesPerSecond)}
        </span>
        <span>{formatEta(download.receivedBytes, download.totalBytes ?? 0, download.bytesPerSecond)}</span>
      </div>
    </div>
  {/if}

  {#if expanded}
    <div class="rounded-lg p-3 text-[12.5px]" style="background:var(--fondo)" data-testid="memory-breakdown">
      <p class="etiqueta mb-1.5">Cómo se calcula la memoria</p>
      <div class="grid grid-cols-2 gap-1">
        <span>Pesos ({card.verdict.memory.weightCopiesPct} %)</span>
        <span class="text-right font-semibold">{formatBytes(card.verdict.memory.weightsBytes)}</span>
        <span>Caché KV ({formatContext(card.verdict.memory.contextTokens)} tokens)</span>
        <span class="text-right font-semibold">{formatBytes(card.verdict.memory.kvCacheBytes)}</span>
        <span>Proceso del motor</span>
        <span class="text-right font-semibold">{formatBytes(card.verdict.memory.overheadBytes)}</span>
        <span class="border-t pt-1 font-bold" style="border-color:var(--borde)">Total</span>
        <span class="border-t pt-1 text-right font-bold" style="border-color:var(--borde)">
          {formatBytes(card.verdict.memory.totalBytes)}
        </span>
      </div>
    </div>
  {/if}

  <footer class="mt-auto flex flex-wrap items-center gap-2 pt-1">
    {#if card.state === "notInstalled"}
      <button
        class="btn btn-primary btn-sm"
        onclick={install}
        disabled={busy || blocked || !!download}
        data-testid="btn-install"
        title={blocked ? "Este equipo no puede ejecutar este modelo" : undefined}
      >
        <Icon name="download" size={15} /> Instalar
      </button>
    {:else if card.state === "installing"}
      <button class="btn btn-primary btn-sm" disabled>Instalando…</button>
    {:else if card.state === "starting"}
      <button class="btn btn-accent btn-sm latido" disabled data-testid="btn-starting">
        Preparando el motor…
      </button>
    {:else if card.state === "installed"}
      <button class="btn btn-accent btn-sm" onclick={start} disabled={busy} data-testid="btn-start">
        <Icon name="play" size={15} /> Ejecutar
      </button>
      <button class="btn btn-danger btn-sm" onclick={remove} disabled={busy} data-testid="btn-remove">
        <Icon name="trash" size={15} /> Eliminar
      </button>
    {:else if card.state === "running"}
      <button
        class="btn btn-primary btn-sm"
        onclick={() => onOpenChat?.(card.model.id)}
        data-testid="btn-open-chat"
      >
        <Icon name="chat" size={15} /> Abrir chat
      </button>
      <button class="btn btn-ghost btn-sm" onclick={stop} disabled={busy} data-testid="btn-stop">
        <Icon name="stop" size={15} /> Detener
      </button>
    {/if}
    <button
      class="btn btn-ghost btn-sm ml-auto"
      onclick={() => (expanded = !expanded)}
      aria-expanded={expanded}
      data-testid="btn-details"
    >
      {expanded ? "Ocultar detalle" : "Ver detalle"}
    </button>
  </footer>
</article>
