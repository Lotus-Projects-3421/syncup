<script lang="ts">
  import ThemeToggle from "$lib/components/theme-toggle.svelte";
</script>

<section class="mx-auto max-w-3xl space-y-6">
  <header>
    <h1 class="text-2xl font-semibold tracking-tight">Settings</h1>
    <p class="mt-1 text-sm text-muted-foreground">
      Application preferences and data locations.
    </p>
  </header>

  <div class="rounded-xl border bg-card p-5 text-card-foreground">
    <h2 class="text-base font-semibold">Appearance</h2>
    <p class="mt-1 text-sm text-muted-foreground">
      Choose a light, dark, or system-following theme. The preference is stored locally.
    </p>
    <div class="mt-4">
      <ThemeToggle />
    </div>
  </div>

  <div class="rounded-xl border bg-card p-5 text-card-foreground">
    <h2 class="text-base font-semibold">Application data</h2>
    <p class="mt-1 text-sm text-muted-foreground">
      SyncUp v2 keeps its database in the platform application data directory. The
      legacy v1 configuration is neither read nor modified in this phase.
    </p>
  </div>

  <div class="rounded-xl border bg-card p-5 text-card-foreground">
    <h2 class="text-base font-semibold">More settings</h2>
    <p class="mt-1 text-sm text-muted-foreground">
      Destination folders, device profiles, media filters, and migration controls are
      added in later phases.
    </p>
  </div>
</section>
