<script lang="ts">
  import { store } from "./lib/state.svelte";
  import Brand from "./lib/components/Brand.svelte";
  import Icon from "./lib/components/Icon.svelte";
  import Toasts from "./lib/components/Toasts.svelte";
  import Home from "./lib/views/Home.svelte";
  import Catalog from "./lib/views/Catalog.svelte";
  import Chat from "./lib/views/Chat.svelte";
  import Settings from "./lib/views/Settings.svelte";

  type View = "home" | "catalog" | "chat" | "settings";
  let view = $state<View>("home");

  const NAV = [
    { id: "home", label: "Inicio", icon: "home" },
    { id: "catalog", label: "Catálogo", icon: "catalog" },
    { id: "chat", label: "Chat", icon: "chat" },
    { id: "settings", label: "Ajustes", icon: "settings" },
  ] as const;

  $effect(() => {
    void store.init();
    return () => store.dispose();
  });

  function navigate(next: View) {
    view = next;
  }
</script>

<div class="flex h-screen overflow-hidden">
  <!-- Navegación -->
  <nav
    class="flex w-[228px] shrink-0 flex-col gap-1 border-r px-3 py-4"
    style="background:var(--superficie);border-color:var(--borde)"
    aria-label="Navegación principal"
  >
    <div class="mb-5 px-2">
      <Brand />
    </div>
    {#each NAV as item}
      <button
        class="flex items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-[13.5px] font-semibold transition"
        style:background={view === item.id
          ? "color-mix(in srgb, var(--color-azul) 10%, transparent)"
          : "transparent"}
        style:color={view === item.id ? "var(--color-azul)" : "var(--texto)"}
        aria-current={view === item.id ? "page" : undefined}
        onclick={() => navigate(item.id)}
        data-testid="nav-{item.id}"
      >
        <Icon name={item.icon} size={17} />
        {item.label}
        {#if item.id === "chat" && store.home?.running}
          <span class="ml-auto h-2 w-2 rounded-full" style="background:var(--color-verde)"></span>
        {/if}
      </button>
    {/each}

    <div class="mt-auto px-2">
      {#if store.home?.running}
        <div class="rounded-lg p-3 text-[12px]" style="background:var(--fondo)" data-testid="sidebar-running">
          <p class="etiqueta mb-1">En ejecución</p>
          <p class="truncate font-semibold">{store.home.running.modelId}</p>
          <p class="mt-0.5" style="color:var(--texto-suave)">{store.home.running.endpoint}</p>
        </div>
      {/if}
      <p class="mt-3 text-center text-[11px]" style="color:var(--texto-suave)">
        v{store.home?.version ?? "…"}
      </p>
    </div>
  </nav>

  <!-- Contenido -->
  <main class="flex-1 overflow-y-auto px-7 py-6" data-testid="main">
    {#if store.fatal}
      <div class="card mx-auto mt-16 max-w-md p-6 text-center" data-testid="fatal">
        <Icon name="warning" size={24} />
        <h1 class="mt-2 text-[17px] font-bold">No se pudo contactar con el núcleo</h1>
        <p class="mt-1.5 text-[13.5px]" style="color:var(--texto-suave)">{store.fatal}</p>
        <button class="btn btn-primary mt-4" onclick={() => store.init()}>
          <Icon name="refresh" size={15} /> Reintentar
        </button>
      </div>
    {:else if store.loading}
      <p class="latido" style="color:var(--texto-suave)">Cargando FactorIA…</p>
    {:else if view === "home"}
      <Home onNavigate={navigate} />
    {:else if view === "catalog"}
      <Catalog onNavigate={navigate} />
    {:else if view === "chat"}
      <div class="h-[calc(100vh-3rem)]"><Chat onNavigate={navigate} /></div>
    {:else}
      <Settings />
    {/if}
  </main>
</div>

<Toasts />
