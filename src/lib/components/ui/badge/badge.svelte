<script lang="ts">
  import type { Snippet } from "svelte";

  let { class: className = "", children }: { class?: string; children: Snippet } = $props();
</script>

<span
  class="inline-flex items-center rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset ring-black/5 dark:ring-white/5 {className}"
>
  {@render children()}
</span>