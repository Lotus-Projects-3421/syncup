<script lang="ts">
  import { Copy } from "@lucide/svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Duplicates</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Find identical photos and videos without permanently deleting anything.
    </p>
  </header>

  <EmptyState
    icon={Copy}
    title="Duplicate scanning is not wired yet"
    description="Candidate scanning, BLAKE2b verification, preview, manifest creation, and undo arrive in Phase 8. No files are moved in this phase."
  />
</section>
