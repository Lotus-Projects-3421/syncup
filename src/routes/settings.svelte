<script lang="ts">
  import { Database, RefreshCw, ShieldCheck } from "@lucide/svelte";
  import { onMount } from "svelte";
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Card } from "$lib/components/ui/card";
  import type { AppSettings, MigrationReport, MigrationStatus_Serialize } from "$lib/ipc";
  import type { AppError } from "$lib/services/errors";
  import {
    fetchMigrationStatus,
    fetchSettings,
    runMigration,
    saveSettings,
  } from "$lib/services/settings";
  import { theme } from "$lib/state/theme.svelte";

  const inputClass =
    "mt-1 flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";

  const MIN_WIDTH = 900;
  const MAX_WIDTH = 7680;
  const MIN_HEIGHT = 600;
  const MAX_HEIGHT = 4320;

  let settings = $state<AppSettings | null>(null);
  let settingsLoading = $state(true);
  let settingsError = $state<AppError | null>(null);
  let widthInput = $state("");
  let heightInput = $state("");
  let saving = $state(false);
  let saveError = $state<AppError | null>(null);
  let saved = $state(false);

  let status = $state<MigrationStatus_Serialize | null>(null);
  let statusLoading = $state(true);
  let statusError = $state<AppError | null>(null);
  let report = $state<MigrationReport | null>(null);
  let migrating = $state(false);
  let migrationError = $state<AppError | null>(null);
  let sourceDbPath = $state("");
  let sourceJsonDir = $state("");

  function parseDimension(raw: string): number | null {
    const trimmed = raw.trim();
    if (!/^[0-9]+$/.test(trimmed)) {
      return null;
    }
    const parsed = Number(trimmed);
    return Number.isSafeInteger(parsed) ? parsed : null;
  }

  let widthValue = $derived(parseDimension(widthInput));
  let heightValue = $derived(parseDimension(heightInput));
  let widthValid = $derived(
    widthValue !== null && widthValue >= MIN_WIDTH && widthValue <= MAX_WIDTH,
  );
  let heightValid = $derived(
    heightValue !== null && heightValue >= MIN_HEIGHT && heightValue <= MAX_HEIGHT,
  );
  let canSave = $derived(settings !== null && widthValid && heightValid && !saving);
  let canMigrate = $derived(
    !migrating && (sourceDbPath.trim().length > 0 || sourceJsonDir.trim().length > 0),
  );

  async function loadSettings(): Promise<void> {
    settingsLoading = true;
    settingsError = null;
    const result = await fetchSettings();
    if (result.ok) {
      settings = result.value;
      widthInput = String(result.value.windowWidth);
      heightInput = String(result.value.windowHeight);
    } else {
      settingsError = result.error;
    }
    settingsLoading = false;
  }

  async function loadStatus(): Promise<void> {
    statusLoading = true;
    statusError = null;
    const result = await fetchMigrationStatus();
    if (result.ok) {
      status = result.value;
    } else {
      statusError = result.error;
    }
    statusLoading = false;
  }

  function markDirty(): void {
    saved = false;
    saveError = null;
  }

  async function handleSave(): Promise<void> {
    const base = settings;
    const width = widthValue;
    const height = heightValue;
    if (base === null || width === null || height === null || !canSave) {
      return;
    }
    saving = true;
    saveError = null;
    saved = false;
    const next: AppSettings = {
      ...base,
      theme: theme.mode,
      windowWidth: width,
      windowHeight: height,
    };
    const result = await saveSettings(next);
    if (result.ok) {
      settings = next;
      saved = true;
    } else {
      saveError = result.error;
    }
    saving = false;
  }

  async function handleMigrate(): Promise<void> {
    if (!canMigrate) {
      return;
    }
    migrating = true;
    migrationError = null;
    const db = sourceDbPath.trim();
    const dir = sourceJsonDir.trim();
    const result = await runMigration(db.length > 0 ? db : null, dir.length > 0 ? dir : null);
    if (result.ok) {
      report = result.value;
      await loadStatus();
    } else {
      migrationError = result.error;
    }
    migrating = false;
  }

  onMount(() => {
    void loadSettings();
    void loadStatus();
  });
</script>

<section class="mx-auto max-w-3xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Settings</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Application preferences and legacy data import.
    </p>
  </header>

  <Card>
    <h2 class="text-base font-semibold">Appearance</h2>
    <p class="mt-1 text-sm text-muted-foreground">
      Theme changes take effect immediately and are written to your stored settings.
    </p>
    <div class="mt-4">
      <ThemeToggle />
    </div>

    <div class="mt-6 border-t pt-4">
      <h3 class="text-sm font-medium">Default window size</h3>
      <p class="mt-1 text-sm text-muted-foreground">
        Saved for future launches. This does not resize the window you are using now.
      </p>

      {#if settingsLoading}
        <p class="mt-4 flex items-center gap-2 text-sm text-muted-foreground">
          <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
          Loading settings…
        </p>
      {:else if settingsError !== null}
        <p class="mt-4 text-sm text-destructive" role="alert">
          {settingsError.code}: {settingsError.message}
        </p>
      {:else if settings !== null}
        <div class="mt-4 grid gap-4 sm:grid-cols-2">
          <div>
            <label for="window-width" class="text-sm font-medium">Width (px)</label>
            <input
              id="window-width"
              type="text"
              inputmode="numeric"
              class={inputClass}
              bind:value={widthInput}
              oninput={markDirty}
              disabled={saving}
              aria-invalid={!widthValid}
              aria-describedby="window-width-range"
            />
            <p id="window-width-range" class="mt-1 text-xs text-muted-foreground">
              {MIN_WIDTH}–{MAX_WIDTH}
            </p>
            {#if !widthValid}
              <p class="mt-1 text-xs text-destructive" role="alert">
                Enter a whole number between {MIN_WIDTH} and {MAX_WIDTH}.
              </p>
            {/if}
          </div>
          <div>
            <label for="window-height" class="text-sm font-medium">Height (px)</label>
            <input
              id="window-height"
              type="text"
              inputmode="numeric"
              class={inputClass}
              bind:value={heightInput}
              oninput={markDirty}
              disabled={saving}
              aria-invalid={!heightValid}
              aria-describedby="window-height-range"
            />
            <p id="window-height-range" class="mt-1 text-xs text-muted-foreground">
              {MIN_HEIGHT}–{MAX_HEIGHT}
            </p>
            {#if !heightValid}
              <p class="mt-1 text-xs text-destructive" role="alert">
                Enter a whole number between {MIN_HEIGHT} and {MAX_HEIGHT}.
              </p>
            {/if}
          </div>
        </div>

        <div class="mt-4 flex flex-wrap items-center gap-3">
          <Button onclick={handleSave} disabled={!canSave}>
            {saving ? "Saving…" : "Save appearance"}
          </Button>
          {#if saved}
            <span class="flex items-center gap-1 text-sm text-muted-foreground" role="status">
              <ShieldCheck class="size-4" aria-hidden="true" />
              Saved to settings.
            </span>
          {/if}
        </div>
        {#if saveError !== null}
          <p class="mt-2 text-sm text-destructive" role="alert">
            {saveError.code}: {saveError.message}
          </p>
        {/if}
      {/if}
    </div>
  </Card>

  <Card>
    <div class="flex items-center gap-2">
      <Database class="size-4 text-muted-foreground" aria-hidden="true" />
      <h2 class="text-base font-semibold">Legacy data import</h2>
    </div>
    <p class="mt-1 text-sm text-muted-foreground">
      Import SyncUp v1 backup history, profiles and settings. Your original v1 files are
      never modified; a verified snapshot is taken first.
    </p>

    <div class="mt-4">
      {#if report !== null}
        <div class="space-y-3">
          <Badge class="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200">
            {report.alreadyMigrated ? "Already imported" : "Import complete"}
          </Badge>
          <p class="text-sm">{report.message}</p>
          <dl class="grid gap-2 text-sm sm:grid-cols-2">
            <div>
              <dt class="text-muted-foreground">Database rows imported</dt>
              <dd class="font-medium">{report.dbRowsImported}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Database rows skipped</dt>
              <dd class="font-medium">{report.dbRowsSkipped}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">JSON records imported</dt>
              <dd class="font-medium">{report.jsonRecordsImported}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Profiles imported</dt>
              <dd class="font-medium">{report.profilesImported}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Settings imported</dt>
              <dd class="font-medium">{report.settingsImported ? "Yes" : "No"}</dd>
            </div>
          </dl>
          {#if report.snapshotPaths.length > 0}
            <div>
              <p class="text-sm text-muted-foreground">Verified snapshots</p>
              <ul class="mt-1 space-y-1 text-xs break-all">
                {#each report.snapshotPaths as snapshotPath (snapshotPath)}
                  <li class="rounded border bg-muted/40 px-2 py-1">{snapshotPath}</li>
                {/each}
              </ul>
            </div>
          {/if}
        </div>
      {:else if statusLoading}
        <p class="flex items-center gap-2 text-sm text-muted-foreground">
          <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
          Checking import status…
        </p>
      {:else if statusError !== null}
        <p class="text-sm text-destructive" role="alert">
          {statusError.code}: {statusError.message}
        </p>
      {:else if status !== null && status.isMigrated}
        <div class="space-y-3">
          <Badge class="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200">
            Already imported
          </Badge>
          <dl class="grid gap-2 text-sm sm:grid-cols-2">
            <div>
              <dt class="text-muted-foreground">Processed records</dt>
              <dd class="font-medium">{status.processedCount}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Profiles</dt>
              <dd class="font-medium">{status.profilesCount}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Schema version</dt>
              <dd class="font-medium">{status.schemaVersion}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground">Settings present</dt>
              <dd class="font-medium">{status.hasSettings ? "Yes" : "No"}</dd>
            </div>
          </dl>
          {#if status.importedAt !== null}
            <p class="text-xs text-muted-foreground">Imported at {status.importedAt}</p>
          {/if}
        </div>
      {:else}
        <div class="space-y-4">
          <div>
            <label for="legacy-db" class="text-sm font-medium">
              Legacy database path (optional)
            </label>
            <input
              id="legacy-db"
              type="text"
              class={inputClass}
              placeholder="/absolute/path/to/state_device.db"
              bind:value={sourceDbPath}
              disabled={migrating}
            />
          </div>
          <div>
            <label for="legacy-json" class="text-sm font-medium">
              Legacy config folder (optional)
            </label>
            <input
              id="legacy-json"
              type="text"
              class={inputClass}
              placeholder="/absolute/path/to/v1/config/folder"
              bind:value={sourceJsonDir}
              disabled={migrating}
            />
          </div>
          <p class="text-xs text-muted-foreground">
            Provide at least one absolute path. If only the folder is given, any
            state_*.db files inside it are also imported.
          </p>
          <div class="flex flex-wrap items-center gap-3">
            <Button onclick={handleMigrate} disabled={!canMigrate}>
              {migrating ? "Importing…" : "Start import"}
            </Button>
            {#if migrating}
              <span class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
                <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
                Importing. This can take a while for large databases.
              </span>
            {/if}
          </div>
          {#if migrationError !== null}
            <p class="text-sm text-destructive" role="alert">
              {migrationError.code}: {migrationError.message}
            </p>
          {/if}
        </div>
      {/if}
    </div>
  </Card>

  <Card>
    <h2 class="text-base font-semibold">Application data</h2>
    <p class="mt-1 text-sm text-muted-foreground">
      SyncUp v2 keeps its database and settings in the platform application data
      directory. Legacy v1 files are left untouched.
    </p>
  </Card>
</section>
