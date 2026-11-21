<script lang="ts">
  import {
    Activity,
    Copy,
    FolderCog,
    LayoutDashboard,
    Settings,
    Smartphone,
    Wifi,
  } from "@lucide/svelte";
  import type { Component } from "svelte";
  import { navigation, routes, type RouteId } from "$lib/state/navigation.svelte";
  import { cn } from "$lib/utils";
  import ThemeToggle from "./theme-toggle.svelte";

  const icons: Record<RouteId, Component> = {
    dashboard: LayoutDashboard,
    "phone-backup": Smartphone,
    "wifi-backup": Wifi,
    organizer: FolderCog,
    duplicates: Copy,
    activity: Activity,
    settings: Settings,
  };
</script>

<aside
  class="flex w-full shrink-0 flex-col border-b border-sidebar-border bg-sidebar md:min-h-screen md:w-64 md:border-r md:border-b-0"
>
  <div class="flex items-center justify-between gap-2 px-4 py-4">
    <a
      href="#/dashboard"
      class="flex items-center gap-2 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring"
      aria-label="SyncUp dashboard"
    >
      <span
        class="grid size-8 place-items-center rounded-lg bg-primary text-sm font-bold text-primary-foreground"
        aria-hidden="true">S</span
      >
      <span class="text-base font-semibold tracking-tight">SyncUp</span>
    </a>
    <div class="md:hidden">
      <ThemeToggle />
    </div>
  </div>

  <nav
    aria-label="Main navigation"
    class="flex gap-1 overflow-x-auto px-2 pb-3 md:flex-col md:overflow-visible md:pb-0"
  >
    {#each routes as route}
      {@const Icon = icons[route.id]}
      <a
        href={`#/${route.id}`}
        aria-current={navigation.current === route.id ? "page" : undefined}
        class={cn(
          "flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium text-sidebar-foreground/80 transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring",
          navigation.current === route.id &&
            "bg-sidebar-accent text-sidebar-accent-foreground",
        )}
      >
        <Icon class="size-4 shrink-0" aria-hidden="true" />
        <span class="whitespace-nowrap">{route.label}</span>
      </a>
    {/each}
  </nav>

  <div class="mt-auto hidden border-t border-sidebar-border p-3 md:block">
    <p class="mb-2 px-1 text-xs font-medium text-sidebar-foreground/60">Theme</p>
    <ThemeToggle />
    <p class="mt-3 px-1 text-xs text-sidebar-foreground/50">v2 foundation</p>
  </div>
</aside>
