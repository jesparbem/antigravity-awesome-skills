<script lang="ts">
  import { api } from "../api";
  import { store } from "../state.svelte";
  import { formatDate } from "../format";
  import Icon from "../components/Icon.svelte";
  import type { AuditEntry, PolicyView, Settings } from "../types";

  let settings = $state<Settings | null>(null);
  let policy = $state<PolicyView | null>(null);
  let audit = $state<AuditEntry[]>([]);
  let saving = $state(false);

  const locked = (key: string) => policy?.locked.includes(key) || policy?.locked.includes("*");

  $effect(() => {
    void (async () => {
      try {
        const [s, p, a] = await Promise.all([api.settings(), api.policy(), api.audit(30)]);
        settings = s;
        policy = p;
        audit = a;
      } catch (err) {
        store.notify("error", err instanceof Error ? err.message : String(err));
      }
    })();
  });

  async function save() {
    if (!settings) return;
    saving = true;
    try {
      settings = await api.saveSettings(settings);
      store.notify("ok", "Ajustes guardados.");
    } catch (err) {
      store.notify("error", err instanceof Error ? err.message : String(err));
    } finally {
      saving = false;
    }
  }
</script>

<div class="flex max-w-3xl flex-col gap-5">
  <header>
    <h1 class="text-[22px] font-extrabold">Ajustes</h1>
    <p class="mt-1 text-[13.5px]" style="color:var(--texto-suave)">
      Lo que gestiona {policy?.organization ?? "Naturgy"} aparece bloqueado y no se puede editar aquí.
    </p>
  </header>

  <!-- Conversación -->
  <section class="card p-5" data-testid="settings-chat">
    <h2 class="mb-3 text-base font-bold">Conversación</h2>
    {#if settings}
      {#if policy?.chat?.systemPrompt}
        <div class="mb-4 rounded-lg p-3 text-[13px]" style="background:var(--color-azul-pale);color:var(--color-azul)">
          <p class="etiqueta mb-1" style="color:var(--color-azul)">
            <Icon name="lock" size={11} /> Instrucciones corporativas
          </p>
          <p>{policy.chat.systemPrompt}</p>
        </div>
      {/if}
      <label class="block">
        <span class="etiqueta">Mis instrucciones permanentes</span>
        <textarea
          class="campo mt-1.5"
          rows="3"
          placeholder="Por ejemplo: responde siempre en castellano y con viñetas."
          bind:value={settings.systemPrompt}
          disabled={locked("chat.systemPrompt")}
          data-testid="input-system-prompt"
        ></textarea>
      </label>
      <label class="mt-4 block">
        <span class="etiqueta">
          Creatividad de las respuestas ({settings.temperature.toFixed(1).replace(".", ",")})
        </span>
        <input
          class="mt-1.5 w-full"
          type="range"
          min="0"
          max="1"
          step="0.1"
          bind:value={settings.temperature}
        />
        <span class="text-[12px]" style="color:var(--texto-suave)">
          Más baja, respuestas más literales; más alta, más variadas.
        </span>
      </label>
      <label class="mt-4 flex items-center gap-2.5 text-[13.5px]">
        <input type="checkbox" bind:checked={settings.autostartLastModel} />
        Arrancar el último modelo usado al abrir FactorIA
      </label>
      <button class="btn btn-primary mt-4" onclick={save} disabled={saving} data-testid="btn-save-settings">
        <Icon name="check" size={15} /> Guardar
      </button>
    {/if}
  </section>

  <!-- Privacidad y red -->
  <section class="card p-5" data-testid="settings-network">
    <h2 class="mb-1 text-base font-bold">Privacidad y red</h2>
    <p class="mb-4 text-[13px]" style="color:var(--texto-suave)">
      Tus prompts, respuestas y conversaciones no salen de este equipo. La red se usa solo para
      descargar modelos y el motor de inferencia.
    </p>
    {#if policy}
      <dl class="grid grid-cols-[1fr_auto] gap-x-4 gap-y-2.5 text-[13px]">
        <dt>Telemetría</dt>
        <dd class="font-semibold" data-testid="telemetry-state">
          {policy.telemetry.enabled ? "Activada por la organización" : "Desactivada"}
        </dd>
        <dt>Modo aislado (sin red)</dt>
        <dd class="font-semibold">{policy.network.offline ? "Sí" : "No"}</dd>
        <dt>Proxy corporativo</dt>
        <dd class="font-semibold">{policy.network.proxy ?? "Del sistema"}</dd>
        <dt>Certificados corporativos</dt>
        <dd class="font-semibold">{policy.network.caBundle ?? "Del sistema"}</dd>
        <dt>Origen de los modelos</dt>
        <dd class="truncate font-semibold" title={policy.catalog.mirrorBaseUrl ?? ""}>
          {policy.catalog.mirrorBaseUrl ?? "Origen público del catálogo"}
        </dd>
        <dt>Actualizaciones</dt>
        <dd class="font-semibold">{policy.updates.channel}</dd>
        <dt>Registro de auditoría local</dt>
        <dd class="font-semibold">{policy.audit.enabled ? "Activado" : "Desactivado"}</dd>
      </dl>
    {/if}
  </section>

  <!-- Política -->
  <section class="card p-5" data-testid="settings-policy" data-loaded={policy ? "true" : "false"}>
    <h2 class="mb-3 flex items-center gap-2 text-base font-bold">
      Configuración corporativa
      {#if policy?.managed}
        <span class="chip" style="color:var(--color-azul);border-color:var(--color-azul)">
          <Icon name="lock" size={11} /> Gestionado por {policy.organization ?? "Naturgy"}
        </span>
      {/if}
    </h2>
    {#if policy}
      <dl class="grid grid-cols-[1fr_auto] gap-x-4 gap-y-2.5 text-[13px]">
        <dt>Origen de la política</dt>
        <dd class="truncate font-semibold" title={policy.origin}>{policy.origin}</dd>
        <dt>Catálogo</dt>
        <dd class="font-semibold">{policy.catalogSource}</dd>
        <dt>Modelos permitidos</dt>
        <dd class="font-semibold">
          {policy.catalog.allowlist.length ? policy.catalog.allowlist.join(", ") : "Todos"}
        </dd>
        <dt>Modelos prohibidos</dt>
        <dd class="font-semibold">
          {policy.catalog.denylist.length ? policy.catalog.denylist.join(", ") : "Ninguno"}
        </dd>
        <dt>Carpeta de datos</dt>
        <dd class="truncate font-semibold" title={policy.dataDir}>{policy.dataDir}</dd>
      </dl>

      <h3 class="mb-2 mt-5 text-[14px] font-bold">Motores de inferencia</h3>
      <div class="flex flex-col gap-2">
        {#each policy.runtimes as runtime (runtime.id)}
          <div class="flex items-start gap-3 rounded-lg p-3 text-[13px]" style="background:var(--fondo)">
            <span
              class="chip"
              class:chip-optimal={runtime.state === "ready"}
              class:chip-compatible={runtime.state === "needsSetup"}
            >
              {runtime.state === "ready"
                ? "Listo"
                : runtime.state === "needsSetup"
                  ? "Por preparar"
                  : "No disponible"}
            </span>
            <div>
              <p class="font-semibold">{runtime.name}</p>
              <p style="color:var(--texto-suave)">{runtime.detail ?? runtime.description}</p>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <!-- Auditoría -->
  <section class="card p-5" data-testid="settings-audit">
    <h2 class="mb-1 text-base font-bold">Registro de auditoría</h2>
    <p class="mb-3 text-[13px]" style="color:var(--texto-suave)">
      Qué se ha hecho en este equipo. Nunca incluye el texto de las conversaciones.
    </p>
    {#if audit.length}
      <div class="overflow-x-auto">
        <table class="w-full text-left text-[12.5px]">
          <thead>
            <tr style="color:var(--texto-suave)">
              <th class="py-1.5 pr-3 font-semibold">Fecha</th>
              <th class="py-1.5 pr-3 font-semibold">Evento</th>
              <th class="py-1.5 pr-3 font-semibold">Modelo</th>
              <th class="py-1.5 font-semibold">Resultado</th>
            </tr>
          </thead>
          <tbody>
            {#each audit as entry}
              <tr class="border-t" style="border-color:var(--borde)">
                <td class="py-1.5 pr-3">{formatDate(entry.ts)}</td>
                <td class="py-1.5 pr-3 font-semibold">{entry.event}</td>
                <td class="py-1.5 pr-3">{entry.modelId ?? "—"}</td>
                <td class="py-1.5">{entry.outcome}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <p class="text-[13px]" style="color:var(--texto-suave)">Todavía no hay actividad registrada.</p>
    {/if}
  </section>

  <p class="text-center text-[12px]" style="color:var(--texto-suave)">
    FactorIA Desktop {store.home?.version ?? ""} · Obra derivada de Rebost (MIT)
  </p>
</div>
