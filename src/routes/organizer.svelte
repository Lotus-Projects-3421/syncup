<script lang="ts">
  import { FolderCog } from "@lucide/svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Organizer</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Organize Google Takeout archives and loose photo folders by date.
    </p>
  </header>

  <EmptyState
    icon={FolderCog}
    title="Takeout organization is not wired yet"
    description="Streaming ZIP processing, sidecar metadata, date precedence, and idempotent naming arrive in Phase 7. No archive is opened in this phase."
  />
</section>
