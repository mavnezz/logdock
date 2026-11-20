<script setup lang="ts">
import { Search } from 'lucide-vue-next'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { type LocationQuery, RouterLink, useRoute, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatNumber, formatRelative } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import ScopePicker from '@/shared/scope/ScopePicker.vue'
import { useScopeStore } from '@/shared/scope/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { LevelBadge } from '@/shared/ui/level'
import { NativeSelect } from '@/shared/ui/native-select'
import { EmptyState, PageHeader } from '@/shared/ui/page'
import { Pager } from '@/shared/ui/pager'

import { type IssueFilters, SORTS, type Sort, STATUSES, type StatusFilter, useIssues } from './api'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const scope = useScopeStore()
const names = useProjectNames()

function one(query: LocationQuery, key: string): string {
  const value = query[key]
  return (Array.isArray(value) ? value[0] : value) ?? ''
}
function pick<T extends string>(value: string, allowed: readonly T[], fallback: T): T {
  return (allowed as readonly string[]).includes(value) ? (value as T) : fallback
}

const status = computed(() =>
  pick<StatusFilter>(one(route.query, 'status'), STATUSES, 'unresolved'),
)
const sort = computed(() => pick<Sort>(one(route.query, 'sort'), SORTS, 'last_seen'))
const q = computed(() => one(route.query, 'q'))
const release = computed(() => one(route.query, 'release'))
const page = computed(() => Math.max(1, Number.parseInt(one(route.query, 'page'), 10) || 1))
const typed = ref(q.value)
watch(q, (value) => (typed.value = value))

function update(changes: Record<string, string>): void {
  const query: Record<string, string> = {
    status: status.value,
    sort: sort.value,
    q: q.value,
    release: release.value,
    // Another filter starts from the first page.
    page: 'page' in changes ? (changes.page ?? '1') : '1',
    ...changes,
  }
  if (query.page === '1') delete query.page
  if (query.status === 'unresolved') delete query.status
  if (query.sort === 'last_seen') delete query.sort
  if (!query.q) delete query.q
  if (!query.release) delete query.release
  void router.replace({ query })
}

const filters = computed<IssueFilters>(() => ({
  project: scope.project,
  environment: scope.environment,
  status: status.value,
  sort: sort.value,
  q: q.value,
  release: release.value,
  page: page.value,
}))
const issues = useIssues(filters)
const list = computed(() => issues.data.value?.items ?? [])
const events = (n: number) => t('issues.eventCount', { n: formatNumber(n) }, n)
const several = computed(() => scope.project === null)
</script>

<template>
  <div class="grid content-start gap-4">
    <PageHeader :title="t('issues.title')" :description="t('issues.description')">
      <ScopePicker />
    </PageHeader>

    <form class="flex flex-wrap items-center gap-2" @submit.prevent="update({ q: typed })">
      <div class="relative min-w-56 flex-1">
        <Search
          class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          v-model="typed"
          class="pl-8"
          :placeholder="t('issues.searchHint')"
          :aria-label="t('issues.search')"
        />
      </div>
      <NativeSelect
        :model-value="status"
        class="w-40"
        :aria-label="t('issues.status')"
        @update:model-value="(value) => update({ status: String(value) })"
      >
        <option v-for="option in STATUSES" :key="option" :value="option">
          {{ t(`issues.statuses.${option}`) }}
        </option>
      </NativeSelect>
      <NativeSelect
        :model-value="sort"
        class="w-44"
        :aria-label="t('issues.sort')"
        @update:model-value="(value) => update({ sort: String(value) })"
      >
        <option v-for="option in SORTS" :key="option" :value="option">
          {{ t(`issues.sorts.${option}`) }}
        </option>
      </NativeSelect>
      <Button type="submit" variant="outline">{{ t('logs.searchButton') }}</Button>
    </form>
    <p v-if="release" class="flex items-center gap-2 text-sm">
      <Badge variant="secondary" class="font-mono">{{ t('issues.newIn', { release }) }}</Badge>
      <Button size="sm" variant="ghost" @click="update({ release: '' })">{{
        t('issues.clearRelease')
      }}</Button>
    </p>

    <Alert v-if="issues.error.value" variant="destructive">{{
      problemMessage(issues.error.value)
    }}</Alert>
    <p v-if="issues.isPending.value" class="text-sm text-muted-foreground">
      {{ t('common.loading') }}
    </p>
    <EmptyState
      v-else-if="list.length === 0 && page === 1"
      :title="t('issues.empty')"
      :description="status === 'unresolved' ? t('issues.emptyHint') : undefined"
    />
    <div v-else class="overflow-hidden rounded-lg border">
      <div
        class="hidden grid-cols-[minmax(0,1fr)_5rem_5rem_8rem] gap-3 border-b bg-muted/40 px-4 py-2 text-xs font-medium text-muted-foreground md:grid"
        aria-hidden="true"
      >
        <span>{{ t('issues.issue') }}</span>
        <span class="text-right">{{ t('issues.events') }}</span>
        <span class="text-right">{{ t('issues.lastDay') }}</span>
        <span class="text-right">{{ t('issues.lastSeen') }}</span>
      </div>
      <ul class="divide-y" :aria-label="t('issues.list')">
        <li
          v-for="issue in list"
          :key="issue.id"
          class="grid gap-1 px-4 py-3 hover:bg-accent/50 md:grid-cols-[minmax(0,1fr)_5rem_5rem_8rem] md:items-center md:gap-3"
        >
          <div class="grid min-w-0 gap-1">
            <div class="flex min-w-0 items-center gap-2">
              <LevelBadge :level="issue.level" />
              <RouterLink
                :to="`/issues/${issue.id}`"
                class="truncate font-medium hover:underline"
                :title="issue.title"
                >{{ issue.title }}</RouterLink
              >
            </div>
            <p
              class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground"
            >
              <span v-if="issue.culprit" class="truncate font-mono">{{ issue.culprit }}</span>
              <span v-if="several">{{ names.project(issue.project_id) }}</span>
              <span>{{ names.environment(issue.environment_id) }}</span>
              <Badge v-if="issue.status === 'resolved'" variant="success">{{
                t('issues.statuses.resolved')
              }}</Badge>
              <Badge v-if="issue.status === 'muted'" variant="secondary">{{
                t('issues.statuses.muted')
              }}</Badge>
              <Badge v-if="issue.regressed_at && issue.status === 'unresolved'" variant="warning">{{
                t('issues.regressed')
              }}</Badge>
              <span class="md:hidden">
                · {{ events(issue.count) }} ·
                {{ formatRelative(issue.last_seen) }}
              </span>
            </p>
          </div>
          <span class="hidden text-right text-sm tabular-nums md:block">{{
            formatNumber(issue.count)
          }}</span>
          <span class="hidden text-right text-sm tabular-nums md:block">{{
            formatNumber(issue.last_day)
          }}</span>
          <time
            class="hidden text-right text-sm text-muted-foreground md:block"
            :datetime="issue.last_seen"
            :title="formatDateTime(issue.last_seen)"
            >{{ formatRelative(issue.last_seen) }}</time
          >
        </li>
      </ul>
    </div>
    <Pager
      :page="page"
      :has-next="issues.data.value?.hasNext ?? false"
      @update:page="(next) => update({ page: String(next) })"
    />
  </div>
</template>
