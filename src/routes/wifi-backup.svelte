<script lang="ts">
  import { Wifi } from "@lucide/svelte";
  import EmptyState from "$lib/components/empty-state.svelte";
</script>

<section class="mx-auto max-w-5xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Wi-Fi Backup</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Receive photos and videos from a phone on the local network.
    </p>
  </header>

  <EmptyState
    icon={Wifi}
    title="Wi-Fi sessions are not wired yet"
    description="The authenticated local upload server, session tokens, and staging area arrive in Phase 6. No network listener is started in this phase."
  />
</section>
