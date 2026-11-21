<script lang="ts">
  import { AppWindow, Database, ShieldCheck } from "@lucide/svelte";
  import { onMount } from "svelte";
  import type { HealthStatus } from "$lib/ipc";
  import { fetchHealth, type FetchResult } from "$lib/services/system";

  let health = $state<FetchResult<HealthStatus> | null>(null);
  let loading = $state(true);

  let appVersion = $derived(
    health !== null && health.ok ? health.value.appVersion : null,
  );
  let dbVersion = $derived(
    health !== null && health.ok ? health.value.dbUserVersion : null,
  );
  let healthError = $derived(health !== null && !health.ok ? health.error : null);

  onMount(() => {
    let cancelled = false;
    void fetchHealth().then((result) => {
      if (!cancelled) {
        health = result;
        loading = false;
      }
    });
    return () => {
      cancelled = true;
    };
  });
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Dashboard</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Foundation status for the new SyncUp desktop application.
    </p>
  </header>

  <div class="grid gap-4 md:grid-cols-3">
    <article class="rounded-xl border bg-card p-5 text-card-foreground">
      <div class="flex items-center gap-2 text-sm font-medium text-muted-foreground">
        <AppWindow class="size-4" aria-hidden="true" />
        Application
      </div>
      {#if loading}
        <p class="mt-4 text-sm text-muted-foreground">Checking typed IPC…</p>
      {:else if appVersion !== null}
        <p class="mt-4 text-2xl font-semibold">v{appVersion}</p>
        <p class="mt-1 text-sm text-muted-foreground">Typed IPC is connected.</p>
      {:else if healthError !== null}
        <p class="mt-4 text-sm text-destructive" role="alert">
          {healthError.code}: {healthError.message}
        </p>
      {:else}
        <p class="mt-4 text-sm text-muted-foreground">Health check unavailable.</p>
      {/if}
    </article>

    <article class="rounded-xl border bg-card p-5 text-card-foreground">
      <div class="flex items-center gap-2 text-sm font-medium text-muted-foreground">
        <Database class="size-4" aria-hidden="true" />
        Database
      </div>
      {#if loading}
        <p class="mt-4 text-sm text-muted-foreground">Opening connection pool…</p>
      {:else if dbVersion !== null}
        <p class="mt-4 text-2xl font-semibold">user_version {dbVersion}</p>
        <p class="mt-1 text-sm text-muted-foreground">No tables yet; schema arrives in Phase 3.</p>
      {:else if healthError !== null}
        <p class="mt-4 text-sm text-destructive" role="alert">
          {healthError.code}: {healthError.message}
        </p>
      {:else}
        <p class="mt-4 text-sm text-muted-foreground">Database status unavailable.</p>
      {/if}
    </article>

    <article class="rounded-xl border bg-card p-5 text-card-foreground">
      <div class="flex items-center gap-2 text-sm font-medium text-muted-foreground">
        <ShieldCheck class="size-4" aria-hidden="true" />
        Safety
      </div>
      <p class="mt-4 text-sm text-muted-foreground">
        No file operations are wired in this phase. Atomic publish, verification, and
        durability arrive in Phase 4.
      </p>
    </article>
  </div>

  <div class="rounded-xl border bg-card p-5 text-card-foreground">
    <h2 class="text-base font-semibold">Phase 2 foundation</h2>
    <ul class="mt-3 grid gap-2 text-sm text-muted-foreground sm:grid-cols-2">
      <li>Navigation shell and dark, light, and system themes</li>
      <li>Strict TypeScript with generated typed IPC bindings</li>
      <li>Rust logging and structured IPC errors</li>
      <li>SQLite connection pool with a version scaffold</li>
    </ul>
  </div>
</section>
