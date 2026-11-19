<script setup lang="ts">
import { ChevronRight } from 'lucide-vue-next'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import type { StackFrame } from './api'

const props = defineProps<{ frames: StackFrame[] }>()
const { t } = useI18n()

const hasAppFrames = computed(() => props.frames.some((f) => f.in_app))
const appOnly = ref(true)
/** Where it was thrown first; the frames come oldest first. */
const shown = computed(() =>
  props.frames
    .map((frame, index) => ({ frame, index }))
    .filter(({ frame }) => !appOnly.value || !hasAppFrames.value || frame.in_app)
    .reverse(),
)
const open = ref(new Set<number>())
watch(
  () => props.frames,
  (frames) => {
    // The frame where it was thrown in the app's own code opens by itself.
    let thrown = -1
    frames.forEach((f, index) => {
      if (f.in_app && f.context_line) thrown = index
    })
    open.value = new Set(thrown >= 0 ? [thrown] : [])
  },
  { immediate: true },
)
function toggle(index: number): void {
  const next = new Set(open.value)
  if (next.has(index)) next.delete(index)
  else next.add(index)
  open.value = next
}
const hasContext = (frame: StackFrame) => Boolean(frame.context_line)
</script>

<!-- Frames of a stack, where it was thrown on top; the app's own frames stand out. -->
<template>
  <div class="grid gap-2">
    <label v-if="hasAppFrames" class="flex items-center gap-2 text-xs text-muted-foreground">
      <input v-model="appOnly" type="checkbox" class="accent-primary" />
      {{ t('issues.appFramesOnly') }}
    </label>
    <ol class="divide-y overflow-hidden rounded-md border font-mono text-xs">
      <li v-for="{ frame, index } in shown" :key="index" :class="!frame.in_app && 'bg-muted/40'">
        <button
          type="button"
          class="flex w-full items-start gap-2 px-3 py-1.5 text-left"
          :class="hasContext(frame) ? 'hover:bg-accent' : 'cursor-default'"
          :aria-expanded="hasContext(frame) ? open.has(index) : undefined"
          @click="hasContext(frame) && toggle(index)"
        >
          <ChevronRight
            class="mt-0.5 size-3 shrink-0 transition-transform"
            :class="[open.has(index) && 'rotate-90', !hasContext(frame) && 'invisible']"
          />
          <span class="grid min-w-0 gap-0.5">
            <span
              class="break-all"
              :class="frame.in_app ? 'font-semibold' : 'text-muted-foreground'"
              >{{ frame.function || frame.module || t('issues.anonymous') }}</span
            >
            <span class="break-all text-muted-foreground">
              {{ frame.filename || frame.abs_path || '?'
              }}<template v-if="frame.lineno">:{{ frame.lineno }}</template>
            </span>
          </span>
        </button>
        <pre
          v-if="open.has(index) && hasContext(frame)"
          class="overflow-x-auto border-t bg-muted/30 py-1 text-[11px] leading-5"
        ><template v-for="(line, i) in frame.pre_context" :key="`pre${i}`"><span class="block px-3 text-muted-foreground"><span class="inline-block w-10 text-right select-none">{{ (frame.lineno ?? 0) - frame.pre_context.length + i }}</span>  {{ line }}</span></template><span class="block bg-destructive/10 px-3 font-semibold"><span class="inline-block w-10 text-right select-none">{{ frame.lineno }}</span>  {{ frame.context_line }}</span><template v-for="(line, i) in frame.post_context" :key="`post${i}`"><span class="block px-3 text-muted-foreground"><span class="inline-block w-10 text-right select-none">{{ (frame.lineno ?? 0) + i + 1 }}</span>  {{ line }}</span></template></pre>
      </li>
    </ol>
  </div>
</template>
