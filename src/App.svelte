<script lang="ts">
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import { navigation } from "$lib/state/navigation.svelte";
  import Activity from "./routes/activity.svelte";
  import Dashboard from "./routes/dashboard.svelte";
  import Duplicates from "./routes/duplicates.svelte";
  import Organizer from "./routes/organizer.svelte";
  import PhoneBackup from "./routes/phone-backup.svelte";
  import Settings from "./routes/settings.svelte";
  import WifiBackup from "./routes/wifi-backup.svelte";
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
