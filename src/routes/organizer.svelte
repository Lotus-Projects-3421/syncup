<script lang="ts">
  import { FolderCog, RefreshCw, ShieldCheck, X } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Card } from "$lib/components/ui/card";
  import { Progress } from "$lib/components/ui/progress";
  import type {
    ConflictMode,
    Job_Serialize,
    OrganizerConfig_Deserialize,
    SortMode,
    SourceType,
  } from "$lib/ipc";
  import type { AppError } from "$lib/services/errors";
  import { cancelJob, fetchJob, fetchJobs } from "$lib/services/jobs";
  import { startOrganizerRun } from "$lib/services/organizer";

  const fieldClass =
    "mt-1 flex w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";
  const selectClass =
    "mt-1 flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";

  let inputsText = $state("");
  let destDir = $state("");
  let sourceType = $state<SourceType>("takeout");
  let sortMode = $state<SortMode>("year");
  let conflictMode = $state<ConflictMode>("keepBoth");
  let overwriteConfirmed = $state(false);

  let starting = $state(false);
  let startError = $state<AppError | null>(null);

  let jobId = $state<string | null>(null);
  let job = $state<Job_Serialize | null>(null);
  let jobError = $state<AppError | null>(null);
  let cancelling = $state(false);
  let cancelError = $state<AppError | null>(null);
  let attaching = $state(true);

  let pollTimer: number | undefined;
  let pollInFlight = false;
  let destroyed = false;

  /** Linux-only: an absolute path must start with a forward slash. */
  function isAbsolutePath(value: string): boolean {
    return value.startsWith("/");
  }

  function parseInputLines(raw: string): string[] {
    const seen = new Set<string>();
    const result: string[] = [];
    for (const line of raw.split("\n")) {
      const trimmed = line.trim();
      if (trimmed.length === 0 || seen.has(trimmed)) {
        continue;
      }
      seen.add(trimmed);
      result.push(trimmed);
    }
    return result;
  }

  function toSourceType(value: string): SourceType {
    return value === "phoneFolder" ? "phoneFolder" : "takeout";
  }

  function toSortMode(value: string): SortMode {
    return value === "month" || value === "day" || value === "flat" ? value : "year";
  }

  function toConflictMode(value: string): ConflictMode {
    return value === "overwrite" || value === "skip" ? value : "keepBoth";
  }

  type RunSummary = { processed: number; skipped: number; errors: number };

  /**
   * The organizer command reports "N organized, M skipped, K errors" both as the
   * final progress message and, when errors exist, as lastError with a Failed
   * status. Parsing it lets the UI show partial success honestly. If the format
   * ever changes this returns null and the raw text is shown instead.
   */
  function parseSummary(text: string | null): RunSummary | null {
    if (text === null) {
      return null;
    }
    const match = /^(\d+) organized, (\d+) skipped, (\d+) errors$/.exec(text.trim());
    if (match === null) {
      return null;
    }
    return {
      processed: Number(match[1]),
      skipped: Number(match[2]),
      errors: Number(match[3]),
    };
  }

  function progressPercent(current: Job_Serialize | null): number {
    if (current === null) {
      return 0;
    }
    const total = current.progress.total;
    if (total === null || total <= 0) {
      return 0;
    }
    return Math.min(100, Math.max(0, Math.round((current.progress.current / total) * 100)));
  }

  let inputPaths = $derived(parseInputLines(inputsText));
  let invalidInputs = $derived(inputPaths.filter((path) => !isAbsolutePath(path)));
  let destTrimmed = $derived(destDir.trim());
  let destValid = $derived(isAbsolutePath(destTrimmed));
  let canStart = $derived(
    !starting &&
      inputPaths.length > 0 &&
      invalidInputs.length === 0 &&
      destValid &&
      (conflictMode !== "overwrite" || overwriteConfirmed),
  );

  let percent = $derived(progressPercent(job));
  let hasTotal = $derived(job !== null && job.progress.total !== null && job.progress.total > 0);
  let summary = $derived(
    job === null ? null : (parseSummary(job.progress.message) ?? parseSummary(job.lastError)),
  );
  let isActive = $derived(
    job !== null && (job.status === "pending" || job.status === "running"),
  );
  let isCompleted = $derived(job !== null && job.status === "completed");
  let isPartial = $derived(
    job !== null && job.status === "failed" && summary !== null && summary.errors > 0,
  );
  let isCancelled = $derived(job !== null && job.status === "cancelled");
  let isHardFailed = $derived(job !== null && job.status === "failed" && !isPartial);
  let isTerminal = $derived(isCompleted || isPartial || isCancelled || isHardFailed);

  function stopPolling(): void {
    if (pollTimer !== undefined) {
      window.clearInterval(pollTimer);
      pollTimer = undefined;
    }
  }

  function startPolling(): void {
    stopPolling();
    if (destroyed) {
      return;
    }
    pollTimer = window.setInterval(() => {
      void pollJob();
    }, 1000);
  }

  async function pollJob(): Promise<void> {
    const id = jobId;
    if (id === null || pollInFlight) {
      return;
    }
    pollInFlight = true;
    const result = await fetchJob(id);
    pollInFlight = false;
    if (destroyed || jobId !== id) {
      return;
    }
    if (result.ok) {
      job = result.value;
      jobError = null;
      if (result.value.status !== "pending" && result.value.status !== "running") {
        stopPolling();
      }
    } else {
      jobError = result.error;
      stopPolling();
    }
  }

  async function attachToActiveJob(): Promise<void> {
    attaching = true;
    const result = await fetchJobs();
    if (result.ok) {
      const active = result.value
        .filter(
          (item) =>
            item.kind === "organizer" &&
            (item.status === "pending" || item.status === "running"),
        )
        .sort((a, b) => b.startedAt - a.startedAt)[0];
      if (active !== undefined && !destroyed) {
        jobId = active.id;
        job = active;
        startPolling();
      }
    }
    attaching = false;
  }

  async function handleStart(): Promise<void> {
    if (!canStart) {
      return;
    }
    starting = true;
    startError = null;
    cancelError = null;
    jobError = null;
    const config: OrganizerConfig_Deserialize = {
      inputs: inputPaths,
      destDir: destTrimmed,
      conflictMode,
      sourceType,
      sortMode,
    };
    const result = await startOrganizerRun(config);
    if (result.ok) {
      jobId = result.value;
      job = null;
      const first = await fetchJob(result.value);
      if (!destroyed && first.ok) {
        job = first.value;
      }
      startPolling();
    } else {
      startError = result.error;
    }
    starting = false;
  }

  async function handleCancel(): Promise<void> {
    const id = jobId;
    if (id === null || cancelling) {
      return;
    }
    cancelling = true;
    cancelError = null;
    const result = await cancelJob(id);
    if (result.ok) {
      await pollJob();
    } else {
      cancelError = result.error;
    }
    cancelling = false;
  }

  function retryPolling(): void {
    jobError = null;
    startPolling();
    void pollJob();
  }

  function resetRun(): void {
    stopPolling();
    jobId = null;
    job = null;
    jobError = null;
    startError = null;
    cancelError = null;
    overwriteConfirmed = false;
  }

  onMount(() => {
    void attachToActiveJob();
    return () => {
      destroyed = true;
      stopPolling();
    };
  });
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Organizer</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Organize Google Takeout archives and loose photo folders by date.
    </p>
  </header>

  {#if attaching}
    <Card>
      <p class="flex items-center gap-2 text-sm text-muted-foreground">
        <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
        Checking for a running organizer job…
      </p>
    </Card>
  {:else if jobId === null}
    <Card>
      <div class="flex items-center gap-2">
        <FolderCog class="size-4 text-muted-foreground" aria-hidden="true" />
        <h2 class="text-base font-semibold">Run setup</h2>
      </div>

      <div class="mt-4 space-y-4">
        <div>
          <label for="organizer-inputs" class="text-sm font-medium">
            Source folders or ZIP files
          </label>
          <textarea
            id="organizer-inputs"
            rows="4"
            class={fieldClass}
            placeholder={"/home/you/Takeout\n/home/you/Photos/DCIM"}
            bind:value={inputsText}
            disabled={starting}
            aria-describedby="organizer-inputs-help"
            aria-invalid={invalidInputs.length > 0}
          ></textarea>
          <p id="organizer-inputs-help" class="mt-1 text-xs text-muted-foreground">
            One absolute Linux path per line. Each entry may be a folder or a .zip archive.
          </p>
          {#if invalidInputs.length > 0}
            <div class="mt-1 text-xs text-destructive" role="alert">
              <p>These entries are not absolute Linux paths (they must start with /):</p>
              <ul class="mt-1 space-y-0.5 break-all">
                {#each invalidInputs as bad (bad)}
                  <li>{bad}</li>
                {/each}
              </ul>
            </div>
          {/if}
        </div>

        <div>
          <label for="organizer-dest" class="text-sm font-medium">Destination folder</label>
          <input
            id="organizer-dest"
            type="text"
            class={fieldClass}
            placeholder="/home/you/Pictures/Organized"
            bind:value={destDir}
            disabled={starting}
            aria-describedby="organizer-dest-help"
            aria-invalid={destTrimmed.length > 0 && !destValid}
          />
          <p id="organizer-dest-help" class="mt-1 text-xs text-muted-foreground">
            Absolute Linux path. The folder is created if it does not exist.
          </p>
          {#if destTrimmed.length > 0 && !destValid}
            <p class="mt-1 text-xs text-destructive" role="alert">
              The destination must be an absolute Linux path starting with /.
            </p>
          {/if}
        </div>

        <div class="grid gap-4 sm:grid-cols-3">
          <div>
            <label for="organizer-source" class="text-sm font-medium">Source type</label>
            <select
              id="organizer-source"
              class={selectClass}
              value={sourceType}
              disabled={starting}
              onchange={(event) => {
                sourceType = toSourceType(event.currentTarget.value);
              }}
            >
              <option value="takeout">Google Takeout</option>
              <option value="phoneFolder">Phone folder</option>
            </select>
          </div>
          <div>
            <label for="organizer-sort" class="text-sm font-medium">Folder layout</label>
            <select
              id="organizer-sort"
              class={selectClass}
              value={sortMode}
              disabled={starting}
              onchange={(event) => {
                sortMode = toSortMode(event.currentTarget.value);
              }}
            >
              <option value="year">By year</option>
              <option value="month">By month</option>
              <option value="day">By day</option>
              <option value="flat">Flat</option>
            </select>
          </div>
          <div>
            <label for="organizer-conflict" class="text-sm font-medium">If a file exists</label>
            <select
              id="organizer-conflict"
              class={selectClass}
              value={conflictMode}
              disabled={starting}
              onchange={(event) => {
                conflictMode = toConflictMode(event.currentTarget.value);
                overwriteConfirmed = false;
              }}
            >
              <option value="keepBoth">Keep both</option>
              <option value="skip">Skip</option>
              <option value="overwrite">Overwrite</option>
            </select>
          </div>
        </div>

        <p class="text-xs text-muted-foreground">
          Date source: {sourceType === "takeout"
            ? "Takeout JSON, then filename, then EXIF, then file time."
            : "Filename, then EXIF, then file time."}
        </p>

        {#if conflictMode === "overwrite"}
          <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
            <p class="text-sm text-destructive">
              Overwrite replaces existing destination files after the new copy is written
              and verified. Files with identical content are still skipped.
            </p>
            <label class="mt-2 flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                bind:checked={overwriteConfirmed}
                disabled={starting}
              />
              I understand existing files may be replaced.
            </label>
          </div>
        {/if}

        <div class="rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
          <p class="font-medium text-foreground">This run will</p>
          <p class="mt-1 break-all">
            Read {inputPaths.length} input {inputPaths.length === 1 ? "entry" : "entries"} and
            write into {destTrimmed.length > 0 ? destTrimmed : "(no destination yet)"}.
          </p>
        </div>

        <div class="flex flex-wrap items-center gap-3">
          <Button onclick={handleStart} disabled={!canStart}>
            {starting ? "Starting…" : "Start organizing"}
          </Button>
          {#if starting}
            <span class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
              Starting the job…
            </span>
          {/if}
        </div>

        {#if startError !== null}
          <p class="text-sm text-destructive" role="alert">
            {startError.code}: {startError.message}
          </p>
        {/if}
      </div>
    </Card>

    <Card>
      <h2 class="text-base font-semibold">How this works</h2>
      <ul class="mt-3 grid gap-2 text-sm text-muted-foreground sm:grid-cols-2">
        <li>ZIP archives are streamed member by member, never fully extracted.</li>
        <li>Hidden entries and symlinks are skipped.</li>
        <li>Only photo and video files are organized, including RAW formats.</li>
        <li>Re-running is idempotent: identical files are skipped, not duplicated.</li>
        <li>Limit: 500,000 files per run.</li>
        <li>Limit: 50 GiB uncompressed per archive.</li>
      </ul>
      <p class="mt-3 text-xs text-muted-foreground">
        A corrupt archive or an inaccessible input path stops the whole run before any file
        is copied.
      </p>
    </Card>
  {:else}
    <Card>
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0">
          <h2 class="text-base font-semibold">Organizer run</h2>
          <p class="mt-1 break-all text-xs text-muted-foreground">Job {jobId}</p>
        </div>
        {#if isCompleted}
          <Badge class="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200">
            Completed
          </Badge>
        {:else if isPartial}
          <Badge class="bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200">
            Completed with errors
          </Badge>
        {:else if isCancelled}
          <Badge class="bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200">
            Cancelled
          </Badge>
        {:else if isHardFailed}
          <Badge class="bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200">Failed</Badge>
        {:else if job !== null}
          <Badge class="bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">
            {job.status === "pending" ? "Pending" : "Running"}
          </Badge>
        {/if}
      </div>

      {#if isActive}
        <div class="mt-4">
          <Progress value={percent} />
          <div class="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
            <span class="min-w-0 truncate">
              {job?.progress.message ?? "Working…"}
            </span>
            {#if hasTotal}
              <span>{job?.progress.current} / {job?.progress.total} · {percent}%</span>
            {:else}
              <span>{job?.progress.current ?? 0} files · total not yet known</span>
            {/if}
          </div>
          <div class="mt-4 flex flex-wrap items-center gap-3">
            <Button variant="destructive" onclick={handleCancel} disabled={cancelling}>
              <X class="size-4" aria-hidden="true" />
              {cancelling ? "Cancelling…" : "Cancel run"}
            </Button>
            <span class="text-xs text-muted-foreground">
              Cancellation takes effect at the next file boundary. Files already published
              are kept.
            </span>
          </div>
        </div>
      {/if}

      {#if isTerminal}
        <div class="mt-4 space-y-3">
          {#if summary !== null}
            <dl class="grid gap-2 text-sm sm:grid-cols-3">
              <div>
                <dt class="text-muted-foreground">Organized</dt>
                <dd class="font-medium">{summary.processed}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">Skipped</dt>
                <dd class="font-medium">{summary.skipped}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">Errors</dt>
                <dd class="font-medium">{summary.errors}</dd>
              </div>
            </dl>
          {/if}

          {#if isCompleted}
            <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <ShieldCheck class="size-4" aria-hidden="true" />
              Run finished. Every published file was verified before it was placed.
            </p>
          {/if}

          {#if isPartial}
            <p class="text-sm text-muted-foreground" role="status">
              The backend marks a run as failed when any file errors, but the files counted
              above were still published and verified. Check the log for the failing items.
            </p>
          {/if}

          {#if isCancelled}
            <p class="text-sm text-muted-foreground" role="status">
              Run cancelled. Files published before cancellation remain in the destination;
              nothing was rolled back.
            </p>
          {/if}

          {#if isHardFailed && job?.lastError}
            <p class="text-sm text-destructive" role="alert">{job.lastError}</p>
          {/if}

          <div class="pt-1">
            <Button variant="outline" onclick={resetRun}>Start another run</Button>
          </div>
        </div>
      {/if}

      {#if jobError !== null}
        <div class="mt-4">
          <p class="text-sm text-destructive" role="alert">
            {jobError.code}: {jobError.message}
          </p>
          <div class="mt-2">
            <Button variant="outline" size="sm" onclick={retryPolling}>Retry status</Button>
          </div>
        </div>
      {/if}

      {#if cancelError !== null}
        <p class="mt-4 text-sm text-destructive" role="alert">
          {cancelError.code}: {cancelError.message}
        </p>
      {/if}
    </Card>
  {/if}
</section>
