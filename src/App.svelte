<script lang="ts">
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import { fetchSettings, saveSettings } from "$lib/services/settings";
  import { navigation } from "$lib/state/navigation.svelte";
  import { theme } from "$lib/state/theme.svelte";
  import Activity from "./routes/activity.svelte";
  import Dashboard from "./routes/dashboard.svelte";
  import Duplicates from "./routes/duplicates.svelte";
  import Organizer from "./routes/organizer.svelte";
  import PhoneBackup from "./routes/phone-backup.svelte";
  import Settings from "./routes/settings.svelte";
  import WifiBackup from "./routes/wifi-backup.svelte";

  // main.ts already reconciled the persisted theme before mounting, so the first
  // effect run must not write anything back. Without this guard a failed
  // bootstrap fetch could let a stale local cache overwrite the saved theme.
  let themeSyncInitialized = false;

  async function persistTheme(mode: string): Promise<void> {
    const current = await fetchSettings();
    if (!current.ok || current.value.theme === mode) {
      return;
    }
    // Spread the loaded settings so unrelated fields are never dropped.
    // Failures are already logged by the service layer.
    await saveSettings({ ...current.value, theme: mode });
  }

  $effect(() => {
    const mode = theme.mode;
    if (!themeSyncInitialized) {
      themeSyncInitialized = true;
      return;
    }
    void persistTheme(mode);
  });
</script>

<div class="flex min-h-screen flex-col bg-background text-foreground md:flex-row">
  <a
    href="#main-content"
    class="sr-only focus:not-sr-only focus:absolute focus:top-4 focus:left-4 focus:z-50 focus:rounded-md focus:bg-primary focus:px-3 focus:py-2 focus:text-primary-foreground"
  >
    Skip to main content
  </a>

  <AppSidebar />

  <main id="main-content" tabindex="-1" class="min-w-0 flex-1 p-4 sm:p-6 lg:p-8">
    {#if navigation.current === "dashboard"}
      <Dashboard />
    {:else if navigation.current === "phone-backup"}
      <PhoneBackup />
    {:else if navigation.current === "wifi-backup"}
      <WifiBackup />
    {:else if navigation.current === "organizer"}
      <Organizer />
    {:else if navigation.current === "duplicates"}
      <Duplicates />
    {:else if navigation.current === "activity"}
      <Activity />
    {:else}
      <Settings />
    {/if}
  </main>
</div>
