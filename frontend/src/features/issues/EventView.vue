<script setup lang="ts">
import { Link2 } from 'lucide-vue-next'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'

import { formatDateTime, formatTime } from '@/shared/lib/format'
import { Badge } from '@/shared/ui/badge'
import { buttonVariants } from '@/shared/ui/button'
import { LevelBadge } from '@/shared/ui/level'

import type { ErrorEvent } from './api'
import StackTrace from './StackTrace.vue'

const props = defineProps<{ event: ErrorEvent }>()
const { t } = useI18n()

/** The one that was thrown first, then what caused it. */
const exceptions = computed(() => [...props.event.exceptions].reverse())
const facts = computed(() =>
  [
    ['issues.fields.time', formatDateTime(props.event.time)],
    ['issues.fields.release', props.event.release],
    ['issues.fields.environment', props.event.environment],
    ['issues.fields.transaction', props.event.transaction],
    ['issues.fields.server', props.event.server_name],
    ['issues.fields.logger', props.event.logger],
    ['issues.fields.sdk', props.event.sdk],
    ['issues.fields.event', props.event.event_id],
  ].filter((fact): fact is [string, string] => Boolean(fact[1])),
)
const user = computed(() => {
  const u = props.event.user
  if (!u) return []
  return [u.id, u.email, u.username, u.ip_address].filter(Boolean) as string[]
})
const contexts = computed(() =>
  Object.entries(props.event.contexts).filter(([key]) => key !== 'trace'),
)

function show(value: unknown): string {
  if (value && typeof value === 'object') {
    return Object.entries(value as Record<string, unknown>)
      .map(([k, v]) => `${k}: ${typeof v === 'string' ? v : JSON.stringify(v)}`)
      .join(', ')
  }
  return typeof value === 'string' ? value : JSON.stringify(value)
}
</script>

<!-- One error event: what was thrown where, what led to it, and what it ran in. -->
<template>
  <div class="grid gap-6">
    <section v-if="exceptions.length" class="grid gap-4">
      <div v-for="(exception, i) in exceptions" :key="i" class="grid gap-2">
        <h3 class="flex flex-wrap items-baseline gap-2">
          <span class="font-mono text-sm font-semibold break-all">{{ exception.type }}</span>
          <Badge v-if="i > 0" variant="outline">{{ t('issues.causedBy') }}</Badge>
          <Badge v-if="exception.handled === false" variant="destructive">{{
            t('issues.unhandled')
          }}</Badge>
        </h3>
        <p class="font-mono text-sm break-words whitespace-pre-wrap">{{ exception.value }}</p>
        <StackTrace v-if="exception.frames.length" :frames="exception.frames" />
      </div>
    </section>
    <section v-else class="grid gap-2">
      <p class="font-mono text-sm break-words whitespace-pre-wrap">
        {{ event.message || t('issues.noMessage') }}
      </p>
      <StackTrace v-if="event.frames.length" :frames="event.frames" />
    </section>

    <section class="grid gap-2">
      <h3 class="text-sm font-medium">{{ t('issues.details') }}</h3>
      <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <template v-for="[key, value] in facts" :key="key">
          <dt class="text-muted-foreground">{{ t(key) }}</dt>
          <dd class="font-mono text-xs break-all">{{ value }}</dd>
        </template>
        <template v-if="user.length">
          <dt class="text-muted-foreground">{{ t('issues.fields.user') }}</dt>
          <dd class="font-mono text-xs break-all">{{ user.join(' · ') }}</dd>
        </template>
        <template v-if="event.trace_id">
          <dt class="text-muted-foreground">{{ t('issues.fields.trace') }}</dt>
          <dd class="flex flex-wrap items-center gap-2 font-mono text-xs break-all">
            {{ event.trace_id }}
            <RouterLink
              :to="{ path: '/logs', query: { trace: event.trace_id } }"
              :class="buttonVariants({ variant: 'outline', size: 'sm' })"
              ><Link2 />{{ t('issues.traceLogs') }}</RouterLink
            >
            <RouterLink
              :to="`/traces/${event.trace_id}`"
              :class="buttonVariants({ variant: 'outline', size: 'sm' })"
              >{{ t('issues.traceView') }}</RouterLink
            >
          </dd>
        </template>
      </dl>
    </section>

    <section v-if="event.tags.length" class="grid gap-2">
      <h3 class="text-sm font-medium">{{ t('issues.tags') }}</h3>
      <ul class="flex flex-wrap gap-1.5">
        <li
          v-for="[key, value] in event.tags"
          :key="key"
          class="rounded border px-1.5 py-0.5 font-mono text-xs"
        >
          <span class="text-muted-foreground">{{ key }}:</span> {{ value }}
        </li>
      </ul>
    </section>

    <section v-if="event.request" class="grid gap-2">
      <h3 class="text-sm font-medium">{{ t('issues.request') }}</h3>
      <p class="font-mono text-xs break-all">
        <span class="font-semibold">{{ event.request.method }}</span> {{ event.request.url
        }}<template v-if="event.request.query">?{{ event.request.query }}</template>
      </p>
      <table v-if="event.request.headers.length" class="w-full text-xs">
        <tbody>
          <tr
            v-for="[name, value] in event.request.headers"
            :key="name"
            class="border-b last:border-0"
          >
            <td class="w-40 py-1 pr-2 align-top font-mono text-muted-foreground">{{ name }}</td>
            <td class="py-1 font-mono break-all">{{ value }}</td>
          </tr>
        </tbody>
      </table>
      <template v-if="event.request.data !== null && event.request.data !== undefined">
        <h4 class="text-xs font-medium text-muted-foreground">{{ t('issues.body') }}</h4>
        <pre class="max-h-72 overflow-auto rounded-md bg-muted p-3 font-mono text-xs">{{
          typeof event.request.data === 'string'
            ? event.request.data
            : JSON.stringify(event.request.data, null, 2)
        }}</pre>
      </template>
      <p v-if="event.request.cookies.length" class="flex flex-wrap items-center gap-1.5 text-xs">
        <span class="text-muted-foreground">{{ t('issues.cookies') }}</span>
        <code
          v-for="name in event.request.cookies"
          :key="name"
          class="rounded bg-muted px-1.5 py-0.5 font-mono"
          >{{ name }}</code
        >
      </p>
    </section>

    <section v-if="event.breadcrumbs.length" class="grid gap-2">
      <h3 class="text-sm font-medium">{{ t('issues.breadcrumbs') }}</h3>
      <ol class="divide-y rounded-md border text-xs">
        <li
          v-for="(crumb, i) in event.breadcrumbs"
          :key="i"
          class="grid grid-cols-[5.5rem_minmax(0,1fr)] gap-2 px-3 py-1.5 sm:grid-cols-[5.5rem_9rem_minmax(0,1fr)]"
        >
          <time v-if="crumb.time" :datetime="crumb.time" class="font-mono text-muted-foreground">{{
            formatTime(crumb.time)
          }}</time>
          <span v-else />
          <span class="hidden items-center gap-1.5 truncate sm:flex">
            <LevelBadge v-if="crumb.level" :level="crumb.level" />
            <span class="truncate text-muted-foreground">{{ crumb.category ?? crumb.type }}</span>
          </span>
          <span class="font-mono break-words">{{ crumb.message ?? show(crumb.data) }}</span>
        </li>
      </ol>
    </section>

    <section v-if="contexts.length" class="grid gap-2">
      <h3 class="text-sm font-medium">{{ t('issues.contexts') }}</h3>
      <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
        <template v-for="[key, value] in contexts" :key="key">
          <dt class="text-muted-foreground">{{ key }}</dt>
          <dd class="font-mono break-all">{{ show(value) }}</dd>
        </template>
      </dl>
    </section>
  </div>
</template>
