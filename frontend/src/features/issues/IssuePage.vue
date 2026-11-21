<script setup lang="ts">
import { ArrowLeft, Check, ChevronLeft, ChevronRight, RotateCcw, Trash2 } from 'lucide-vue-next'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatNumber, formatRelative, formatTime } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import { useSessionStore } from '@/shared/session/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { ConfirmDialog } from '@/shared/ui/confirm-dialog'
import { LevelBadge } from '@/shared/ui/level'
import { NativeSelect } from '@/shared/ui/native-select'
import { RequestTimeline } from '@/shared/trace'

import {
  type ChangeIssue,
  type ErrorEvent,
  MUTES,
  useChangeIssue,
  useDeleteIssue,
  useEvent,
  useIssue,
  useIssueEvents,
  useIssueTags,
  useTrace,
  tagSearch,
} from './api'
import EventView from './EventView.vue'
import IssueHours from './IssueHours.vue'

const props = defineProps<{ id: string }>()
const i18n = useI18n()
const { t } = i18n
const router = useRouter()
const session = useSessionStore()
const names = useProjectNames()

const id = computed(() => Number(props.id))
const detail = useIssue(id)
const events = useIssueEvents(id)
const eventItems = computed(() => events.data.value?.pages.flat() ?? [])
const change = useChangeIssue(id)
const remove = useDeleteIssue(id)
const issue = computed(() => detail.data.value?.issue)

/** `null` shows the latest event, kept with the issue. */
const chosen = ref<string | null>(null)
watch(id, () => (chosen.value = null))
const picked = useEvent(chosen)
// The query's data comes deeply unwrapped; it is the API's shape all the same.
const event = computed(
  () =>
    (chosen.value === null ? detail.data.value?.latest : picked.data.value?.event) as
      ErrorEvent | null | undefined,
)
const trace = useTrace(computed(() => event.value?.trace_id ?? null))
const tags = useIssueTags(id)
const users = computed(() => detail.data.value?.users ?? 0)
const tagName = (key: string) =>
  i18n.te(`issues.tagKeys.${key}`) ? t(`issues.tagKeys.${key}`) : key
const percent = (count: number, total: number) =>
  total > 0 ? Math.max(1, Math.round((count / total) * 100)) : 0

// Newer and older events than the one shown, from those loaded.
const shownId = computed(() => chosen.value ?? detail.data.value?.latest?.event_id ?? null)
const position = computed(() => eventItems.value.findIndex((e) => e.event_id === shownId.value))
function step(by: number): void {
  const next = eventItems.value[position.value + by]
  if (next) chosen.value = next.event_id
  else if (by > 0 && events.hasNextPage.value) void events.fetchNextPage()
}
const others = computed(() =>
  (trace.data.value?.events ?? []).filter((e) => e.event_id !== event.value?.event_id),
)

const mute = ref('')
async function act(body: ChangeIssue): Promise<void> {
  await change.mutateAsync(body).catch(() => undefined)
}
async function muteFor(key: string): Promise<void> {
  const option = MUTES.find((m) => m.key === key)
  mute.value = ''
  if (option) await act(option.change)
}

const confirmDelete = ref(false)
async function deleteIssue(): Promise<void> {
  await remove.mutateAsync()
  await router.push('/issues')
}

const statusText = computed(() => {
  const i = issue.value
  if (!i) return ''
  if (i.status === 'resolved' && i.resolved_in_next_release) {
    return t('issues.resolvedAfter', { release: i.resolved_in_next_release })
  }
  if (i.status === 'muted') {
    if (i.muted_until) return t('issues.mutedUntil', { time: formatDateTime(i.muted_until) })
    if (i.muted_remaining) return t('issues.mutedFor', { count: i.muted_remaining })
    return t('issues.mutedForever')
  }
  return t(`issues.statuses.${i.status}`)
})
</script>

<template>
  <div class="grid content-start gap-6">
    <RouterLink
      to="/issues"
      class="flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      ><ArrowLeft class="size-4" />{{ t('issues.title') }}</RouterLink
    >
    <Alert v-if="detail.error.value" variant="destructive">{{
      problemMessage(detail.error.value)
    }}</Alert>
    <p v-else-if="!issue" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
    <template v-else>
      <header class="grid gap-3">
        <div class="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
          <LevelBadge :level="issue.level" />
          <Badge v-if="issue.category === 'performance'" variant="outline">{{
            t('issues.performance')
          }}</Badge>
          <span
            >{{ names.project(issue.project_id) }} /
            {{ names.environment(issue.environment_id) }}</span
          >
          <Badge
            :variant="
              issue.status === 'resolved'
                ? 'success'
                : issue.status === 'muted'
                  ? 'secondary'
                  : 'default'
            "
            >{{ statusText }}</Badge
          >
          <Badge v-if="issue.regressed_at && issue.status === 'unresolved'" variant="warning">{{
            t('issues.regressedAt', { time: formatRelative(issue.regressed_at) })
          }}</Badge>
        </div>
        <h1 class="text-xl font-semibold tracking-tight break-words">{{ issue.title }}</h1>
        <p v-if="issue.culprit" class="font-mono text-sm break-all text-muted-foreground">
          {{ issue.culprit }}
        </p>
        <div class="flex flex-wrap items-center gap-2">
          <template v-if="issue.status !== 'resolved'">
            <Button size="sm" :disabled="change.isPending.value" @click="act({ action: 'resolve' })"
              ><Check />{{ t('issues.resolve') }}</Button
            >
            <Button
              v-if="issue.last_release"
              size="sm"
              variant="outline"
              :disabled="change.isPending.value"
              :title="t('issues.resolveNextHint', { release: issue.last_release })"
              @click="act({ action: 'resolve_next_release' })"
              >{{ t('issues.resolveNext') }}</Button
            >
          </template>
          <Button
            v-if="issue.status !== 'unresolved'"
            size="sm"
            variant="outline"
            :disabled="change.isPending.value"
            @click="act({ action: 'reopen' })"
            ><RotateCcw />{{ t('issues.reopen') }}</Button
          >
          <NativeSelect
            v-if="issue.status !== 'muted'"
            v-model="mute"
            class="h-8 w-44"
            :aria-label="t('issues.mute')"
            @update:model-value="(key) => muteFor(String(key))"
          >
            <option value="" disabled>{{ t('issues.mute') }}…</option>
            <option v-for="option in MUTES" :key="option.key" :value="option.key">
              {{ t(`issues.mutes.${option.key}`) }}
            </option>
          </NativeSelect>
          <Button
            v-if="session.isAdmin"
            size="sm"
            variant="ghost"
            class="text-destructive"
            @click="confirmDelete = true"
            ><Trash2 />{{ t('common.delete') }}</Button
          >
        </div>
        <Alert v-if="change.error.value" variant="destructive">{{
          problemMessage(change.error.value)
        }}</Alert>
      </header>

      <dl class="grid grid-cols-2 gap-4 rounded-lg border p-4 text-sm sm:grid-cols-5">
        <div>
          <dt class="text-muted-foreground">{{ t('issues.events') }}</dt>
          <dd class="text-lg font-semibold tabular-nums">{{ formatNumber(issue.count) }}</dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('issues.usersAffected') }}</dt>
          <dd v-if="users" class="text-lg font-semibold tabular-nums">{{ formatNumber(users) }}</dd>
          <dd v-else class="text-xs text-muted-foreground">{{ t('issues.noUsers') }}</dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('issues.firstSeen') }}</dt>
          <dd :title="formatDateTime(issue.first_seen)">{{ formatRelative(issue.first_seen) }}</dd>
          <dd v-if="issue.first_release" class="font-mono text-xs text-muted-foreground">
            {{ issue.first_release }}
          </dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('issues.lastSeen') }}</dt>
          <dd :title="formatDateTime(issue.last_seen)">{{ formatRelative(issue.last_seen) }}</dd>
          <dd v-if="issue.last_release" class="font-mono text-xs text-muted-foreground">
            {{ issue.last_release }}
          </dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('issues.releases') }}</dt>
          <dd class="tabular-nums">{{ formatNumber(detail.data.value?.releases.length ?? 0) }}</dd>
        </div>
      </dl>

      <IssueHours :hours="detail.data.value?.hours ?? []" :level="issue.level" />

      <div class="grid gap-6 lg:grid-cols-[minmax(0,1fr)_18rem]">
        <section class="grid min-w-0 content-start gap-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h2 class="font-medium">
              {{ chosen === null ? t('issues.latestEvent') : t('issues.chosenEvent') }}
            </h2>
            <div class="flex items-center gap-1">
              <Button
                size="sm"
                variant="outline"
                :disabled="position <= 0"
                :aria-label="t('issues.newerEvent')"
                @click="step(-1)"
                ><ChevronLeft />{{ t('issues.newer') }}</Button
              >
              <Button
                size="sm"
                variant="outline"
                :disabled="
                  position < 0 || (position >= eventItems.length - 1 && !events.hasNextPage.value)
                "
                :aria-label="t('issues.olderEvent')"
                @click="step(1)"
                >{{ t('issues.older') }}<ChevronRight
              /></Button>
              <Button v-if="chosen !== null" size="sm" variant="ghost" @click="chosen = null">{{
                t('issues.showLatest')
              }}</Button>
            </div>
          </div>
          <Alert v-if="picked.error.value" variant="destructive">{{
            problemMessage(picked.error.value)
          }}</Alert>
          <EventView v-if="event" :event="event" />
          <p v-else class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>

          <section v-if="event?.trace_id && trace.data.value" class="grid gap-2">
            <h3 class="text-sm font-medium">{{ t('issues.flow') }}</h3>
            <RequestTimeline
              v-if="trace.data.value.transactions.length"
              :transactions="trace.data.value.transactions"
              :focus-span="event.span_id"
              :focus-name="event.transaction"
            />
            <p v-else class="text-sm text-muted-foreground">{{ t('issues.flowMissing') }}</p>
          </section>

          <section v-if="event?.trace_id && trace.data.value" class="grid gap-2">
            <h3 class="text-sm font-medium">{{ t('issues.sameTrace') }}</h3>
            <p
              v-if="trace.data.value.logs.length === 0 && others.length === 0"
              class="text-sm text-muted-foreground"
            >
              {{ t('issues.traceEmpty') }}
            </p>
            <ol
              v-if="trace.data.value.logs.length"
              class="divide-y rounded-md border font-mono text-xs"
              :aria-label="t('issues.traceLogsList')"
            >
              <li
                v-for="entry in trace.data.value.logs"
                :key="entry.id"
                class="grid grid-cols-[6.5rem_4.5rem_minmax(0,1fr)] items-baseline gap-2 px-3 py-1.5"
              >
                <time :datetime="entry.time" class="text-muted-foreground">{{
                  formatTime(entry.time)
                }}</time>
                <LevelBadge :level="entry.level" />
                <span class="truncate" :title="entry.message">{{ entry.message }}</span>
              </li>
            </ol>
            <ul v-if="others.length" class="grid gap-1 text-sm">
              <li v-for="other in others" :key="other.event_id">
                <RouterLink :to="`/issues/${other.issue_id}`" class="hover:underline">{{
                  other.message
                }}</RouterLink>
              </li>
            </ul>
          </section>
        </section>

        <aside class="grid content-start gap-6">
          <section v-if="tags.data.value?.length" class="grid gap-3">
            <h2 class="text-sm font-medium">{{ t('issues.origin') }}</h2>
            <div v-for="tag in tags.data.value" :key="tag.key" class="grid gap-1">
              <h3 class="flex justify-between gap-2 text-xs text-muted-foreground">
                <span>{{ tagName(tag.key) }}</span>
                <span class="tabular-nums">{{
                  tag.complete
                    ? t('issues.distinct', { n: formatNumber(tag.distinct) }, tag.distinct)
                    : t('issues.distinctAtLeast', { n: formatNumber(tag.distinct) })
                }}</span>
              </h3>
              <ul class="grid gap-1 text-xs" :aria-label="tagName(tag.key)">
                <li v-for="value in tag.values" :key="value.value" class="relative">
                  <span
                    class="absolute inset-y-0 left-0 rounded-sm bg-muted"
                    :style="{ width: `${percent(value.count, tag.total)}%` }"
                    aria-hidden="true"
                  />
                  <RouterLink
                    :to="{
                      path: '/issues',
                      query: {
                        q: tagSearch(tag.key, value.value),
                        status: 'all',
                        // The issues of its own kind.
                        ...(issue.category === 'error' ? {} : { category: issue.category }),
                      },
                    }"
                    class="relative flex justify-between gap-2 rounded-sm px-1.5 py-0.5 hover:underline"
                    :title="t('issues.tagIssues', { value: value.value })"
                  >
                    <span class="truncate font-mono">{{ value.value }}</span>
                    <span class="shrink-0 tabular-nums text-muted-foreground"
                      >{{ percent(value.count, tag.total) }} %</span
                    >
                  </RouterLink>
                </li>
              </ul>
            </div>
          </section>
          <section class="grid gap-2">
            <h2 class="text-sm font-medium">{{ t('issues.recentEvents') }}</h2>
            <ol class="divide-y rounded-md border text-xs">
              <li v-for="item in eventItems" :key="item.event_id">
                <button
                  type="button"
                  class="grid w-full gap-0.5 px-3 py-1.5 text-left hover:bg-accent"
                  :class="
                    (chosen ?? detail.data.value?.latest?.event_id) === item.event_id && 'bg-accent'
                  "
                  @click="chosen = item.event_id"
                >
                  <span :title="formatDateTime(item.time)">{{ formatRelative(item.time) }}</span>
                  <span class="truncate text-muted-foreground">
                    {{ [item.release, item.service, item.host].filter(Boolean).join(' · ') || '–' }}
                  </span>
                </button>
              </li>
            </ol>
            <Button
              v-if="events.hasNextPage.value"
              variant="ghost"
              size="sm"
              :disabled="events.isFetchingNextPage.value"
              @click="events.fetchNextPage()"
              >{{ t('common.older') }}</Button
            >
            <p
              v-if="events.data.value && eventItems.length === 0"
              class="text-xs text-muted-foreground"
            >
              {{ t('issues.eventsGone') }}
            </p>
          </section>
          <section v-if="detail.data.value?.releases.length" class="grid gap-2">
            <h2 class="text-sm font-medium">{{ t('issues.releases') }}</h2>
            <table class="w-full text-xs">
              <tbody>
                <tr
                  v-for="release in detail.data.value.releases"
                  :key="release.release"
                  class="border-b last:border-0"
                >
                  <td class="py-1 pr-2 font-mono break-all">{{ release.release }}</td>
                  <td class="py-1 text-right tabular-nums">{{ formatNumber(release.count) }}</td>
                  <td class="py-1 pl-2 text-right text-muted-foreground">
                    {{ formatRelative(release.last_seen) }}
                  </td>
                </tr>
              </tbody>
            </table>
          </section>
        </aside>
      </div>

      <ConfirmDialog
        v-model:open="confirmDelete"
        :title="t('issues.deleteTitle')"
        :description="t('issues.deleteDescription')"
        :confirm-label="t('common.delete')"
        :cancel-label="t('common.cancel')"
        destructive
        @confirm="deleteIssue"
      />
    </template>
  </div>
</template>
