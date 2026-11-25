<script lang="ts">
  import { Copy, RefreshCw, ShieldCheck, Wifi, X } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { toDataURL } from "qrcode";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Card } from "$lib/components/ui/card";
  import { Progress } from "$lib/components/ui/progress";
  import type {
    Job_Serialize,
    JobStatus,
    WifiConfig_Deserialize,
    WifiSessionInfo,
  } from "$lib/ipc";
  import type { AppError } from "$lib/services/errors";
  import { toAppError } from "$lib/services/errors";
  import { fetchJob, fetchJobs } from "$lib/services/jobs";
  import { startWifiSession, stopWifiSession } from "$lib/services/wifi";

  const fieldClass =
    "mt-1 flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";

  let destinationRoot = $state("");
  let starting = $state(false);
  let startError = $state<AppError | null>(null);

  let info = $state<WifiSessionInfo | null>(null);
  let jobId = $state<string | null>(null);
  let job = $state<Job_Serialize | null>(null);

  let attaching = $state(true);
  let attachError = $state<AppError | null>(null);
  let jobError = $state<AppError | null>(null);

  let stopping = $state(false);
  let stopError = $state<AppError | null>(null);
  let stopNotice = $state<string | null>(null);
  let stopRequested = $state(false);

  let orphaned = $state(false);
  let ownedSession = $state(false);
  let observedPhase = $state(false);
  let everFinalizing = $state(false);

  let qrDataUrl = $state<string | null>(null);
  let qrError = $state<AppError | null>(null);
  let copied = $state(false);
  let copyError = $state<AppError | null>(null);

  let pollTimer: number | undefined;
  let copyTimer: number | undefined;
  let pollInFlight = false;
  let destroyed = false;

  type Phase = "unknown" | "upload" | "finalize";

  /** Linux-only: an absolute path must start with a forward slash. */
  function isAbsolutePath(value: string): boolean {
    return value.startsWith("/");
  }

  function isTerminalStatus(status: JobStatus): boolean {
    return status === "completed" || status === "failed" || status === "cancelled";
  }

  /**
   * The backend reuses one job for both phases and writes distinguishable
   * messages: "session active" and "N uploaded, M failed" during intake (where
   * current/total are bytes against the session quota), then "publishing N files"
   * and "N published, M errors" during finalization (where current/total are file
   * counts). Detecting the phase from the message keeps the labels honest.
   */
  function detectPhase(message: string | null): Phase {
    if (message === null) {
      return "unknown";
    }
    const text = message.trim();
    if (text === "session active" || /^\d+ uploaded, \d+ failed$/.test(text)) {
      return "upload";
    }
    if (/^publishing \d+ files$/.test(text) || /^\d+ published, \d+ errors$/.test(text)) {
      return "finalize";
    }
    return "unknown";
  }

  function parseUpload(text: string | null): { uploaded: number; failed: number } | null {
    if (text === null) {
      return null;
    }
    const match = /^(\d+) uploaded, (\d+) failed$/.exec(text.trim());
    if (match === null) {
      return null;
    }
    return { uploaded: Number(match[1]), failed: Number(match[2]) };
  }

  function parseFinalize(text: string | null): { published: number; errors: number } | null {
    if (text === null) {
      return null;
    }
    const match = /^(\d+) published, (\d+) errors$/.exec(text.trim());
    if (match === null) {
      return null;
    }
    return { published: Number(match[1]), errors: Number(match[2]) };
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

  function percentOf(current: Job_Serialize | null): number {
    if (current === null) {
      return 0;
    }
    const total = current.progress.total;
    if (total === null || total <= 0) {
      return 0;
    }
    return Math.min(100, Math.max(0, Math.round((current.progress.current / total) * 100)));
  }

  /**
   * Records what this page actually observed, so terminal wording can tell the
   * difference between a cancel before publishing, a cancel during publishing,
   * and a session whose earlier phase was never seen (refresh recovery).
   */
  function notePhase(snapshot: Job_Serialize): void {
    const phase = detectPhase(snapshot.progress.message);
    if (phase !== "unknown") {
      observedPhase = true;
    }
    if (phase === "finalize") {
      everFinalizing = true;
    }
  }

  let destTrimmed = $derived(destinationRoot.trim());
  let destValid = $derived(isAbsolutePath(destTrimmed));
  let canStart = $derived(!starting && !attaching && jobId === null && destValid);

  let phase = $derived(detectPhase(job === null ? null : job.progress.message));
  let percent = $derived(percentOf(job));
  let isActive = $derived(
    job !== null && (job.status === "pending" || job.status === "running"),
  );
  let finalizing = $derived(isActive && (stopRequested || phase === "finalize"));
  let uploading = $derived(isActive && !finalizing);
  let isCompleted = $derived(job !== null && job.status === "completed");
  let isCancelled = $derived(job !== null && job.status === "cancelled");
  let isFailed = $derived(job !== null && job.status === "failed");
  let isTerminal = $derived(isCompleted || isCancelled || isFailed);

  let uploadSummary = $derived(parseUpload(job === null ? null : job.progress.message));
  let finalizeSummary = $derived(parseFinalize(job === null ? null : job.progress.message));

  // A cancel can only be described as "nothing published, staging cleared" when
  // this page owned the session, saw its intake phase, and never saw or asked
  // for publishing. Anything else needs qualified wording.
  let cancelBeforePublish = $derived(
    isCancelled && !stopRequested && !everFinalizing && ownedSession && observedPhase,
  );
  let cancelDuringPublish = $derived(isCancelled && (stopRequested || everFinalizing));
  let cancelUnknownPhase = $derived(
    isCancelled && !cancelBeforePublish && !cancelDuringPublish,
  );

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
      notePhase(result.value);
      if (isTerminalStatus(result.value.status)) {
        stopPolling();
        stopNotice = null;
      }
    } else {
      // A polling failure is not evidence that the session stopped.
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
            item.kind === "wifi" && (item.status === "pending" || item.status === "running"),
        )
        .sort((a, b) => b.startedAt - a.startedAt)[0];
      if (active !== undefined) {
        jobId = active.id;
        job = active;
        info = null;
        orphaned = true;
        ownedSession = false;
        notePhase(active);
        startPolling();
      }
    } else {
      attachError = result.error;
    }
    attaching = false;
  }

  async function generateQr(url: string): Promise<void> {
    qrError = null;
    qrDataUrl = null;
    try {
      // URL only. The token must never be encoded into the QR image.
      qrDataUrl = await toDataURL(url, { margin: 1, width: 220, errorCorrectionLevel: "M" });
    } catch (error) {
      qrError = toAppError(error);
    }
  }

  async function handleStart(): Promise<void> {
    if (!canStart) {
      return;
    }
    starting = true;
    startError = null;
    attachError = null;
    const config: WifiConfig_Deserialize = {
      destinationRoot: destTrimmed,
      bindAll: false,
      ttlSecs: null,
    };
    const result = await startWifiSession(config);
    if (destroyed) {
      starting = false;
      return;
    }
    if (result.ok) {
      info = result.value;
      jobId = result.value.jobId;
      job = null;
      ownedSession = true;
      orphaned = false;
      stopRequested = false;
      observedPhase = false;
      everFinalizing = false;
      stopError = null;
      stopNotice = null;
      jobError = null;
      copied = false;
      copyError = null;
      await generateQr(result.value.url);
      const first = await fetchJob(result.value.jobId);
      if (!destroyed && first.ok) {
        job = first.value;
        notePhase(first.value);
      }
      startPolling();
    } else {
      startError = result.error;
    }
    starting = false;
  }

  async function copyToken(): Promise<void> {
    const value = info === null ? null : info.token;
    if (value === null) {
      return;
    }
    copyError = null;
    try {
      await navigator.clipboard.writeText(value);
      copied = true;
      if (copyTimer !== undefined) {
        window.clearTimeout(copyTimer);
      }
      copyTimer = window.setTimeout(() => {
        copied = false;
      }, 2000);
    } catch {
      // Fixed wording: never echo clipboard content or the token into the UI.
      copyError = {
        code: "CLIPBOARD",
        message: "Could not copy automatically. Select the token above and copy it manually.",
      };
    }
  }

  /**
   * Explicit stop. Publishing progress must appear immediately, so polling is
   * restarted and kicked before the IPC is awaited. The backend holds the stop
   * request open until finalization finishes, and the job status is the only
   * authoritative outcome.
   */
  async function handleStop(): Promise<void> {
    const id = jobId;
    if (id === null || stopping) {
      return;
    }
    stopping = true;
    stopError = null;
    stopNotice = null;
    stopRequested = true;
    startPolling();
    void pollJob();

    const result = await stopWifiSession();
    if (destroyed || jobId !== id) {
      stopping = false;
      return;
    }

    const checked = await fetchJob(id);
    if (destroyed || jobId !== id) {
      stopping = false;
      return;
    }
    if (checked.ok) {
      job = checked.value;
      jobError = null;
      notePhase(checked.value);
      if (isTerminalStatus(checked.value.status)) {
        stopPolling();
        stopNotice = null;
        stopError = null;
      } else if (result.ok) {
        // stop() also returns Ok when the backend holds no session, for example
        // while an earlier stop is still publishing. Do not claim success.
        stopNotice =
          "Stop request accepted, but the backend job is still running. It may already be publishing files. Waiting for it to reach a final state.";
        startPolling();
      } else {
        stopError = result.error;
        startPolling();
      }
    } else {
      jobError = checked.error;
      if (!result.ok) {
        stopError = result.error;
      }
      startPolling();
    }
    stopping = false;
  }

  function resetSession(): void {
    stopPolling();
    jobId = null;
    job = null;
    info = null;
    orphaned = false;
    ownedSession = false;
    observedPhase = false;
    everFinalizing = false;
    stopRequested = false;
    jobError = null;
    startError = null;
    stopError = null;
    stopNotice = null;
    qrDataUrl = null;
    qrError = null;
    copied = false;
    copyError = null;
  }

  onMount(() => {
    void attachToActiveJob();
    return () => {
      destroyed = true;
      stopPolling();
      if (copyTimer !== undefined) {
        window.clearTimeout(copyTimer);
      }
    };
  });
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Wi-Fi Backup</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Receive photos and videos from a phone on the local network.
    </p>
  </header>

  <div class="rounded-xl border border-destructive/40 bg-destructive/10 p-4" role="note">
    <div class="flex items-start gap-2">
      <Wifi class="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
      <div class="min-w-0">
        <h2 class="text-sm font-semibold text-destructive">
          Not encrypted — use only on a network you trust
        </h2>
        <p class="mt-1 text-sm text-muted-foreground">
          Wi-Fi transfer uses plain HTTP. Your files and the session token travel without
          encryption, so anyone able to monitor this network can observe them. This version
          has no HTTPS. Prefer USB Phone Backup on shared or public networks.
        </p>
      </div>
    </div>
  </div>

  {#if attaching}
    <Card>
      <p class="flex items-center gap-2 text-sm text-muted-foreground">
        <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
        Checking for an active Wi-Fi session…
      </p>
    </Card>
  {:else if jobId === null}
    <Card>
      <h2 class="text-base font-semibold">Session setup</h2>

      <div class="mt-4 space-y-4">
        <div>
          <label for="wifi-dest" class="text-sm font-medium">Destination folder</label>
          <input
            id="wifi-dest"
            type="text"
            class={fieldClass}
            placeholder="/home/you/Pictures/PhoneBackup"
            bind:value={destinationRoot}
            disabled={starting}
            aria-describedby="wifi-dest-help"
            aria-invalid={destTrimmed.length > 0 && !destValid}
          />
          <p id="wifi-dest-help" class="mt-1 text-xs text-muted-foreground">
            Absolute Linux path to a folder that already exists. SyncUp will not create it.
            Uploaded files are published here when you stop the session.
          </p>
          {#if destTrimmed.length > 0 && !destValid}
            <p class="mt-1 text-xs text-destructive" role="alert">
              The destination must be an absolute Linux path starting with /.
            </p>
          {/if}
        </div>

        <div class="rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
          <p>Uploads are staged first and only published to the destination when you press Stop session.</p>
          <p class="mt-1">Sessions expire after 30 minutes. Expiry, and cancelling the job from Activity, discard staged uploads without publishing them.</p>
          <p class="mt-1">One session at a time. Limits: 8 GiB per file, 50 GiB per session.</p>
        </div>

        <div class="flex flex-wrap items-center gap-3">
          <Button onclick={handleStart} disabled={!canStart}>
            {starting ? "Starting…" : "Start session"}
          </Button>
          {#if starting}
            <span class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <RefreshCw class="size-4 animate-spin" aria-hidden="true" />
              Starting the upload server…
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
              The active-session check failed, so an existing session may still be running.
              Starting a new one is refused by the backend while another is active.
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
          <h2 class="text-base font-semibold">Wi-Fi session</h2>
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
        {:else if finalizing}
          <Badge class="bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">
            Finalizing backup
          </Badge>
        {:else if job !== null}
          <Badge class="bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">
            {job.status === "pending" ? "Pending" : "Accepting uploads"}
          </Badge>
        {/if}
      </div>

      {#if orphaned && isActive}
        <div class="mt-4 rounded-md border border-destructive/40 bg-destructive/10 p-3">
          <p class="text-sm text-destructive" role="alert">
            An active Wi-Fi session was detected, but its connection details are unavailable.
          </p>
          <p class="mt-1 text-xs text-muted-foreground">
            The address and token are returned only once when a session starts and are not
            stored, so they cannot be restored after a reload. You can still stop the session
            below; stopping publishes whatever has already been uploaded. Nothing is stopped
            automatically.
          </p>
        </div>
      {/if}

      {#if info !== null && uploading}
        <div class="mt-4 grid gap-4 sm:grid-cols-[auto_1fr]">
          <div>
            {#if qrDataUrl !== null}
              <img
                src={qrDataUrl}
                alt="QR code containing the session address only"
                class="size-[220px] rounded-md border bg-white p-1"
                width="220"
                height="220"
              />
            {:else if qrError !== null}
              <div class="flex size-[220px] items-center justify-center rounded-md border border-dashed p-3 text-center text-xs text-muted-foreground">
                QR code unavailable. Type the address below into the phone browser.
              </div>
            {:else}
              <div class="flex size-[220px] items-center justify-center rounded-md border border-dashed text-xs text-muted-foreground">
                Generating QR code…
              </div>
            {/if}
            <p class="mt-1 text-xs text-muted-foreground">The QR code contains the address only.</p>
          </div>

          <div class="min-w-0 space-y-3">
            <div>
              <p class="text-sm font-medium">Address</p>
              <p class="mt-1 break-all text-sm text-muted-foreground">{info.url}</p>
              <p class="mt-1 text-xs text-muted-foreground">
                {info.ip} · port {info.port}
              </p>
            </div>

            <div>
              <label for="wifi-token" class="text-sm font-medium">Session token</label>
              <input
                id="wifi-token"
                type="text"
                class={fieldClass}
                value={info.token}
                readonly
                aria-describedby="wifi-token-help"
              />
              <div class="mt-2 flex flex-wrap items-center gap-2">
                <Button variant="outline" size="sm" onclick={copyToken}>
                  <Copy class="size-4" aria-hidden="true" />
                  {copied ? "Copied" : "Copy token"}
                </Button>
                {#if copied}
                  <span class="text-xs text-muted-foreground" role="status">Token copied to the clipboard.</span>
                {/if}
              </div>
              <p id="wifi-token-help" class="mt-1 text-xs text-muted-foreground">
                The token is never copied automatically and is never part of the address or the
                QR code. Anyone holding it can upload to this session.
              </p>
              {#if copyError !== null}
                <p class="mt-1 text-xs text-destructive" role="alert">{copyError.message}</p>
              {/if}
            </div>

            <div>
              <p class="text-sm font-medium">On the phone</p>
              <ol class="mt-1 list-decimal space-y-1 pl-5 text-xs text-muted-foreground">
                <li>Join the same Wi-Fi network as this computer.</li>
                <li>Scan the QR code, or open the address above in the phone browser.</li>
                <li>Paste or type the session token into the page.</li>
                <li>Choose photos or videos and upload them.</li>
                <li>Return here and press Stop session to publish them to the destination.</li>
              </ol>
            </div>

            <p class="break-all text-xs text-muted-foreground">Session {info.sessionId}</p>
          </div>
        </div>
      {/if}

      {#if isActive}
        <div class="mt-4">
          <Progress value={percent} />
          <div class="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
            {#if finalizing}
              <span class="min-w-0 truncate" role="status">
                {job?.progress.message ?? "Publishing staged files…"}
              </span>
              <span>
                {job?.progress.current ?? 0} / {job?.progress.total ?? 0} files · {percent}%
              </span>
            {:else if phase === "upload"}
              <span class="min-w-0 truncate" role="status">
                {uploadSummary === null
                  ? "Waiting for uploads…"
                  : uploadSummary.uploaded + " uploaded, " + uploadSummary.failed + " failed"}
              </span>
              <span>
                Session quota used: {formatBytes(job?.progress.current ?? 0)} of {formatBytes(job?.progress.total ?? 0)}
              </span>
            {:else}
              <span class="min-w-0 truncate" role="status">{job?.progress.message ?? "Working…"}</span>
              <span>{percent}%</span>
            {/if}
          </div>

          {#if finalizing}
            <p class="mt-2 text-xs text-muted-foreground">
              Uploads are closed. Staged files are being verified and published to the destination.
              The backup is not complete until this finishes.
            </p>
          {:else}
            <p class="mt-2 text-xs text-muted-foreground">
              The bar shows session quota usage, not how much of your backup is finished.
            </p>
          {/if}

          <div class="mt-4 flex flex-wrap items-center gap-3">
            <Button variant="destructive" onclick={handleStop} disabled={stopping}>
              <X class="size-4" aria-hidden="true" />
              {stopping
                ? "Stopping…"
                : orphaned
                  ? "Stop existing session"
                  : "Stop session and publish"}
            </Button>
            <span class="text-xs text-muted-foreground">
              Stopping closes uploads and publishes staged files. Cancelling this job from the
              Activity screen instead discards staged uploads.
            </span>
          </div>

          {#if stopNotice !== null}
            <p class="mt-3 text-sm text-muted-foreground" role="status">{stopNotice}</p>
          {/if}
        </div>
      {/if}

      {#if isTerminal}
        <div class="mt-4 space-y-3">
          {#if finalizeSummary !== null}
            <dl class="grid gap-2 text-sm sm:grid-cols-2">
              <div>
                <dt class="text-muted-foreground">Files published</dt>
                <dd class="font-medium">{finalizeSummary.published}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">Publish errors</dt>
                <dd class="font-medium">{finalizeSummary.errors}</dd>
              </div>
            </dl>
          {/if}

          {#if isCompleted}
            <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <ShieldCheck class="size-4" aria-hidden="true" />
              Session completed. Staged files were verified and published to the destination,
              then the staging area was removed.
            </p>
          {/if}

          {#if cancelBeforePublish}
            <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              <p class="text-sm text-destructive" role="alert">
                Session cancelled before publishing began. Uploaded files were not published
                and the staging area was cleared.
              </p>
              <p class="mt-1 text-xs text-muted-foreground">
                Nothing was written to the destination. Upload the files again in a new session.
              </p>
            </div>
          {/if}

          {#if cancelDuringPublish}
            <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              <p class="text-sm text-destructive" role="alert">
                Session cancelled while publishing. Some files may already be in the destination.
              </p>
              <p class="mt-1 text-xs text-muted-foreground">
                The remaining staged files were preserved rather than deleted. Check the
                destination folder and the application log to see which files arrived.
              </p>
            </div>
          {/if}

          {#if cancelUnknownPhase}
            <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              <p class="text-sm text-destructive" role="alert">
                Session ended as cancelled. This page could not observe when it stopped.
              </p>
              <p class="mt-1 text-xs text-muted-foreground">
                If it stopped before publishing, staged uploads were discarded. If it stopped
                during publishing, some files may already be in the destination and the rest
                were preserved. Check the destination folder and the application log.
              </p>
            </div>
          {/if}

          {#if isFailed}
            <div class="rounded-md border border-destructive/40 bg-destructive/10 p-3">
              {#if job?.lastError}
                <p class="text-sm text-destructive" role="alert">{job.lastError}</p>
              {:else}
                <p class="text-sm text-destructive" role="alert">Publishing failed.</p>
              {/if}
              <p class="mt-1 text-xs text-muted-foreground">
                Some files may already have been published to the destination. The staging area
                was preserved for manual recovery. SyncUp cannot resume this session
                automatically — inspect the destination folder and the application log, then
                upload anything missing in a new session.
              </p>
            </div>
          {/if}

          <div class="pt-1">
            <Button variant="outline" onclick={resetSession}>Start another session</Button>
          </div>
        </div>
      {/if}

      {#if jobError !== null}
        <div class="mt-4">
          <p class="text-sm text-destructive" role="alert">
            {jobError.code}: {jobError.message}
          </p>
          <p class="mt-1 text-xs text-muted-foreground">
            Job status could not be read. The session may still be active; it has not been
            treated as stopped.
          </p>
          <div class="mt-2">
            <Button variant="outline" size="sm" onclick={retryPolling}>Retry status</Button>
          </div>
        </div>
      {/if}

      {#if stopError !== null}
        <div class="mt-4">
          <p class="text-sm text-destructive" role="alert">
            {stopError.code}: {stopError.message}
          </p>
          {#if isActive}
            <p class="mt-1 text-xs text-muted-foreground">
              The job is still active, so the session has not been cleared. You can try stopping
              again.
            </p>
          {/if}
        </div>
      {/if}
    </Card>
  {/if}
</section>
