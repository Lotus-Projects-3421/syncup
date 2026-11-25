<script lang="ts">
  import { Copy, RefreshCw, ShieldCheck, X } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Card } from "$lib/components/ui/card";
  import type { DuplicateGroup, Job_Serialize } from "$lib/ipc";
  import { fetchScanResults, startScan } from "$lib/services/dedup";
  import type { AppError } from "$lib/services/errors";
  import { cancelJob, fetchJob, fetchJobs } from "$lib/services/jobs";

  const fieldClass =
    "mt-1 flex w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";

  /** Groups rendered at once. Large scans must not freeze the webview. */
  const PAGE_SIZE = 25;

  let rootsText = $state("");
  let excludesText = $state("");

  let starting = $state(false);
  let startError = $state<AppError | null>(null);

  let jobId = $state<string | null>(null);
  let job = $state<Job_Serialize | null>(null);
  let jobError = $state<AppError | null>(null);
  let cancelling = $state(false);
  let cancelError = $state<AppError | null>(null);

  let attaching = $state(true);
  let attachError = $state<AppError | null>(null);
  let attached = $state(false);

  let groups = $state<DuplicateGroup[]>([]);
  let resultsLoaded = $state(false);
  let resultsLoading = $state(false);
  let resultsError = $state<AppError | null>(null);
  let visibleCount = $state(PAGE_SIZE);

  let pollTimer: number | undefined;
  let pollInFlight = false;
  let destroyed = false;

  /** Linux-only: an absolute path must start with a forward slash. */
  function isAbsolutePath(value: string): boolean {
    return value.startsWith("/");
  }

  function parseLines(raw: string): string[] {
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

  function formatBytes(value: number): string {
    if (!Number.isFinite(value) || value <= 0) {
      return "0 B";
    }
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let size = value;
    let unit = 0;
    while (size >= 1024 && unit < units.length - 1) {
      size = size / 1024;
      unit += 1;
    }
    return (unit === 0 ? size.toFixed(0) : size.toFixed(1)) + " " + units[unit];
  }

  function formatModified(mtimeNs: number): string {
    if (!Number.isFinite(mtimeNs) || mtimeNs <= 0) {
      return "unknown";
    }
    const ms = Math.floor(mtimeNs / 1_000_000);
    const date = new Date(ms);
    if (Number.isNaN(date.getTime())) {
      return "unknown";
    }
    return date.toLocaleString();
  }

  let scanRoots = $derived(parseLines(rootsText));
  let excludePatterns = $derived(parseLines(excludesText));
  let invalidRoots = $derived(scanRoots.filter((path) => !isAbsolutePath(path)));
  let canStart = $derived(
    !starting && !attaching && jobId === null && scanRoots.length > 0 && invalidRoots.length === 0,
  );

  // A dedup scan never enters "running": the backend only sets that on its
  // first progress update, which run_scan never sends. Both non-terminal
  // states therefore mean "still working".
  let isActive = $derived(
    job !== null && (job.status === "pending" || job.status === "running"),
  );
  let isCompleted = $derived(job !== null && job.status === "completed");
  let isCancelled = $derived(job !== null && job.status === "cancelled");
  let isFailed = $derived(job !== null && job.status === "failed");

  let totalGroups = $derived(groups.length);
  let totalDuplicates = $derived(
    groups.reduce((sum, group) => sum + group.duplicates.length, 0),
  );
  let reclaimableBytes = $derived(
    groups.reduce((sum, group) => sum + group.size * group.duplicates.length, 0),
  );
  let visibleGroups = $derived(groups.slice(0, visibleCount));
  let hasMore = $derived(visibleCount < totalGroups);
  let noDuplicates = $derived(isCompleted && resultsLoaded && totalGroups === 0);

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

  function clearResults(): void {
    groups = [];
    resultsLoaded = false;
    resultsError = null;
    visibleCount = PAGE_SIZE;
  }

  async function loadResults(id: string): Promise<void> {
    resultsLoading = true;
    resultsError = null;
    const result = await fetchScanResults(id);
    if (destroyed || jobId !== id) {
      return;
    }
    if (result.ok) {
      groups = result.value;
      resultsLoaded = true;
      visibleCount = PAGE_SIZE;
    } else {
      groups = [];
      resultsLoaded = false;
      resultsError = result.error;
    }
    resultsLoading = false;
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
      if (result.value.status === "completed") {
        stopPolling();
        await loadResults(id);
      } else if (result.value.status === "cancelled" || result.value.status === "failed") {
        stopPolling();
        // A cancelled or failed run must never leave earlier groups on screen.
        clearResults();
      }
    } else {
      // A polling failure is not evidence that the job stopped.
      jobError = result.error;
      stopPolling();
    }
  }

  function retryPolling(): void {
    jobError = null;
    startPolling();
    void pollJob();
  }

  async function attachToActiveJob(): Promise<void> {
    attaching = true;
    attachError = null;
    const result = await fetchJobs();
    if (destroyed) {
      attaching = false;
      return;
    }
    if (result.ok) {
      const active = result.value
        .filter(
          (item) =>
            item.kind === "dedup" && (item.status === "pending" || item.status === "running"),
        )
        .sort((a, b) => b.startedAt - a.startedAt)[0];
      if (active !== undefined) {
        jobId = active.id;
        job = active;
        attached = true;
        clearResults();
        startPolling();
      }
    } else {
      attachError = result.error;
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
    attachError = null;
    // Discard any previous preview before the new run begins.
    clearResults();
    const result = await startScan({ scanRoots, excludePatterns });
    if (destroyed) {
      starting = false;
      return;
    }
    if (result.ok) {
      jobId = result.value;
      job = null;
      attached = false;
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
    if (destroyed || jobId !== id) {
      cancelling = false;
      return;
    }
    if (result.ok) {
      await pollJob();
    } else {
      cancelError = result.error;
    }
    cancelling = false;
  }

  function resetScan(): void {
    stopPolling();
    jobId = null;
    job = null;
    attached = false;
    jobError = null;
    startError = null;
    cancelError = null;
    clearResults();
  }

  function showMore(): void {
    visibleCount = visibleCount + PAGE_SIZE;
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
    <h1 class="text-2xl font-semibold tracking-tight">Duplicates</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Find identical photos and videos without permanently deleting anything.
    </p>
  </header>

  {#if attaching}
    <Card>
      <p class="flex items-center gap-2 text-sm text-muted-foreground">
        <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
        Checking for a running duplicate job…
      </p>
    </Card>
  {:else if jobId === null}
    <Card>
      <div class="flex items-center gap-2">
        <Copy class="size-4 text-muted-foreground" aria-hidden="true" />
        <h2 class="text-base font-semibold">Scan setup</h2>
      </div>

      <div class="mt-4 space-y-4">
        <div>
          <label for="dedup-roots" class="text-sm font-medium">Folders to scan</label>
          <textarea
            id="dedup-roots"
            rows="4"
            class={fieldClass}
            placeholder={"/home/you/Pictures\n/home/you/Backup/Phone"}
            bind:value={rootsText}
            disabled={starting}
            aria-describedby="dedup-roots-help"
            aria-invalid={invalidRoots.length > 0}
          ></textarea>
          <p id="dedup-roots-help" class="mt-1 text-xs text-muted-foreground">
            One absolute Linux path per line. Each folder is scanned recursively. Overlapping
            folders are counted once per file.
          </p>
          {#if invalidRoots.length > 0}
            <div class="mt-1 text-xs text-destructive" role="alert">
              <p>These entries are not absolute Linux paths (they must start with /):</p>
              <ul class="mt-1 space-y-0.5 break-all">
                {#each invalidRoots as bad (bad)}
                  <li>{bad}</li>
                {/each}
              </ul>
            </div>
          {/if}
        </div>

        <div>
          <label for="dedup-excludes" class="text-sm font-medium">Exclude names containing</label>
          <textarea
            id="dedup-excludes"
            rows="2"
            class={fieldClass}
            placeholder={"thumbnail\nedited"}
            bind:value={excludesText}
            disabled={starting}
            aria-describedby="dedup-excludes-help"
          ></textarea>
          <p id="dedup-excludes-help" class="mt-1 text-xs text-muted-foreground">
            Optional. One fragment per line. Any file or folder whose name contains a fragment
            is skipped. This is a plain substring match, not a glob pattern.
          </p>
        </div>

        <div class="rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
          <p>Files are compared by size, then a quick hash, then a full BLAKE2b hash.</p>
          <p class="mt-1">Hidden entries, symlinks, empty files and _Duplicates_ folders are skipped.</p>
          <p class="mt-1">Nothing is moved during a scan. This step only reads files.</p>
        </div>

        <div class="flex flex-wrap items-center gap-3">
          <Button onclick={handleStart} disabled={!canStart}>
            {starting ? "Starting…" : "Start scan"}
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

        {#if attachError !== null}
          <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
            <p class="text-sm text-destructive" role="alert">
              {attachError.code}: {attachError.message}
            </p>
            <p class="mt-1 text-xs text-muted-foreground">
              The active-job check failed, so a duplicate job may still be running.
            </p>
            <div class="mt-2">
              <Button variant="outline" size="sm" onclick={attachToActiveJob}>Re-check</Button>
            </div>
          </div>
        {/if}
      </div>
    </Card>
  {:else}
    <Card>
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0">
          <h2 class="text-base font-semibold">Duplicate scan</h2>
          <p class="mt-1 break-all text-xs text-muted-foreground">Job {jobId}</p>
        </div>
        {#if isCompleted}
          <Badge class="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200">
            Completed
          </Badge>
        {:else if isCancelled}
          <Badge class="bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200">
            Cancelled
          </Badge>
        {:else if isFailed}
          <Badge class="bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200">Failed</Badge>
        {:else}
          <Badge class="bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">
            Scanning
          </Badge>
        {/if}
      </div>

      {#if attached && isActive}
        <div class="mt-4 rounded-md border border-destructive/40 bg-destructive/10 p-3">
          <p class="text-sm text-destructive" role="alert">
            A duplicate job was already running when this page opened.
          </p>
          <p class="mt-1 text-xs text-muted-foreground">
            Scans, moves and restores share one job type, and this job has not reported which
            operation it is performing. Its progress is shown below as it arrives. Nothing has
            been stopped automatically.
          </p>
        </div>
      {/if}

      {#if isActive}
        <div class="mt-4">
          <div class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
            {#if job?.progress.message}
              <span class="min-w-0 truncate">{job.progress.message}</span>
            {:else}
              <span>Scanning files. This can take a while on large folders.</span>
            {/if}
          </div>
          <p class="mt-2 text-xs text-muted-foreground">
            The backend does not report how many files a scan has examined, so no percentage or
            file count is shown. The result appears when the scan finishes.
          </p>
          {#if job?.progress.total !== null && job?.progress.total !== undefined}
            <p class="mt-1 text-xs text-muted-foreground">
              {job.progress.current} of {job.progress.total} operations reported.
            </p>
          {/if}
          <div class="mt-4 flex flex-wrap items-center gap-3">
            <Button variant="destructive" onclick={handleCancel} disabled={cancelling}>
              <X class="size-4" aria-hidden="true" />
              {cancelling ? "Cancelling…" : "Cancel job"}
            </Button>
            <span class="text-xs text-muted-foreground">
              Cancellation takes effect at the next file boundary.
            </span>
          </div>
        </div>
      {/if}

      {#if isCancelled}
        <div class="mt-4 space-y-3">
          <p class="text-sm text-muted-foreground" role="status">
            Job cancelled. No preview is shown for a cancelled run, and nothing was moved.
          </p>
          <div>
            <Button variant="outline" onclick={resetScan}>Start another scan</Button>
          </div>
        </div>
      {/if}

      {#if isFailed}
        <div class="mt-4 space-y-3">
          <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
            {#if job?.lastError}
              <p class="text-sm text-destructive" role="alert">{job.lastError}</p>
            {:else}
              <p class="text-sm text-destructive" role="alert">
                The job failed without reporting a reason.
              </p>
            {/if}
            <p class="mt-1 text-xs text-muted-foreground">
              Nothing was moved. Check that every scan folder exists and is readable, then scan
              again.
            </p>
          </div>
          <div>
            <Button variant="outline" onclick={resetScan}>Start another scan</Button>
          </div>
        </div>
      {/if}

      {#if isCompleted}
        <div class="mt-4 space-y-3">
          {#if resultsLoading}
            <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
              Loading the preview…
            </p>
          {:else if resultsError !== null}
            <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              <p class="text-sm text-destructive" role="alert">
                {resultsError.code}: {resultsError.message}
              </p>
              <p class="mt-1 text-xs text-muted-foreground">
                Scan results are held in memory only. They are cleared after a move and lost
                when the application closes, so this preview can no longer be retrieved. Run a
                new scan to see the current duplicates.
              </p>
            </div>
          {:else if noDuplicates}
            <div class="rounded-md border border-dashed p-6 text-center">
              <ShieldCheck class="mx-auto size-6 text-muted-foreground" aria-hidden="true" />
              <h3 class="mt-2 text-sm font-semibold">No duplicates found</h3>
              <p class="mt-1 text-sm text-muted-foreground">
                Every scanned file was unique. Nothing needs to be moved.
              </p>
            </div>
          {:else if resultsLoaded}
            <dl class="grid gap-2 text-sm sm:grid-cols-3">
              <div>
                <dt class="text-muted-foreground">Duplicate groups</dt>
                <dd class="font-medium">{totalGroups}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">Copies that would move</dt>
                <dd class="font-medium">{totalDuplicates}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">Space they occupy</dt>
                <dd class="font-medium">{formatBytes(reclaimableBytes)}</dd>
              </div>
            </dl>
          {/if}

          <div>
            <Button variant="outline" onclick={resetScan}>Start another scan</Button>
          </div>
        </div>
      {/if}

      {#if jobError !== null}
        <div class="mt-4">
          <p class="text-sm text-destructive" role="alert">
            {jobError.code}: {jobError.message}
          </p>
          <p class="mt-1 text-xs text-muted-foreground">
            Job status could not be read. The job may still be running; it has not been treated
            as stopped.
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

    {#if resultsLoaded && totalGroups > 0}
      <Card>
        <h2 class="text-base font-semibold">Preview</h2>
        <div class="mt-2 rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
          <p class="font-medium text-foreground">
            This is the complete set of copies a move would quarantine.
          </p>
          <p class="mt-1">
            SyncUp chooses which copy to keep using folder depth, then oldest modification time,
            then path order. Individual copies cannot be selected or deselected: a move acts on
            every group shown here. To narrow the set, scan fewer folders or add exclusions.
          </p>
          <p class="mt-1">
            Nothing has been moved yet. Moving and undoing are not available on this screen yet.
          </p>
        </div>

        <p class="mt-3 text-xs text-muted-foreground">
          Showing {visibleGroups.length} of {totalGroups} groups.
        </p>

        <div class="mt-3 space-y-4">
          {#each visibleGroups as group, index (group.hash + "-" + index)}
            <div class="rounded-md border p-3">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <h3 class="text-sm font-medium">Group {index + 1}</h3>
                <span class="text-xs text-muted-foreground">
                  {formatBytes(group.size)} each · {group.duplicates.length} cop{group.duplicates.length === 1 ? "y" : "ies"} would move
                </span>
              </div>

              <div class="mt-3">
                <p class="text-xs font-medium text-foreground">Kept</p>
                <div class="mt-1 rounded-md border border-dashed p-2">
                  <p class="break-all text-sm">{group.keep.path}</p>
                  <p class="mt-1 text-xs text-muted-foreground">
                    Modified {formatModified(group.keep.mtimeNs)}
                  </p>
                </div>
              </div>

              <div class="mt-3">
                <p class="text-xs font-medium text-foreground">Will be moved to quarantine</p>
                <ul class="mt-1 space-y-2">
                  {#each group.duplicates as dup (dup.path)}
                    <li class="rounded-md border p-2">
                      <p class="break-all text-sm">{dup.path}</p>
                      <p class="mt-1 text-xs text-muted-foreground">
                        Modified {formatModified(dup.mtimeNs)}
                      </p>
                    </li>
                  {/each}
                </ul>
              </div>

              <p class="mt-3 break-all text-xs text-muted-foreground">
                BLAKE2b {group.hash}
              </p>
            </div>
          {/each}
        </div>

        {#if hasMore}
          <div class="mt-4">
            <Button variant="outline" onclick={showMore}>Show more groups</Button>
          </div>
        {/if}
      </Card>
    {/if}
  {/if}
</section>
