<script setup lang="ts">
import { Copy, Link2, X } from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatDateTime } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { LevelBadge } from '@/shared/ui/level'

import type { LogEntry } from './api'

const props = defineProps<{ entry: LogEntry }>()
const emit = defineEmits<{ close: []; trace: [traceId: string]; filter: [term: string] }>()
const { t } = useI18n()
const names = useProjectNames()
const copied = ref(false)

const fields = computed(() =>
  [
    ['logs.fields.time', formatDateTime(props.entry.time) + ` (${props.entry.time})`],
    ['logs.fields.received', formatDateTime(props.entry.received)],
    [
      'logs.fields.project',
      `${names.project(props.entry.project_id)} / ${names.environment(props.entry.environment_id)}`,
    ],
    ['logs.fields.service', props.entry.service],
    ['logs.fields.host', props.entry.host],
    ['logs.fields.container', props.entry.container],
    ['logs.fields.release', props.entry.release],
    ['logs.fields.trace', props.entry.trace_id],
    ['logs.fields.user', props.entry.user_id],
  ].filter((field): field is [string, string] => Boolean(field[1])),
)
const attributes = computed(() => Object.entries(props.entry.attributes as Record<string, unknown>))

function show(value: unknown): string {
  return typeof value === 'string' ? value : JSON.stringify(value)
}

async function copy(): Promise<void> {
  await navigator.clipboard.writeText(JSON.stringify(props.entry, null, 2))
  copied.value = true
  setTimeout(() => (copied.value = false), 1500)
}
</script>

<!-- One entry with everything it carries. -->
<template>
  <aside
    class="grid content-start gap-4 rounded-xl bg-card p-4 shadow-panel"
    :aria-label="t('logs.detail')"
  >
    <div class="flex items-start justify-between gap-2">
      <LevelBadge :level="entry.level" />
      <button
        type="button"
        class="text-muted-foreground hover:text-foreground"
        :aria-label="t('common.close')"
        @click="emit('close')"
      >
        <X class="size-4" />
      </button>
    </div>
    <p class="font-mono text-sm break-words whitespace-pre-wrap">{{ entry.message }}</p>
    <p
      v-if="entry.template && entry.template !== entry.message"
      class="font-mono text-xs text-muted-foreground"
    >
      {{ entry.template }}
    </p>
    <div class="flex flex-wrap gap-1">
      <Badge v-if="entry.redactions > 0" variant="warning">{{
        t('logs.redacted', { count: entry.redactions })
      }}</Badge>
      <Badge v-if="entry.truncated" variant="outline">{{ t('logs.truncated') }}</Badge>
    </div>
    <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
      <template v-for="[key, value] in fields" :key="key">
        <dt class="text-muted-foreground">{{ t(key) }}</dt>
        <dd class="font-mono text-xs break-all">{{ value }}</dd>
      </template>
    </dl>
    <div v-if="attributes.length" class="grid gap-1">
      <h3 class="text-sm font-medium">{{ t('logs.attributes') }}</h3>
      <table class="w-full text-xs">
        <tbody>
          <tr v-for="[key, value] in attributes" :key="key" class="border-b last:border-0">
            <td class="py-1 pr-2 align-top font-mono text-muted-foreground">
              <button
                type="button"
                class="hover:text-foreground hover:underline"
                @click="emit('filter', `${key}:${show(value)}`)"
              >
                {{ key }}
              </button>
            </td>
            <td class="py-1 font-mono break-all">{{ show(value) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <div class="flex flex-wrap gap-2">
      <Button
        v-if="entry.trace_id"
        size="sm"
        variant="outline"
        @click="emit('trace', entry.trace_id)"
      >
        <Link2 />{{ t('logs.sameTrace') }}
      </Button>
      <Button size="sm" variant="ghost" @click="copy"
        ><Copy />{{ copied ? t('keys.copied') : t('logs.copyJson') }}</Button
      >
    </div>
  </aside>
</template>
