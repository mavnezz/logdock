<script setup lang="ts">
import { Label } from '@/shared/ui/label'

defineProps<{ label: string; for: string; hint?: string; error?: string }>()
</script>

<!-- A labelled input with its hint or error below. -->
<template>
  <div class="grid gap-1.5">
    <Label :for="$props.for">{{ label }}</Label>
    <slot />
    <p v-if="error" class="text-xs text-destructive">{{ error }}</p>
    <p v-else-if="hint" class="text-xs text-muted-foreground">{{ hint }}</p>
  </div>
</template>
