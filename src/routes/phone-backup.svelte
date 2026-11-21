<script lang="ts">
  import { Smartphone } from "@lucide/svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Phone Backup</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Back up photos and videos from an Android phone connected over USB.
    </p>
  </header>

  <EmptyState
    icon={Smartphone}
    title="USB device support is not wired yet"
    description="Linux MTP/GVFS discovery, folder browsing, and the backup pipeline arrive in Phase 5. No device is accessed in this phase."
  />
</section>
