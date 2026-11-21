<script lang="ts">
  import { Monitor, Moon, Sun } from "@lucide/svelte";
  import type { Component } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { theme, type ThemeMode } from "$lib/state/theme.svelte";

  const options: ReadonlyArray<{ mode: ThemeMode; label: string; icon: Component }> = [
    { mode: "light", label: "Light theme", icon: Sun },
    { mode: "dark", label: "Dark theme", icon: Moon },
    { mode: "system", label: "System theme", icon: Monitor },
  ];
</script>

<div role="group" aria-label="Theme" class="flex items-center gap-1 rounded-md border p-1">
  {#each options as option}
    {@const Icon = option.icon}
    <Button
      variant={theme.mode === option.mode ? "secondary" : "ghost"}
      size="icon-sm"
      aria-pressed={theme.mode === option.mode}
      aria-label={option.label}
      title={option.label}
      onclick={() => theme.setMode(option.mode)}
    >
      <Icon class="size-4" aria-hidden="true" />
    </Button>
  {/each}
</div>
