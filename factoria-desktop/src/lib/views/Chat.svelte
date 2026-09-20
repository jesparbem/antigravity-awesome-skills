<script lang="ts">
  import { api } from "../api";
  import { store } from "../state.svelte";
  import { formatDuration, formatDate } from "../format";
  import { renderMarkdown } from "../markdown";
  import Icon from "../components/Icon.svelte";
  import type { Message, Thread } from "../types";

  interface Props {
    onNavigate: (view: "home" | "catalog" | "settings") => void;
  }
  let { onNavigate }: Props = $props();

  let activeThread = $state<Thread | null>(null);
  let messages = $state<Message[]>([]);
  let draft = $state("");
  let sending = $state(false);
  let pendingId = $state<string | null>(null);
  let scroller = $state<HTMLDivElement | null>(null);

  const running = $derived(store.home?.running ?? null);
  const streamingText = $derived(pendingId ? (store.streaming[pendingId] ?? "") : "");
  const canSend = $derived(!!running && !!activeThread && draft.trim().length > 0 && !sending);
  const lastUserMessage = $derived([...messages].reverse().find((m) => m.role === "user"));

  // Al llegar a la vista, abre la última conversación o crea una.
  $effect(() => {
    if (activeThread) return;
    void (async () => {
      const threads = store.threads.length ? store.threads : await api.threads();
      if (threads.length) {
        await open(threads[0]);
      } else if (running) {
        await newThread();
      }
    })();
  });

  // Mantener la vista abajo mientras llega texto.
  $effect(() => {
    void streamingText;
    void messages.length;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

  async function open(thread: Thread) {
    activeThread = thread;
    messages = await api.messages(thread.id);
    pendingId = null;
  }

  async function newThread() {
    try {
      const thread = await api.createThread();
      store.threads = [thread, ...store.threads];
      await open(thread);
    } catch (err) {
      store.notify("error", err instanceof Error ? err.message : String(err));
    }
  }

  async function send(regenerate = false) {
    if (!activeThread || !running) return;
    const text = regenerate ? (lastUserMessage?.content ?? "") : draft.trim();
    if (!text) return;

    const thread = activeThread;
    sending = true;
    store.generating = true;
    if (!regenerate) {
      messages = [
        ...messages,
        {
          id: `local-${Date.now()}`,
          role: "user",
          content: text,
          createdAt: new Date().toISOString(),
          local: true,
        },
      ];
      draft = "";
    } else {
      // Regenerar descarta la respuesta anterior también en pantalla.
      messages = messages.filter((m, i) => !(m.role === "assistant" && i === messages.length - 1));
    }

    // El identificador real llega con la respuesta; hasta entonces se muestra
    // el texto del primer flujo que aparezca para esta conversación.
    const before = new Set(Object.keys(store.streaming));
    const watcher = setInterval(() => {
      const fresh = Object.keys(store.streaming).find((id) => !before.has(id));
      if (fresh) {
        pendingId = fresh;
        clearInterval(watcher);
      }
    }, 40);

    try {
      await api.send(thread.id, text, regenerate);
      clearInterval(watcher);
      if (pendingId) store.clearStream(pendingId);
      pendingId = null;
      messages = await api.messages(thread.id);
      store.threads = await api.threads();
    } catch (err) {
      clearInterval(watcher);
      pendingId = null;
      store.notify("error", err instanceof Error ? err.message : String(err));
    } finally {
      sending = false;
      store.generating = false;
    }
  }

  async function stop() {
    try {
      await api.stopGeneration();
    } catch (err) {
      store.notify("error", err instanceof Error ? err.message : String(err));
    }
  }

  async function clear() {
    if (!activeThread) return;
    await api.clearThread(activeThread.id);
    messages = [];
    store.notify("ok", "Conversación limpiada.");
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      store.notify("ok", "Copiado al portapapeles.");
    } catch {
      store.notify("error", "El navegador no permitió copiar.");
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (canSend) void send();
    }
  }
</script>

<div class="grid h-full gap-5 lg:grid-cols-[250px_1fr]">
  <!-- Conversaciones -->
  <aside class="card flex flex-col overflow-hidden" data-testid="thread-list">
    <div class="flex items-center justify-between border-b px-3.5 py-3" style="border-color:var(--borde)">
      <span class="etiqueta">Conversaciones</span>
      <button class="btn btn-ghost btn-sm" onclick={newThread} data-testid="btn-new-thread" aria-label="Nueva conversación">
        <Icon name="plus" size={15} />
      </button>
    </div>
    <div class="flex-1 overflow-y-auto p-2">
      {#each store.threads as thread (thread.id)}
        <button
          class="w-full rounded-lg px-3 py-2 text-left text-[13px] transition"
          style:background={activeThread?.id === thread.id
            ? "color-mix(in srgb, var(--color-azul) 10%, transparent)"
            : "transparent"}
          onclick={() => open(thread)}
        >
          <span class="block truncate font-semibold">{thread.title}</span>
          <span class="block text-[11.5px]" style="color:var(--texto-suave)">
            {thread.messageCount} mensajes
          </span>
        </button>
      {:else}
        <p class="px-3 py-2 text-[13px]" style="color:var(--texto-suave)">Aún no hay ninguna.</p>
      {/each}
    </div>
  </aside>

  <!-- Conversación -->
  <section class="card flex min-h-0 flex-col overflow-hidden" data-testid="chat-panel">
    <header class="flex flex-wrap items-center gap-3 border-b px-4 py-3" style="border-color:var(--borde)">
      <div class="min-w-0 flex-1">
        <h1 class="truncate text-[15px] font-bold">{activeThread?.title ?? "Chat local"}</h1>
        {#if running}
          <p class="text-[12.5px]" style="color:var(--texto-suave)">
            {running.modelId} · {running.runtime} · {running.contextTokens} tokens de contexto
          </p>
        {/if}
      </div>
      {#if running}
        <span class="chip chip-running" data-testid="local-badge" title={running.endpoint}>
          <Icon name="shield" size={13} /> Procesando localmente
        </span>
      {/if}
      <button class="btn btn-ghost btn-sm" onclick={clear} disabled={!activeThread} data-testid="btn-clear">
        <Icon name="trash" size={14} /> Limpiar
      </button>
    </header>

    {#if !running}
      <div class="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center" data-testid="chat-no-model">
        <Icon name="sparkles" size={26} />
        <h2 class="text-[17px] font-bold">No hay ningún modelo en marcha</h2>
        <p class="max-w-sm text-[13.5px]" style="color:var(--texto-suave)">
          Para conversar con la IA en este equipo, instala y arranca uno de los modelos del catálogo.
        </p>
        <button class="btn btn-primary" onclick={() => onNavigate("catalog")} data-testid="btn-go-catalog">
          <Icon name="catalog" size={15} /> Ir al catálogo
        </button>
      </div>
    {:else}
      <div class="flex-1 overflow-y-auto px-4 py-5" bind:this={scroller} data-testid="messages">
        {#if !messages.length && !streamingText}
          <div class="mx-auto mt-10 max-w-md text-center">
            <h2 class="text-[17px] font-bold">Pregunta lo que necesites</h2>
            <p class="mt-1.5 text-[13.5px]" style="color:var(--texto-suave)">
              La respuesta se genera en este equipo con {running.modelId}.
            </p>
          </div>
        {/if}

        <div class="mx-auto flex max-w-3xl flex-col gap-4">
          {#each messages as message (message.id)}
            <div
              class="flex flex-col gap-1"
              class:items-end={message.role === "user"}
              data-testid="message"
              data-role={message.role}
            >
              <div
                class="max-w-[85%] rounded-2xl px-4 py-2.5 text-[14px]"
                style:background={message.role === "user" ? "var(--color-azul)" : "var(--fondo)"}
                style:color={message.role === "user" ? "#fff" : "var(--texto)"}
              >
                {#if message.role === "assistant"}
                  <div class="md">{@html renderMarkdown(message.content)}</div>
                {:else}
                  <span class="whitespace-pre-wrap">{message.content}</span>
                {/if}
              </div>
              {#if message.role === "assistant"}
                <div class="flex items-center gap-2 text-[11.5px]" style="color:var(--texto-suave)">
                  {#if message.local}
                    <span class="chip chip-running" style="padding:1px 7px;font-size:10.5px">Local</span>
                  {/if}
                  {#if message.metrics}
                    <span data-testid="metrics">
                      {message.metrics.tokensPerSecond.toFixed(1)} tok/s · primer token en
                      {formatDuration(message.metrics.timeToFirstTokenMs)}
                    </span>
                  {/if}
                  <span>{formatDate(message.createdAt)}</span>
                  <button
                    class="btn btn-ghost btn-sm"
                    style="padding:2px 7px"
                    onclick={() => copy(message.content)}
                    data-testid="btn-copy"
                  >
                    <Icon name="copy" size={12} /> Copiar
                  </button>
                </div>
              {/if}
            </div>
          {/each}

          {#if streamingText}
            <div class="flex flex-col gap-1" data-testid="streaming-message">
              <div class="max-w-[85%] rounded-2xl px-4 py-2.5 text-[14px]" style="background:var(--fondo)">
                <div class="md">{@html renderMarkdown(streamingText)}</div>
              </div>
              <span class="latido text-[11.5px]" style="color:var(--texto-suave)">
                Generando en este equipo…
              </span>
            </div>
          {:else if sending}
            <span class="latido text-[12.5px]" style="color:var(--texto-suave)" data-testid="thinking">
              Procesando localmente…
            </span>
          {/if}
        </div>
      </div>

      <footer class="border-t px-4 py-3" style="border-color:var(--borde)">
        <div class="mx-auto flex max-w-3xl flex-col gap-2">
          <div class="flex items-end gap-2">
            <textarea
              class="campo max-h-40 min-h-[44px] resize-y"
              rows="1"
              placeholder="Escribe tu mensaje…"
              bind:value={draft}
              onkeydown={onKeydown}
              disabled={sending}
              data-testid="composer"
            ></textarea>
            {#if sending}
              <button class="btn btn-ghost" onclick={stop} data-testid="btn-stop-generation">
                <Icon name="stop" size={15} /> Detener
              </button>
            {:else}
              <button class="btn btn-primary" onclick={() => send()} disabled={!canSend} data-testid="btn-send">
                <Icon name="send" size={15} /> Enviar
              </button>
            {/if}
          </div>
          <div class="flex items-center justify-between text-[11.5px]" style="color:var(--texto-suave)">
            <span>Intro envía · Mayús+Intro salta de línea</span>
            <button
              class="btn btn-ghost btn-sm"
              style="padding:2px 8px"
              onclick={() => send(true)}
              disabled={sending || !lastUserMessage}
              data-testid="btn-regenerate"
            >
              <Icon name="refresh" size={12} /> Regenerar
            </button>
          </div>
        </div>
      </footer>
    {/if}
  </section>
</div>
