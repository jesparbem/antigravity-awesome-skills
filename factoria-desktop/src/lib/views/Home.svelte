<script lang="ts">
  import { api } from "../api";
  import { store } from "../state.svelte";
  import { ACCELERATOR_LABELS, formatBytes, formatPercent } from "../format";
  import Icon from "../components/Icon.svelte";
  import ModelCardView from "../components/ModelCard.svelte";
  import type { ResourceSample } from "../types";

  interface Props {
    onNavigate: (view: "catalog" | "chat" | "settings") => void;
  }
  let { onNavigate }: Props = $props();

  let resources = $state<ResourceSample | null>(null);

  const home = $derived(store.home);
  const ramPercent = $derived(
    resources && resources.ramTotalBytes > 0
      ? (resources.ramUsedBytes / resources.ramTotalBytes) * 100
      : 0,
  );

  // El panel de recursos se refresca solo mientras la Home está visible.
  $effect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const sample = await api.resources();
        if (alive) resources = sample;
      } catch {
        /* un fallo puntual de muestreo no merece un aviso */
      }
    };
    void tick();
    const timer = setInterval(tick, 5000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  });

  async function installAndRun(modelId: string) {
    try {
      store.notify("info", "Descargando el modelo. Puedes seguir usando la aplicación.");
      await api.installModel(modelId);
      await api.startModel(modelId);
      await store.refreshModels();
      onNavigate("chat");
    } catch (err) {
      store.notify("error", err instanceof Error ? err.message : String(err));
    }
  }
</script>

{#if home}
  <div class="flex flex-col gap-6">
    <!-- Cabecera -->
    <section
      class="relative overflow-hidden rounded-2xl px-7 py-8"
      style="background: linear-gradient(135deg, var(--color-azul) 0%, var(--color-azul-profundo) 100%)"
      data-testid="home-hero"
    >
      <div
        class="pointer-events-none absolute -right-16 -top-20 h-64 w-64 rounded-full opacity-20"
        style="background: radial-gradient(circle, var(--color-naranja) 0%, transparent 70%)"
      ></div>
      <p class="etiqueta" style="color: rgba(255,255,255,.65)">
        {home.organization} · {home.hardware.hostname || "este equipo"}
      </p>
      <h1 class="mt-2 text-[30px] font-extrabold text-white">Tu IA. En tu PC.</h1>
      <p class="mt-1.5 max-w-xl text-[15px]" style="color: rgba(255,255,255,.8)">
        {#if home.running}
          <strong class="text-white">{home.running.modelId}</strong> está en marcha en este equipo.
        {:else if home.installed.length}
          Tienes {home.installed.length}
          {home.installed.length === 1 ? "modelo instalado" : "modelos instalados"}. Arranca uno para
          empezar a conversar.
        {:else}
          Hemos analizado tu equipo y elegido los modelos que mejor funcionan en él.
        {/if}
      </p>
      <div class="mt-5 flex flex-wrap gap-2.5">
        {#if home.running}
          <button class="btn btn-accent" onclick={() => onNavigate("chat")} data-testid="quick-chat">
            <Icon name="chat" size={16} /> Abrir chat
          </button>
        {:else if home.installed.length}
          <button
            class="btn btn-accent"
            onclick={async () => {
              const id = home.installed[0].model.id;
              try {
                await api.startModel(id);
                await store.refreshModels();
                onNavigate("chat");
              } catch (err) {
                store.notify("error", err instanceof Error ? err.message : String(err));
              }
            }}
            data-testid="quick-run"
          >
            <Icon name="play" size={16} /> Ejecutar modelo
          </button>
        {:else if home.recommended.length}
          <button
            class="btn btn-accent"
            onclick={() => installAndRun(home.recommended[0].model.id)}
            data-testid="quick-install"
          >
            <Icon name="download" size={16} /> Instalar {home.recommended[0].model.name}
          </button>
        {/if}
        <button
          class="btn"
          style="background: rgba(255,255,255,.14); color: #fff"
          onclick={() => onNavigate("catalog")}
          data-testid="quick-catalog"
        >
          <Icon name="catalog" size={16} /> Ver catálogo
        </button>
      </div>
    </section>

    <div class="grid gap-5 lg:grid-cols-[1.15fr_1fr]">
      <!-- Mi PC -->
      <section class="card p-5" data-testid="panel-hardware">
        <div class="mb-3 flex items-center gap-2">
          <Icon name="cpu" size={17} />
          <h2 class="text-base font-bold">Mi PC</h2>
        </div>
        <dl class="grid grid-cols-2 gap-x-5 gap-y-3 text-[13px]">
          <div>
            <dt class="etiqueta">Sistema</dt>
            <dd class="font-semibold">{home.hardware.os} {home.hardware.osVersion}</dd>
          </div>
          <div>
            <dt class="etiqueta">Arquitectura</dt>
            <dd class="font-semibold">{home.hardware.arch}</dd>
          </div>
          <div class="col-span-2">
            <dt class="etiqueta">Procesador</dt>
            <dd class="font-semibold" data-testid="hw-cpu">
              {home.hardware.cpuBrand}
              <span style="color:var(--texto-suave)">
                · {home.hardware.physicalCores} núcleos / {home.hardware.logicalCores} hilos
              </span>
            </dd>
          </div>
          <div>
            <dt class="etiqueta">Memoria</dt>
            <dd class="font-semibold" data-testid="hw-ram">
              {formatBytes(home.hardware.totalRamBytes)}
            </dd>
          </div>
          <div>
            <dt class="etiqueta">Disco libre</dt>
            <dd class="font-semibold">{formatBytes(home.hardware.freeDiskBytes)}</dd>
          </div>
          <div class="col-span-2">
            <dt class="etiqueta">Gráfica</dt>
            <dd class="font-semibold" data-testid="hw-gpu">
              {#if home.hardware.gpus.length}
                {home.hardware.gpus[0].name}
                {#if home.hardware.usableVramBytes > 0}
                  <span style="color:var(--texto-suave)">
                    · {formatBytes(home.hardware.usableVramBytes)}
                    {home.hardware.unifiedMemory ? "unificada" : "VRAM"}
                  </span>
                {/if}
              {:else}
                Sin gráfica dedicada detectada
              {/if}
            </dd>
          </div>
          <div class="col-span-2">
            <dt class="etiqueta">Acelerador de inferencia</dt>
            <dd>
              <span class="chip" style="color:var(--color-azul);border-color:var(--color-azul)">
                {ACCELERATOR_LABELS[home.hardware.accelerator] ?? home.hardware.accelerator}
              </span>
            </dd>
          </div>
        </dl>
      </section>

      <!-- Recursos -->
      <section class="card p-5" data-testid="panel-resources">
        <div class="mb-3 flex items-center gap-2">
          <Icon name="gpu" size={17} />
          <h2 class="text-base font-bold">Recursos</h2>
        </div>
        {#if resources}
          <div class="flex flex-col gap-4">
            <div>
              <div class="mb-1 flex justify-between text-[13px]">
                <span class="etiqueta">CPU</span>
                <span class="font-semibold">{formatPercent(resources.cpuPercent)}</span>
              </div>
              <div class="barra"><span style:width="{Math.min(100, resources.cpuPercent)}%"></span></div>
            </div>
            <div>
              <div class="mb-1 flex justify-between text-[13px]">
                <span class="etiqueta">Memoria</span>
                <span class="font-semibold">
                  {formatBytes(resources.ramUsedBytes)} / {formatBytes(resources.ramTotalBytes)}
                </span>
              </div>
              <div class="barra"><span style:width="{ramPercent}%"></span></div>
            </div>
            <div class="flex items-center justify-between border-t pt-3 text-[13px]" style="border-color:var(--borde)">
              <span class="etiqueta">Disco libre</span>
              <span class="font-semibold">{formatBytes(resources.diskFreeBytes)}</span>
            </div>
            <div class="flex items-center justify-between text-[13px]">
              <span class="etiqueta">VRAM utilizable</span>
              <span class="font-semibold">
                {home.hardware.usableVramBytes > 0
                  ? formatBytes(home.hardware.usableVramBytes)
                  : "—"}
              </span>
            </div>
          </div>
        {:else}
          <p class="latido text-[13px]" style="color:var(--texto-suave)">Midiendo…</p>
        {/if}
      </section>
    </div>

    <!-- Modelos instalados -->
    {#if home.installed.length}
      <section data-testid="panel-installed">
        <h2 class="mb-3 text-base font-bold">Modelos instalados</h2>
        <div class="grid gap-4 xl:grid-cols-2">
          {#each home.installed as card (card.model.id)}
            <ModelCardView {card} compact onOpenChat={() => onNavigate("chat")} />
          {/each}
        </div>
      </section>
    {/if}

    <!-- Recomendados -->
    {#if home.recommended.length}
      <section data-testid="panel-recommended">
        <div class="mb-3 flex items-center justify-between">
          <h2 class="text-base font-bold">Recomendados para este equipo</h2>
          <button class="btn btn-ghost btn-sm" onclick={() => onNavigate("catalog")}>
            Ver todo <Icon name="arrowRight" size={14} />
          </button>
        </div>
        <div class="grid gap-4 xl:grid-cols-2">
          {#each home.recommended as card (card.model.id)}
            <ModelCardView {card} onOpenChat={() => onNavigate("chat")} />
          {/each}
        </div>
      </section>
    {:else if !home.installed.length}
      <section class="card p-5" data-testid="panel-no-fit">
        <div class="flex items-start gap-3">
          <Icon name="warning" size={18} />
          <div>
            <h2 class="text-base font-bold">Ningún modelo del catálogo encaja en este equipo</h2>
            <p class="mt-1 text-[13.5px]" style="color:var(--texto-suave)">
              El catálogo autorizado no incluye ningún modelo que este equipo pueda ejecutar con
              garantías. En el catálogo verás el motivo concreto de cada uno.
            </p>
          </div>
        </div>
      </section>
    {/if}
  </div>
{:else}
  <p class="latido" style="color:var(--texto-suave)">Analizando tu equipo…</p>
{/if}
