<script lang="ts">
  import type { Component } from "svelte";

  let {
    icon,
    title,
    description,
  }: { icon: Component; title: string; description: string } = $props();

  const Icon = $derived(icon);
</script>

<div
  class="flex min-h-80 flex-col items-center justify-center rounded-xl border border-dashed p-8 text-center"
>
  <div class="mb-4 grid size-12 place-items-center rounded-full bg-muted text-muted-foreground">
    <Icon class="size-6" aria-hidden="true" />
  </div>
  <h2 class="text-lg font-semibold">{title}</h2>
  <p class="mt-2 max-w-md text-sm text-muted-foreground">{description}</p>
</div>
