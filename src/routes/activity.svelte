<script lang="ts">
  import { Activity } from "@lucide/svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Activity</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Review completed jobs, warnings, and recoverable failures.
    </p>
  </header>

  <EmptyState
    icon={Activity}
    title="No activity yet"
    description="Job history and progress events are not implemented in this phase. Backup, organizer, and duplicate runs will appear here once their services are wired."
  />
</section>
