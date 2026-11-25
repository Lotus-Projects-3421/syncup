<script lang="ts">
  import { RefreshCw, ShieldCheck, Smartphone, X } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Card } from "$lib/components/ui/card";
  import { Progress } from "$lib/components/ui/progress";
  import type {
    BackupConfig,
    ConflictMode,
    FolderNode,
    Job_Serialize,
    MtpDevice,
  } from "$lib/ipc";
  import { browseFolders, discoverDevices, startBackup } from "$lib/services/backup";
  import type { AppError } from "$lib/services/errors";
  import { cancelJob, fetchJob, fetchJobs } from "$lib/services/jobs";

  const fieldClass =
    "mt-1 flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";
  const selectClass =
    "mt-1 flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";

  let devices = $state<MtpDevice[]>([]);
  let devicesLoading = $state(false);
  let devicesError = $state<AppError | null>(null);
  let devicesLoaded = $state(false);
  let selectedDeviceId = $state<string | null>(null);

  let folders = $state<FolderNode[]>([]);
  let foldersLoading = $state(false);
  let foldersError = $state<AppError | null>(null);
  let selectedPaths = $state<string[]>([]);

  let destinationRoot = $state("");
  let conflictMode = $state<ConflictMode>("skip");
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

  type FlatFolder = { relPath: string; name: string; depth: number };

  function flattenFolders(nodes: FolderNode[], depth: number, out: FlatFolder[]): void {
    for (const node of nodes) {
      out.push({ relPath: node.relPath, name: node.name, depth });
      flattenFolders(node.children, depth + 1, out);
    }
  }

  /** Linux-only: an absolute path must start with a forward slash. */
  function isAbsolutePath(value: string): boolean {
    return value.startsWith("/");
  }

  function isAncestor(ancestor: string, descendant: string): boolean {
    return descendant.startsWith(ancestor + "/");
  }

  function coveringAncestor(relPath: string): string | null {
    return selectedPaths.find((p) => isAncestor(p, relPath)) ?? null;
  }

  function toggleFolder(relPath: string): void {
    if (selectedPaths.includes(relPath)) {
      selectedPaths = selectedPaths.filter((p) => p !== relPath);
      return;
    }
    if (coveringAncestor(relPath) !== null) {
      return;
    }
    // Selecting a folder replaces any selected descendants, because the
    // backend walks each selected path independently and would visit them twice.
    selectedPaths = [...selectedPaths.filter((p) => !isAncestor(relPath, p)), relPath];
  }

  function toConflictMode(value: string): ConflictMode {
    return value === "keepBoth" || value === "overwrite" ? value : "skip";
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

  let flatFolders = $derived.by(() => {
    const out: FlatFolder[] = [];
    flattenFolders(folders, 0, out);
    return out;
  });
  let selectedDevice = $derived(devices.find((d) => d.id === selectedDeviceId) ?? null);
  let destTrimmed = $derived(destinationRoot.trim());
  let destAbsolute = $derived(isAbsolutePath(destTrimmed));
  let destInsideDevice = $derived(
    selectedDevice !== null &&
      destTrimmed.length > 0 &&
      (destTrimmed === selectedDevice.mountPath ||
        destTrimmed.startsWith(selectedDevice.mountPath + "/")),
  );
  let canStart = $derived(
    !starting &&
      selectedDeviceId !== null &&
      selectedPaths.length > 0 &&
      destAbsolute &&
      !destInsideDevice &&
      (conflictMode !== "overwrite" || overwriteConfirmed),
  );

  let percent = $derived(progressPercent(job));
  let hasTotal = $derived(job !== null && job.progress.total !== null && job.progress.total > 0);
  let isActive = $derived(
    job !== null && (job.status === "pending" || job.status === "running"),
  );
  let isCompleted = $derived(job !== null && job.status === "completed");
  let isCancelled = $derived(job !== null && job.status === "cancelled");
  let isFailed = $derived(job !== null && job.status === "failed");
  let isTerminal = $derived(isCompleted || isCancelled || isFailed);
  let uncopiedCount = $derived(
    job !== null && job.progress.total !== null
      ? Math.max(0, job.progress.total - job.progress.current)
      : 0,
  );
  let completedWithGaps = $derived(isCompleted && uncopiedCount > 0);

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
            item.kind === "backup" && (item.status === "pending" || item.status === "running"),
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

  async function loadDevices(): Promise<void> {
    devicesLoading = true;
    devicesError = null;
    const result = await discoverDevices();
    if (destroyed) {
      return;
    }
    if (result.ok) {
      devices = result.value;
      devicesLoaded = true;
      if (selectedDeviceId !== null && !devices.some((d) => d.id === selectedDeviceId)) {
        selectedDeviceId = null;
        folders = [];
        selectedPaths = [];
      }
    } else {
      devicesError = result.error;
    }
    devicesLoading = false;
  }

  async function selectDevice(id: string): Promise<void> {
    if (selectedDeviceId === id) {
      return;
    }
    selectedDeviceId = id;
    selectedPaths = [];
    folders = [];
    foldersError = null;
    foldersLoading = true;
    const result = await browseFolders(id);
    if (destroyed || selectedDeviceId !== id) {
      return;
    }
    if (result.ok) {
      folders = result.value;
    } else {
      foldersError = result.error;
    }
    foldersLoading = false;
  }

  async function handleStart(): Promise<void> {
    const deviceId = selectedDeviceId;
    if (!canStart || deviceId === null) {
      return;
    }
    starting = true;
    startError = null;
    cancelError = null;
    jobError = null;
    const config: BackupConfig = {
      deviceId,
      sourceRelPaths: selectedPaths,
      destinationRoot: destTrimmed,
      conflictMode,
      verifyHash: true,
    };
    const result = await startBackup(config);
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
    void loadDevices();
    return () => {
      destroyed = true;
      stopPolling();
    };
  });
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Phone Backup</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Back up photos and videos from an Android phone connected over USB.
    </p>
  </header>

  {#if attaching}
    <Card>
      <p class="flex items-center gap-2 text-sm text-muted-foreground">
        <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
        Checking for a running backup job…
      </p>
    </Card>
  {:else if jobId !== null}
    <Card>
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0">
          <h2 class="text-base font-semibold">Backup run</h2>
          <p class="mt-1 break-all text-xs text-muted-foreground">Job {jobId}</p>
        </div>
        {#if completedWithGaps}
          <Badge class="bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200">
            Completed with uncopied files
          </Badge>
        {:else if isCompleted}
          <Badge class="bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200">
            Completed
          </Badge>
        {:else if isCancelled}
          <Badge class="bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200">
            Cancelled
          </Badge>
        {:else if isFailed}
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
            <span class="min-w-0 truncate">{job?.progress.message ?? "Copying…"}</span>
            {#if hasTotal}
              <span>{job?.progress.current} copied / {job?.progress.total} found · {percent}%</span>
            {:else}
              <span>Preparing file list…</span>
            {/if}
          </div>
          <div class="mt-4 flex flex-wrap items-center gap-3">
            <Button variant="destructive" onclick={handleCancel} disabled={cancelling}>
              <X class="size-4" aria-hidden="true" />
              {cancelling ? "Cancelling…" : "Cancel backup"}
            </Button>
            <span class="text-xs text-muted-foreground">
              Cancellation takes effect at the next file boundary. Files already copied are kept.
            </span>
          </div>
        </div>
      {/if}

      {#if isTerminal}
        <div class="mt-4 space-y-3">
          {#if hasTotal}
            <dl class="grid gap-2 text-sm sm:grid-cols-3">
              <div>
                <dt class="text-muted-foreground">Files found</dt>
                <dd class="font-medium">{job?.progress.total}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">Files copied</dt>
                <dd class="font-medium">{job?.progress.current}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">Not copied</dt>
                <dd class="font-medium">{uncopiedCount}</dd>
              </div>
            </dl>
          {/if}

          {#if completedWithGaps}
            <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              <p class="text-sm text-destructive" role="alert">
                {uncopiedCount} of {job?.progress.total} files were not copied.
              </p>
              <p class="mt-1 text-xs text-muted-foreground">
                The backup pipeline continues past individual file errors and only aborts
                after 25 consecutive failures, so a run can finish while some files were
                skipped. The per-file reasons are written to the application log.
              </p>
            </div>
          {:else if isCompleted}
            <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <ShieldCheck class="size-4" aria-hidden="true" />
              Every file was verified with BLAKE2b before it was published.
            </p>
          {/if}

          {#if isCancelled}
            <p class="text-sm text-muted-foreground" role="status">
              Backup cancelled. Files copied before cancellation remain in the destination;
              nothing was rolled back.
            </p>
          {/if}

          {#if isFailed && job?.lastError}
            <p class="text-sm text-destructive" role="alert">{job.lastError}</p>
          {/if}

          <div class="flex flex-wrap gap-3 pt-1">
            <Button variant="outline" onclick={resetRun}>Start another backup</Button>
            {#if isFailed}
              <Button variant="ghost" onclick={loadDevices} disabled={devicesLoading}>
                <RefreshCw class="size-4" aria-hidden="true" />
                Refresh devices
              </Button>
            {/if}
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
  {:else}
    <Card>
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <Smartphone class="size-4 text-muted-foreground" aria-hidden="true" />
          <h2 class="text-base font-semibold">Device</h2>
        </div>
        <Button variant="outline" size="sm" onclick={loadDevices} disabled={devicesLoading}>
          <RefreshCw class="size-4" aria-hidden="true" />
          {devicesLoading ? "Searching…" : "Refresh devices"}
        </Button>
      </div>

      {#if devicesLoading && !devicesLoaded}
        <p class="mt-4 flex items-center gap-2 text-sm text-muted-foreground">
          <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
          Looking for mounted MTP devices…
        </p>
      {:else if devicesError !== null}
        <p class="mt-4 text-sm text-destructive" role="alert">
          {devicesError.code}: {devicesError.message}
        </p>
      {:else if devices.length === 0}
        <div class="mt-4 rounded-md border border-dashed p-4">
          <h3 class="text-sm font-semibold">No mounted device found</h3>
          <p class="mt-1 text-sm text-muted-foreground">
            SyncUp reads devices that Linux has already mounted. It cannot mount the phone
            for you. To make the phone appear:
          </p>
          <ol class="mt-3 list-decimal space-y-1 pl-5 text-sm text-muted-foreground">
            <li>Connect the phone with a USB data cable (not a charge-only cable).</li>
            <li>Unlock the phone screen.</li>
            <li>
              Open the notification shade, tap the USB notification, and choose
              <span class="font-medium text-foreground">File Transfer / MTP</span>
              (not "Charging only" or "Photo transfer / PTP").
            </li>
            <li>
              Open your Linux file manager and click the phone in the sidebar so the
              desktop mounts it over GVFS.
            </li>
            <li>Return here and press Refresh devices.</li>
          </ol>
          <p class="mt-3 text-xs text-muted-foreground">
            Once mounted, the device appears under /run/user/&lt;your-uid&gt;/gvfs/. If the
            phone still does not appear, try a different USB port or cable.
          </p>
        </div>
      {:else}
        <div class="mt-4 space-y-2" role="radiogroup" aria-label="Connected devices">
          {#each devices as device (device.id)}
            <label
              class="flex cursor-pointer items-start gap-3 rounded-md border p-3 text-sm"
            >
              <input
                type="radio"
                name="syncup-device"
                class="mt-1"
                checked={selectedDeviceId === device.id}
                disabled={starting}
                onchange={() => selectDevice(device.id)}
              />
              <span class="min-w-0">
                <span class="block font-medium break-all">{device.displayName}</span>
                <span class="block text-xs text-muted-foreground break-all">
                  {device.mountPath}
                </span>
              </span>
            </label>
          {/each}
          <p class="text-xs text-muted-foreground">
            The name shown is the device serial reported by GVFS, not a product name.
          </p>
        </div>
      {/if}

      <p class="mt-4 text-xs text-muted-foreground">
        USB backup is Linux-only in this version. On Windows or macOS use Wi-Fi Backup instead.
      </p>
    </Card>

    {#if selectedDeviceId !== null}
      <Card>
        <h2 class="text-base font-semibold">Folders to back up</h2>
        <p class="mt-1 text-sm text-muted-foreground">
          Each selected folder is copied recursively, including everything underneath it.
        </p>

        {#if foldersLoading}
          <p class="mt-4 flex items-center gap-2 text-sm text-muted-foreground">
            <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
            Reading the device folder tree…
          </p>
        {:else if foldersError !== null}
          <p class="mt-4 text-sm text-destructive" role="alert">
            {foldersError.code}: {foldersError.message}
          </p>
        {:else if flatFolders.length === 0}
          <p class="mt-4 text-sm text-muted-foreground">
            No folders were listed for this device.
          </p>
        {:else}
          <div class="mt-4 max-h-80 overflow-y-auto rounded-md border p-3">
            <ul class="space-y-1">
              {#each flatFolders as folder (folder.relPath)}
                {@const covered = coveringAncestor(folder.relPath)}
                {@const checked = selectedPaths.includes(folder.relPath)}
                <li style="padding-left: {folder.depth * 16}px">
                  <label class="flex items-start gap-2 text-sm">
                    <input
                      type="checkbox"
                      class="mt-1"
                      checked={checked || covered !== null}
                      disabled={covered !== null || starting}
                      onchange={() => toggleFolder(folder.relPath)}
                    />
                    <span class="min-w-0 break-all">
                      {folder.name}
                      {#if covered !== null}
                        <span class="text-xs text-muted-foreground">
                          (already included via {covered})
                        </span>
                      {/if}
                    </span>
                  </label>
                </li>
              {/each}
            </ul>
          </div>
          <p class="mt-2 text-xs text-muted-foreground">
            Only the first three folder levels are listed, up to 2000 entries. A folder shown
            without subfolders may still contain deeper folders — selecting it backs up
            everything underneath. Selecting a parent folder clears its children from the
            selection so no file is visited twice.
          </p>
          <p class="mt-2 text-sm">
            {selectedPaths.length} folder{selectedPaths.length === 1 ? "" : "s"} selected.
          </p>
        {/if}
      </Card>

      <Card>
        <h2 class="text-base font-semibold">Destination and options</h2>

        <div class="mt-4 space-y-4">
          <div>
            <label for="backup-dest" class="text-sm font-medium">Destination folder</label>
            <input
              id="backup-dest"
              type="text"
              class={fieldClass}
              placeholder="/home/you/Pictures/PhoneBackup"
              bind:value={destinationRoot}
              disabled={starting}
              aria-describedby="backup-dest-help"
              aria-invalid={destTrimmed.length > 0 && (!destAbsolute || destInsideDevice)}
            />
            <p id="backup-dest-help" class="mt-1 text-xs text-muted-foreground">
              Absolute Linux path to a folder that already exists. SyncUp will not create it.
              The device folder structure is mirrored inside it.
            </p>
            {#if destTrimmed.length > 0 && !destAbsolute}
              <p class="mt-1 text-xs text-destructive" role="alert">
                The destination must be an absolute Linux path starting with /.
              </p>
            {/if}
            {#if destInsideDevice}
              <p class="mt-1 text-xs text-destructive" role="alert">
                The destination must not be inside the device mount.
              </p>
            {/if}
          </div>

          <div>
            <label for="backup-conflict" class="text-sm font-medium">If a file already exists</label>
            <select
              id="backup-conflict"
              class={selectClass}
              value={conflictMode}
              disabled={starting}
              onchange={(event) => {
                conflictMode = toConflictMode(event.currentTarget.value);
                overwriteConfirmed = false;
              }}
            >
              <option value="skip">Skip (recommended)</option>
              <option value="keepBoth">Keep both</option>
              <option value="overwrite">Overwrite</option>
            </select>
            {#if conflictMode === "skip"}
              <p class="mt-1 text-xs text-muted-foreground">
                Existing files at the same path are left untouched. This makes repeat backups
                safe to re-run.
              </p>
            {:else if conflictMode === "keepBoth"}
              <p class="mt-1 text-xs text-destructive">
                Keep both writes a numbered copy such as "name (1).jpg" whenever the target
                exists. Re-running the same backup will duplicate every file. Use Skip for
                repeat backups.
              </p>
            {:else}
              <p class="mt-1 text-xs text-muted-foreground">
                The existing file is replaced only after the new copy is written and verified.
              </p>
            {/if}
          </div>

          {#if conflictMode === "overwrite"}
            <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              <p class="text-sm text-destructive">
                Overwrite replaces existing files in the destination. This cannot be undone.
              </p>
              <label class="mt-2 flex items-center gap-2 text-sm">
                <input type="checkbox" bind:checked={overwriteConfirmed} disabled={starting} />
                I understand existing files may be replaced.
              </label>
            </div>
          {/if}

          <div class="rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
            <p>Every file is verified with BLAKE2b before it is published. Verification cannot be disabled.</p>
            <p class="mt-1">Limit: 100,000 files per run. The run aborts after 25 consecutive file errors.</p>
          </div>

          <div class="flex flex-wrap items-center gap-3">
            <Button onclick={handleStart} disabled={!canStart}>
              {starting ? "Starting…" : "Start backup"}
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
    {/if}
  {/if}
</section>
