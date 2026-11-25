<script lang="ts">
  import {
    AppWindow,
    Copy,
    Database,
    FolderCog,
    ShieldCheck,
    Smartphone,
    Wifi,
  } from "@lucide/svelte";
  import { onMount } from "svelte";
  import type { HealthStatus, JobKind, Job_Serialize } from "$lib/ipc";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Card } from "$lib/components/ui/card";
  import { fetchJobs } from "$lib/services/jobs";
  import { fetchHealth, type FetchResult } from "$lib/services/system";

  const RECENT_LIMIT = 5;
  const POLL_INTERVAL_MS = 5000;

  let health = $state<FetchResult<HealthStatus> | null>(null);
  let healthLoading = $state(true);

  let jobsResult = $state<FetchResult<Job_Serialize[]> | null>(null);
  let jobsLoading = $state(true);

  // Monotonic guard: a response is applied only if its sequence number is the
  // latest issued, so a slow earlier request can never overwrite a newer result.
  let requestSeq = 0;

  let appVersion = $derived(
    health !== null && health.ok ? health.value.appVersion : null,
  );
  let dbVersion = $derived(
    health !== null && health.ok ? health.value.dbUserVersion : null,
  );
  let healthError = $derived(health !== null && !health.ok ? health.error : null);

  let recentJobs = $derived<Job_Serialize[]>(
    jobsResult !== null && jobsResult.ok
      ? [...jobsResult.value]
          .sort((a, b) => b.startedAt - a.startedAt)
          .slice(0, RECENT_LIMIT)
      : [],
  );
  let jobsError = $derived(
    jobsResult !== null && !jobsResult.ok ? jobsResult.error : null,
  );

  async function loadHealth(): Promise<void> {
    const result = await fetchHealth();
    health = result;
    healthLoading = false;
  }

  async function loadJobs(): Promise<void> {
    const seq = ++requestSeq;
    const result = await fetchJobs();
    if (seq !== requestSeq) return; // stale response; discard
    jobsResult = result;
    jobsLoading = false;
  }

  function statusBadge(status: Job_Serialize["status"]): {
    label: string;
    className: string;
  } {
    switch (status) {
      case "running":
        return {
          label: "Running",
          className: "bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200",
        };
      case "completed":
        return {
          label: "Completed",
          className:
            "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200",
        };
      case "failed":
        return {
          label: "Failed",
          className: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200",
        };
      case "cancelled":
        return {
          label: "Cancelled",
          className:
            "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200",
        };
      case "pending":
      default:
        return {
          label: "Pending",
          className: "bg-gray-100 text-gray-800 dark:bg-gray-900 dark:text-gray-200",
        };
    }
  }

  function kindLabel(kind: JobKind): string {
    switch (kind) {
      case "backup":
        return "Phone Backup";
      case "wifi":
        return "Wi-Fi Backup";
      case "organizer":
        return "Organizer";
      case "dedup":
        return "Duplicates";
      case "migration":
        return "Migration";
      default:
        return String(kind);
    }
  }

  function formatTime(epochSeconds: number): string {
    return new Date(epochSeconds * 1000).toLocaleString();
  }

  onMount(() => {
    let active = true;
    void loadHealth();
    void loadJobs();

    const interval = window.setInterval(() => {
      if (active) void loadJobs();
    }, POLL_INTERVAL_MS);

    return () => {
      active = false;
      requestSeq++; // invalidate any in-flight response after unmount
      window.clearInterval(interval);
    };
  });
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Dashboard</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Application health, recent activity, and quick access to SyncUp workflows.
    </p>
  </header>

  <div class="grid gap-4 md:grid-cols-3">
    <Card>
      <div
        class="flex items-center gap-2 text-sm font-medium text-muted-foreground"
      >
        <AppWindow class="size-4" aria-hidden="true" />
        Application
      </div>
      {#if healthLoading}
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
    </Card>

    <Card>
      <div
        class="flex items-center gap-2 text-sm font-medium text-muted-foreground"
      >
        <Database class="size-4" aria-hidden="true" />
        Database
      </div>
      {#if healthLoading}
        <p class="mt-4 text-sm text-muted-foreground">Opening connection pool…</p>
      {:else if dbVersion !== null}
        <p class="mt-4 text-2xl font-semibold">user_version {dbVersion}</p>
        <p class="mt-1 text-sm text-muted-foreground">
          Schema v2 active (processed-files table, WAL mode).
        </p>
      {:else if healthError !== null}
        <p class="mt-4 text-sm text-destructive" role="alert">
          {healthError.code}: {healthError.message}
        </p>
      {:else}
        <p class="mt-4 text-sm text-muted-foreground">Database status unavailable.</p>
      {/if}
    </Card>

    <Card>
      <div
        class="flex items-center gap-2 text-sm font-medium text-muted-foreground"
      >
        <ShieldCheck class="size-4" aria-hidden="true" />
        Safety
      </div>
      <p class="mt-4 text-sm text-muted-foreground">
        Core file operations use destination-local staging with atomic publish and
        size verification; selected workflows add BLAKE2b content checks. Duplicate
        cleanup moves files to quarantine rather than deleting them, and recorded
        moves can be undone via manifests where the destination is still free.
        These protections apply to the operations that implement them, not uniformly
        to every action the app performs.
      </p>
    </Card>
  </div>

  <Card>
    <div class="flex items-center justify-between gap-3">
      <h2 class="text-base font-semibold">Recent activity</h2>
      <Button variant="link" size="sm" href="#/activity">
        View all activity
      </Button>
    </div>

    {#if jobsLoading}
      <p class="mt-4 text-sm text-muted-foreground">Loading recent jobs…</p>
    {:else if jobsError !== null}
      <p class="mt-4 text-sm text-destructive" role="alert">
        {jobsError.code}: {jobsError.message}
      </p>
    {:else if recentJobs.length === 0}
      <p class="mt-4 text-sm text-muted-foreground">
        No recent activity. Start a backup, organizer run, or duplicate scan to see
        it here.
      </p>
    {:else}
      <ul class="mt-4 divide-y">
        {#each recentJobs as job (job.id)}
          {@const badge = statusBadge(job.status)}
          <li class="flex items-start justify-between gap-4 py-3 first:pt-0 last:pb-0">
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-sm font-medium">{kindLabel(job.kind)}</span>
                <Badge class={badge.className}>{badge.label}</Badge>
              </div>
              <p class="mt-1 text-xs text-muted-foreground">
                Started {formatTime(job.startedAt)}
                {#if job.finishedAt != null}
                  · Finished {formatTime(job.finishedAt)}
                {/if}
              </p>
              {#if job.progress.message}
                <p
                  class="mt-1 truncate text-xs text-muted-foreground"
                  title={job.progress.message}
                >
                  {job.progress.message}
                </p>
              {/if}
              {#if job.lastError}
                <p
                  class="mt-1 truncate text-xs text-destructive"
                  title={job.lastError}
                >
                  {job.lastError}
                </p>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </Card>

  <Card>
    <h2 class="text-base font-semibold">Quick actions</h2>
    <p class="mt-1 text-sm text-muted-foreground">
      Jump straight to a workflow.
    </p>
    <div class="mt-4 flex flex-wrap gap-2">
      <Button variant="outline" href="#/phone-backup">
        <Smartphone class="size-4" aria-hidden="true" />
        Phone Backup
      </Button>
      <Button variant="outline" href="#/wifi-backup">
        <Wifi class="size-4" aria-hidden="true" />
        Wi-Fi Backup
      </Button>
      <Button variant="outline" href="#/organizer">
        <FolderCog class="size-4" aria-hidden="true" />
        Organizer
      </Button>
      <Button variant="outline" href="#/duplicates">
        <Copy class="size-4" aria-hidden="true" />
        Duplicates
      </Button>
    </div>
  </Card>
</section>
