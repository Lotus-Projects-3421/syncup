<script lang="ts">
  let { value = 0, class: className = "", ...restProps }: { value?: number; class?: string } = $props();
</script>

<div
  class="relative h-2 w-full overflow-hidden rounded-full bg-primary/20 {className}"
  role="progressbar"
  aria-valuenow={value}
  aria-valuemin={0}
  aria-valuemax={100}
  {...restProps}
>
  <div
    class="h-full bg-primary transition-all duration-300 ease-in-out"
    style="width: {Math.max(0, Math.min(100, value))}%"
  ></div>
</div>