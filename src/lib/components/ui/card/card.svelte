<script lang="ts">
  import type { Snippet } from "svelte";

  let { class: className = "", children }: { class?: string; children: Snippet } = $props();
</script>

<div
  class="rounded-xl border bg-card p-5 text-card-foreground shadow-sm {className}"
>
  {@render children()}
</div>