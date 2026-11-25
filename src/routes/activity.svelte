<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { commands, type Job } from "$lib/ipc";
  import { Card } from "$lib/components/ui/card";
  import { Progress } from "$lib/components/ui/progress";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import { Activity as ActivityIcon, RefreshCw, X } from "@lucide/svelte";

  let jobs = $state<Job[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let pollInterval: number | undefined;

  async function fetchJobs() {
    try {
      const result = await commands.listJobs();
      jobs = result.sort((a, b) => b.startedAt - a.startedAt);
      error = null;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : "Failed to fetch jobs";
    } finally {
      loading = false;
    }
  }

  async function cancelJob(jobId: string) {
    try {
      await commands.cancelJob(jobId);
      await fetchJobs();
    } catch (e: unknown) {
      console.error("Cancel failed:", e);
    }
  }

  function formatStatus(status: string) {
    switch (status) {
      case "running": return { text: "Running", class: "bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200" };
      case "completed": return { text: "Completed", class: "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200" };
      case "failed": return { text: "Failed", class: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200" };
      case "cancelled": return { text: "Cancelled", class: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200" };
      default: return { text: status, class: "bg-gray-100 text-gray-800 dark:bg-gray-900 dark:text-gray-200" };
    }
  }

  function formatKind(kind: string) {
    return kind.charAt(0).toUpperCase() + kind.slice(1);
  }

  function formatTime(ts: number) {
    return new Date(ts * 1000).toLocaleTimeString();
  }

  function getProgress(job: Job) {
    const p = job.progress;
    const total = p.total ?? 0;
    if (total > 0) {
      return Math.round((p.current / total) * 100);
    }
    return 0;
  }

  onMount(() => {
    fetchJobs();
    pollInterval = window.setInterval(fetchJobs, 1000);
  });

  onDestroy(() => {
    if (pollInterval) window.clearInterval(pollInterval);
  });
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header class="flex items-center justify-between">
    <div>
      <h1 class="text-2xl font-semibold tracking-tight">Activity</h1>
      <p class="mt-1 text-sm text-muted-foreground">
        Monitor running jobs, progress, and history.
      </p>
    </div>
    <Button variant="outline" size="icon" onclick={fetchJobs} aria-label="Refresh jobs">
      <RefreshCw class="size-4" />
    </Button>
  </header>

  {#if loading}
    <p class="text-sm text-muted-foreground">Loading jobs...</p>
  {:else if error}
    <Card class="border-destructive">
      <p class="text-sm text-destructive">Error: {error}</p>
    </Card>
  {:else if jobs.length === 0}
    <Card>
      <div class="flex flex-col items-center justify-center py-12 text-center">
        <ActivityIcon class="size-12 text-muted-foreground mb-4" />
        <h2 class="text-lg font-semibold">No activity yet</h2>
        <p class="mt-2 max-w-md text-sm text-muted-foreground">
          When you start a backup, organizer, or duplicate scan, it will appear here.
        </p>
      </div>
    </Card>
  {:else}
    <div class="space-y-4">
      {#each jobs as job (job.id)}
        {@const statusInfo = formatStatus(job.status)}
        <Card>
          <div class="flex items-start justify-between gap-4">
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2 mb-1">
                <h3 class="font-semibold truncate">{formatKind(job.kind)} Job</h3>
                <Badge class={statusInfo.class}>{statusInfo.text}</Badge>
              </div>
              <p class="text-xs text-muted-foreground">
                Started at {formatTime(job.startedAt)}
                {#if job.finishedAt != null}
                  • Finished at {formatTime(job.finishedAt)}
                {/if}
              </p>
              {#if job.progress.message}
                <p class="mt-2 text-sm truncate" title={job.progress.message}>
                  {job.progress.message}
                </p>
              {/if}
              {#if job.status === "running" || job.status === "pending"}
                <div class="mt-3">
                  <Progress value={getProgress(job)} />
                  <p class="mt-1 text-xs text-muted-foreground text-right">
                    {job.progress.current} {#if job.progress.total != null}/ {job.progress.total}{/if}
                  </p>
                </div>
              {/if}
              {#if job.lastError}
                <p class="mt-2 text-sm text-destructive truncate" title={job.lastError}>
                  Error: {job.lastError}
                </p>
              {/if}
            </div>
            {#if job.status === "running" || job.status === "pending"}
              <Button variant="destructive" size="icon" onclick={() => cancelJob(job.id)} aria-label="Cancel job">
                <X class="size-4" />
              </Button>
            {/if}
          </div>
        </Card>
      {/each}
    </div>
  {/if}
</section>