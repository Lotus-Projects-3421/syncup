<script lang="ts">
  import { RefreshCw, X } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Card } from "$lib/components/ui/card";
  import type { DedupManifestSummary, DuplicateGroup, Job_Serialize } from "$lib/ipc";
  import {
    fetchManifests,
    fetchScanResults,
    startMove,
    startScan,
    startUndo,
  } from "$lib/services/dedup";
  import type { AppError } from "$lib/services/errors";
  import { cancelJob, fetchJob, fetchJobs } from "$lib/services/jobs";

  const fieldClass =
    "mt-1 flex w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";

  // Scan state
  let scanRootsText = $state("");
  let scanExcludesText = $state("");
  let scanStarting = $state(false);
  let scanError = $state<AppError | null>(null);

  // Job state (shared by scan, move, undo)
  let jobId = $state<string | null>(null);
  let job = $state<Job_Serialize | null>(null);
  let jobError = $state<AppError | null>(null);
  let cancelling = $state(false);
  let cancelError = $state<AppError | null>(null);

  // Scan results
  let groups = $state<DuplicateGroup[]>([]);

  // Move state
  let moveConfirmed = $state(false);
  let moveStarting = $state(false);
  let moveError = $state<AppError | null>(null);
  let moveJobId = $state<string | null>(null);

  // Manifest state
  let manifests = $state<DedupManifestSummary[]>([]);
  let manifestsLoading = $state(false);
  let manifestsError = $state<AppError | null>(null);
  let selectedManifestId = $state<string | null>(null);
  let undoConfirmed = $state(false);
  let undoStarting = $state(false);
  let undoError = $state<AppError | null>(null);
  let undoJobId = $state<string | null>(null);
  let undoResultManifest = $state<DedupManifestSummary | null>(null);
  let moveManifestError = $state<AppError | null>(null);
  let undoManifestError = $state<AppError | null>(null);
  let moveBaselineIds = $state<string[] | null>(null);
  let moveManifestCreated = $state(false);
  let cancelledMove = $state(false);
  let cancelledUndo = $state(false);

  // Re-entry state
  let attaching = $state(true);

  let pollTimer: number | undefined;
  let destroyed = false;

  // Scan derived
  let scanRoots = $derived(scanRootsText.split("\n").map((s) => s.trim()).filter((s) => s.length > 0));
  let scanExcludes = $derived(scanExcludesText.split("\n").map((s) => s.trim()).filter((s) => s.length > 0));
  let scanInvalid = $derived(scanRoots.length === 0);
  let canStartScan = $derived(!scanStarting && !scanInvalid && jobId === null);

  // Job derived
  let isActive = $derived(job !== null && (job.status === "pending" || job.status === "running"));
  let isTerminal = $derived(job !== null && (job.status === "completed" || job.status === "failed" || job.status === "cancelled"));
  let progressPercent = $derived(job?.progress.total ? Math.round((job.progress.current / job.progress.total) * 100) : 0);

  // Move derived
  let canStartMove = $derived(
    !moveStarting && groups.length > 0 && moveConfirmed && jobId === null
  );

  // Manifest derived
  let selectedManifest = $derived(manifests.find((m) => m.manifestId === selectedManifestId) ?? null);
  let canStartUndo = $derived(
    !undoStarting && selectedManifest !== null && undoConfirmed && jobId === null
  );

  // Undo job tracking
  let undoInProgress = $derived(undoJobId !== null && jobId === undoJobId);
  let moveInProgress = $derived(moveJobId !== null && jobId === moveJobId);

  async function handleStartScan(): Promise<void> {
    if (!canStartScan) return;
    scanStarting = true;
    scanError = null;
    groups = [];
    moveManifestError = null;
    undoManifestError = null;
    moveBaselineIds = null;
    moveManifestCreated = false;
    cancelledMove = false;
    cancelledUndo = false;
    const result = await startScan({ scanRoots, excludePatterns: scanExcludes });
    if (result.ok) {
      jobId = result.value;
      job = null;
      startPolling();
    } else {
      scanError = result.error;
    }
    scanStarting = false;
  }

  async function loadScanResults(scanJobId: string): Promise<void> {
    const result = await fetchScanResults(scanJobId);
    if (result.ok) {
      groups = result.value;
    }
  }

  async function handleStartMove(): Promise<void> {
    if (!canStartMove || jobId !== null) return;
    moveStarting = true;
    moveError = null;
    moveManifestError = null;
    moveManifestCreated = false;

    const baseline = await fetchManifests();
    if (baseline.ok) {
      moveBaselineIds = baseline.value.map((m) => m.manifestId);
    }

    const result = await startMove(scanJobId!, true);
    if (result.ok) {
      moveJobId = result.value;
      jobId = result.value;
      job = null;
      groups = [];
      startPolling();
    } else {
      moveError = result.error;
    }
    moveStarting = false;
  }

  let scanJobId = $derived(jobId !== null && !moveJobId && !undoJobId ? jobId : null);

  async function loadManifests(): Promise<void> {
    manifestsLoading = true;
    manifestsError = null;
    const result = await fetchManifests();
    if (result.ok) {
      manifests = result.value;
    } else {
      manifestsError = result.error;
    }
    manifestsLoading = false;
  }

  async function handleStartUndo(): Promise<void> {
    if (!canStartUndo || selectedManifest === null || jobId !== null) return;
    undoStarting = true;
    undoError = null;
    undoResultManifest = null;
    const result = await startUndo(selectedManifest.manifestId);
    if (result.ok) {
      undoJobId = result.value;
      jobId = result.value;
      job = null;
      startPolling();
    } else {
      undoError = result.error;
    }
    undoStarting = false;
  }

  async function handleCancel(): Promise<void> {
    if (jobId === null || cancelling) return;
    cancelling = true;
    cancelError = null;
    const result = await cancelJob(jobId);
    if (!result.ok) {
      cancelError = result.error;
    }
    cancelling = false;
  }

  function startPolling(): void {
    stopPolling();
    if (destroyed) return;
    pollTimer = window.setInterval(() => {
      void pollJob();
    }, 1000);
  }

  function stopPolling(): void {
    if (pollTimer !== undefined) {
      window.clearInterval(pollTimer);
      pollTimer = undefined;
    }
  }

  async function pollJob(): Promise<void> {
    if (jobId === null) return;
    const result = await fetchJob(jobId);
    if (!result.ok) {
      jobError = result.error;
      return;
    }
    job = result.value;
    jobError = null;

    if (result.value.status === "completed") {
      stopPolling();
      const completedJobId = jobId;
      // If this was a scan, load results
      if (completedJobId === scanJobId) {
        await loadScanResults(completedJobId);
      }
      // If this was a move, refresh manifests
      if (completedJobId === moveJobId) {
        const manifestResult = await fetchManifests();
        if (manifestResult.ok && moveBaselineIds !== null) {
          const known = new Set(moveBaselineIds);
          const created = manifestResult.value.filter((m) => !known.has(m.manifestId));
          if (created.length === 1) {
            moveManifestCreated = true;
          } else if (created.length === 0) {
            moveManifestError = { code: "UNKNOWN", message: "No new readable manifest was found. The move outcome could not be verified; files may still have been moved." };
          } else {
            moveManifestError = { code: "UNKNOWN", message: `${created.length} new manifests appeared. The move outcome could not be verified; files may still have been moved.` };
          }
        } else if (!manifestResult.ok) {
          moveManifestError = manifestResult.error;
        }
        moveJobId = null;
        moveBaselineIds = null;
      }
      // If this was an undo, refresh the specific manifest
      if (completedJobId === undoJobId) {
        const manifestResult = await fetchManifests();
        if (!manifestResult.ok) {
          undoManifestError = manifestResult.error;
        } else {
          const updated = manifests.find((m) => m.manifestId === selectedManifestId);
          if (updated) {
            undoResultManifest = updated;
          }
        }
        undoJobId = null;
      }
      jobId = null;
      job = null;
    } else if (result.value.status === "failed" || result.value.status === "cancelled") {
      stopPolling();
      if (jobId === moveJobId) {
        cancelledMove = result.value.status === "cancelled";
        moveJobId = null;
      }
      if (jobId === undoJobId) {
        cancelledUndo = result.value.status === "cancelled";
        undoJobId = null;
      }
      jobId = null;
      job = null;
    }
  }

  async function attachToActiveJob(): Promise<void> {
    attaching = true;
    const result = await fetchJobs();
    if (result.ok) {
      const active = result.value.find((j) => j.kind === "dedup" && (j.status === "pending" || j.status === "running"));
      if (active) {
        jobId = active.id;
        job = active;
        startPolling();
      }
    }
    attaching = false;
  }

  onMount(() => {
    void attachToActiveJob();
    void loadManifests();
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
      Find and safely remove duplicate files. Nothing is deleted; duplicates are moved to quarantine and can be restored.
    </p>
  </header>

  {#if attaching}
    <Card>
      <p class="flex items-center gap-2 text-sm text-muted-foreground">
        <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
        Checking for active jobs…
      </p>
    </Card>
  {:else}
    <!-- Scan Setup -->
    <Card>
      <h2 class="text-base font-semibold">Scan for Duplicates</h2>
      <div class="mt-4 space-y-4">
        <div>
          <label for="scan-roots" class="text-sm font-medium">Folders to scan</label>
          <textarea
            id="scan-roots"
            class={fieldClass}
            placeholder="/home/user/Pictures&#10;/home/user/Documents"
            bind:value={scanRootsText}
            rows="3"
            disabled={jobId !== null}
          ></textarea>
          <p class="mt-1 text-xs text-muted-foreground">One absolute path per line. Overlapping folders are deduplicated automatically.</p>
        </div>
        <div>
          <label for="scan-excludes" class="text-sm font-medium">Exclude patterns (optional)</label>
          <textarea
            id="scan-excludes"
            class={fieldClass}
            placeholder=".git&#10;node_modules"
            bind:value={scanExcludesText}
            rows="2"
            disabled={jobId !== null}
          ></textarea>
          <p class="mt-1 text-xs text-muted-foreground">One pattern per line. Files and folders matching these patterns are skipped.</p>
        </div>
        <Button onclick={handleStartScan} disabled={!canStartScan}>
          {scanStarting ? "Starting…" : "Start Scan"}
        </Button>
        {#if scanError}
          <p class="text-sm text-destructive">{scanError.message}</p>
        {/if}
      </div>
    </Card>

    <!-- Scan Results Preview -->
    {#if groups.length > 0 && jobId === null}
      <Card>
        <h2 class="text-base font-semibold">Scan Results</h2>
        <p class="mt-2 text-sm text-muted-foreground">
          Found {groups.length} duplicate group{groups.length === 1 ? "" : "s"} with {groups.reduce((sum, g) => sum + g.duplicates.length, 0)} file{groups.reduce((sum, g) => sum + g.duplicates.length, 0) === 1 ? "" : "s"} that can be moved to quarantine.
        </p>
        <div class="mt-4 space-y-3">
          {#each groups.slice(0, 5) as group}
            <div class="rounded-md border p-3">
              <p class="text-sm font-medium">Group: {group.duplicates.length + 1} files · {group.size} bytes each</p>
              <p class="mt-1 text-xs text-muted-foreground">Hash: {group.hash.slice(0, 16)}…</p>
              <ul class="mt-2 space-y-1">
                <li class="text-xs"><strong>Keep:</strong> {group.keep.path}</li>
                {#each group.duplicates as dup}
                  <li class="text-xs text-muted-foreground">Move: {dup.path}</li>
                {/each}
              </ul>
            </div>
          {/each}
          {#if groups.length > 5}
            <p class="text-sm text-muted-foreground">…and {groups.length - 5} more group{groups.length - 5 === 1 ? "" : "s"}</p>
          {/if}
        </div>
        <div class="mt-4 space-y-3">
          <label class="flex items-start gap-2 text-sm">
            <input type="checkbox" bind:checked={moveConfirmed} class="mt-1" disabled={jobId !== null} />
            <span>I understand that all {groups.reduce((sum, g) => sum + g.duplicates.length, 0)} duplicate file{groups.reduce((sum, g) => sum + g.duplicates.length, 0) === 1 ? "" : "s"} will be moved to quarantine. This can be undone later.</span>
          </label>
          <Button onclick={handleStartMove} disabled={!canStartMove}>
            {moveStarting ? "Starting…" : "Move Duplicates to Quarantine"}
          </Button>
          {#if moveError}
            <p class="text-sm text-destructive">{moveError.message}</p>
          {/if}
        </div>
      </Card>
    {/if}

    <!-- Active Job Progress -->
    {#if isActive}
      <Card>
        <div class="flex items-start justify-between gap-3">
          <div>
            <h2 class="text-base font-semibold">
              {moveInProgress ? "Moving Duplicates" : undoInProgress ? "Restoring Files" : "Scanning for Duplicates"}
            </h2>
            <p class="mt-1 break-all text-xs text-muted-foreground">Job {jobId}</p>
          </div>
          <Button variant="destructive" size="sm" onclick={handleCancel} disabled={cancelling}>
            <X class="size-4" aria-hidden="true" />
            {cancelling ? "Cancelling…" : "Cancel"}
          </Button>
        </div>
        <div class="mt-4">
          <div class="flex items-center gap-2 text-sm">
            <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
            <span>{job?.progress.message ?? "Working…"}</span>
          </div>
          <div class="mt-2 h-2 w-full overflow-hidden rounded-full bg-muted">
            <div class="h-full bg-primary transition-all" style="width: {progressPercent}%"></div>
          </div>
          <p class="mt-1 text-xs text-muted-foreground">{progressPercent}%</p>
        </div>
        {#if cancelError}
          <p class="mt-3 text-sm text-destructive">{cancelError.message}</p>
        {/if}
      </Card>
    {/if}

    <!-- Terminal Job Results -->
    {#if job !== null && isTerminal && !isActive}
      <Card>
        <h2 class="text-base font-semibold">
          {moveInProgress ? "Move" : undoInProgress ? "Restore" : "Scan"}
          {job.status === "completed" ? "Completed" : job.status === "cancelled" ? "Cancelled" : "Failed"}
        </h2>
        {#if job.status === "completed"}
          {#if scanJobId && groups.length > 0}
            <p class="mt-2 text-sm">Scan found {groups.length} duplicate group{groups.length === 1 ? "" : "s"}.</p>
          {:else if moveInProgress}
            {#if moveManifestError}
              <p class="mt-2 text-sm text-destructive">
                {moveManifestError.message}
              </p>
            {:else if moveManifestCreated}
              <p class="mt-2 text-sm">Duplicates moved to quarantine. Check the Manifest History below to undo if needed.</p>
            {:else}
              <p class="mt-2 text-sm text-muted-foreground">Move completed, but the manifest record could not be verified.</p>
            {/if}
          {:else if undoInProgress}
            {#if undoManifestError}
              <p class="mt-2 text-sm text-destructive">
                {undoManifestError.message} The undo outcome could not be verified; some files may still have been restored.
              </p>
            {:else if undoResultManifest}
              <div class="mt-2 space-y-2">
              <p class="text-sm">Restore completed for manifest {undoResultManifest.manifestId.slice(0, 8)}…</p>
              <dl class="grid grid-cols-2 gap-2 text-sm">
                <div>
                  <dt class="text-muted-foreground">Restored</dt>
                  <dd class="font-medium">{undoResultManifest.undoneOps}</dd>
                </div>
                <div>
                  <dt class="text-muted-foreground">Already restored</dt>
                  <dd class="font-medium">{undoResultManifest.undoneOps}</dd>
                </div>
                <div>
                  <dt class="text-muted-foreground">Blocked</dt>
                  <dd class="font-medium">{undoResultManifest.blockedOps}</dd>
                </div>
                <div>
                  <dt class="text-muted-foreground">Failed</dt>
                  <dd class="font-medium">{undoResultManifest.failedOps}</dd>
                </div>
              </dl>
              {#if undoResultManifest.blockedOps > 0}
                <p class="text-xs text-muted-foreground">
                  Blocked operations occur when the original file location is occupied. The quarantine file is preserved.
                </p>
              {/if}
              {#if undoResultManifest.attentionOps > 0}
                <p class="text-xs text-destructive">
                  {undoResultManifest.attentionOps} operation{undoResultManifest.attentionOps === 1 ? "" : "s"} need attention. Check the application log for details.
                </p>
              {/if}
            </div>
            {/if}
          {/if}
        {:else if job.status === "cancelled" && cancelledMove}
          <p class="mt-2 text-sm text-muted-foreground">
            Move cancelled at a file boundary. Files already moved to quarantine remain there.
          </p>
        {:else if job.status === "cancelled" && cancelledUndo}
          <p class="mt-2 text-sm text-muted-foreground">
            Undo cancelled at a file boundary. Files already restored remain restored, while remaining operations were not necessarily completed.
          </p>
        {:else if job.status === "cancelled"}
          <p class="mt-2 text-sm text-muted-foreground">Job cancelled. No files were changed.</p>
        {:else if job.lastError}
          <p class="mt-2 text-sm text-destructive">{job.lastError}</p>
        {/if}
      </Card>
    {/if}

    <!-- Manifest History -->
    {#if manifests.length > 0 || manifestsLoading}
      <Card>
        <div class="flex items-start justify-between gap-3">
          <h2 class="text-base font-semibold">Manifest History</h2>
          <Button variant="outline" size="sm" onclick={loadManifests} disabled={manifestsLoading}>
            <RefreshCw class={manifestsLoading ? "size-4 animate-spin" : "size-4"} aria-hidden="true" />
            Refresh
          </Button>
        </div>
        {#if manifestsLoading}
          <p class="mt-3 flex items-center gap-2 text-sm text-muted-foreground">
            <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
            Loading manifests…
          </p>
        {:else if manifestsError}
          <p class="mt-3 text-sm text-destructive">{manifestsError.message}</p>
        {:else if manifests.length === 0}
          <p class="mt-3 text-sm text-muted-foreground">No manifests found.</p>
        {:else}
          <div class="mt-3 space-y-3">
            {#each manifests as manifest}
              {@const isSelected = selectedManifestId === manifest.manifestId}
              <div class="rounded-md border p-3" class:border-primary={isSelected}>
                <label class="flex items-start gap-3">
                  <input
                    type="radio"
                    name="manifest-select"
                    bind:group={selectedManifestId}
                    value={manifest.manifestId}
                    class="mt-1"
                    disabled={jobId !== null}
                  />
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center gap-2">
                      <p class="text-sm font-medium break-all">{manifest.manifestId.slice(0, 8)}…</p>
                      {#if manifest.potentiallyUndoable}
                        <Badge class="bg-muted text-muted-foreground text-xs">Undoable</Badge>
                      {/if}
                    </div>
                    <p class="mt-1 text-xs text-muted-foreground">Created: {new Date(manifest.createdAt).toLocaleString()}</p>
                    <dl class="mt-2 grid grid-cols-3 gap-2 text-xs">
                      <div>
                        <dt class="text-muted-foreground">Total</dt>
                        <dd class="font-medium">{manifest.totalOps}</dd>
                      </div>
                      <div>
                        <dt class="text-muted-foreground">Moved</dt>
                        <dd class="font-medium">{manifest.completedOps}</dd>
                      </div>
                      <div>
                        <dt class="text-muted-foreground">Restored</dt>
                        <dd class="font-medium">{manifest.undoneOps}</dd>
                      </div>
                      {#if manifest.failedOps > 0}
                        <div>
                          <dt class="text-muted-foreground">Failed</dt>
                          <dd class="font-medium">{manifest.failedOps}</dd>
                        </div>
                      {/if}
                      {#if manifest.blockedOps > 0}
                        <div>
                          <dt class="text-muted-foreground">Blocked</dt>
                          <dd class="font-medium">{manifest.blockedOps}</dd>
                        </div>
                      {/if}
                      {#if manifest.attentionOps > 0}
                        <div>
                          <dt class="text-muted-foreground">Attention</dt>
                          <dd class="font-medium text-destructive">{manifest.attentionOps}</dd>
                        </div>
                      {/if}
                    </dl>
                  </div>
                </label>
              </div>
            {/each}
          </div>

          {#if selectedManifest}
            <div class="mt-4 space-y-3">
              {#if selectedManifest.completedOps === 0}
                <p class="text-sm text-muted-foreground">
                  This manifest has no completed operations. Nothing to restore.
                </p>
              {:else if selectedManifest.undoneOps === selectedManifest.completedOps}
                <p class="text-sm text-muted-foreground">
                  All operations in this manifest have already been restored.
                </p>
              {:else}
                <label class="flex items-start gap-2 text-sm">
                  <input type="checkbox" bind:checked={undoConfirmed} class="mt-1" disabled={jobId !== null} />
                  <span>
                    I understand that {selectedManifest.completedOps - selectedManifest.undoneOps} file{selectedManifest.completedOps - selectedManifest.undoneOps === 1 ? "" : "s"} will be restored to their original locations. Files whose original locations are occupied will be skipped.
                  </span>
                </label>
                <Button onclick={handleStartUndo} disabled={!canStartUndo}>
                  {undoStarting ? "Starting…" : "Restore Files from Quarantine"}
                </Button>
                {#if undoError}
                  <p class="text-sm text-destructive">{undoError.message}</p>
                {/if}
              {/if}
            </div>
          {/if}
        {/if}
      </Card>
    {/if}

    {#if jobError}
      <Card>
        <p class="text-sm text-destructive">{jobError.message}</p>
      </Card>
    {/if}
  {/if}
</section>
