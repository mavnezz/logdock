<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { problemMessage } from '@/shared/i18n'
import type { Project } from '@/shared/api/types'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { FormField } from '@/shared/ui/page'
import { Switch } from '@/shared/ui/switch'

import { useServerRetention, useTestMail, useUpdateProject } from './api'

const props = defineProps<{ project: Project }>()
const { t } = useI18n()
const update = useUpdateProject(() => props.project.id)
const testMail = useTestMail(() => props.project.id)
const server = useServerRetention()
const KINDS = ['logs', 'events', 'traces'] as const
type Kept = Record<(typeof KINDS)[number], string>

const lines = (values: string[] | null | undefined) => (values ?? []).join('\n')
const list = (text: string) =>
  text
    .split(/[\n,]/)
    .map((v) => v.trim())
    .filter(Boolean)

/** Milliseconds as typed; empty is off. */
const millis = (text: string) => {
  const value = Number.parseInt(text, 10)
  return Number.isFinite(value) && value > 0 ? value : null
}
const msText = (value: number | null | undefined) => (value ? String(value) : '')

/** `GET /orders/{order} = 500`, one per line. */
function thresholds(text: string) {
  return text
    .split('\n')
    .map((line) => {
      const at = line.lastIndexOf('=')
      if (at < 0) return null
      const ms = Number.parseInt(line.slice(at + 1), 10)
      const name = line.slice(0, at).trim()
      return name && Number.isFinite(ms) && ms >= 0 ? { name, ms } : null
    })
    .filter((t): t is { name: string; ms: number } => t !== null)
}

/** Days as typed; empty keeps the server's. */
const days = (text: string) => {
  const value = Number.parseInt(text, 10)
  return Number.isFinite(value) ? value : null
}
const daysText = (value: number | null | undefined) => (value == null ? '' : String(value))

function fromProject(project: Project) {
  const s = project.settings
  const p = s.performance
  const kept = (environment: number): Kept => {
    const own = (s.retention ?? []).find((r) => r.environment_id === environment)
    return {
      logs: daysText(own?.logs_days),
      events: daysText(own?.events_days),
      traces: daysText(own?.traces_days),
    }
  }
  return {
    name: project.name,
    notify: lines(s.notify),
    notifyNew: s.notify_new_issues,
    notifyRegressions: s.notify_regressions,
    notifyMonitors: s.notify_monitors,
    extraKeys: lines(s.redaction.extra_keys),
    allowOnly: s.redaction.allowed_keys != null,
    allowedKeys: lines(s.redaction.allowed_keys),
    scrubEmails: s.redaction.scrub_emails,
    scrubIps: s.redaction.scrub_ip_addresses,
    requestMs: msText(p?.request_ms),
    jobMs: msText(p?.job_ms),
    slowQueryMs: msText(p?.slow_query_ms),
    nPlusOne: p?.n_plus_one ?? true,
    overrides: (p?.overrides ?? []).map((o) => `${o.name} = ${o.ms}`).join('\n'),
    retention: Object.fromEntries(project.environments.map((e) => [e.id, kept(e.id)])) as Record<
      number,
      Kept
    >,
  }
}

const form = reactive(fromProject(props.project))
watch(
  () => props.project,
  (project) => Object.assign(form, fromProject(project)),
)
const saved = ref(false)

async function save(): Promise<void> {
  saved.value = false
  await update.mutateAsync({
    name: form.name,
    settings: {
      notify: list(form.notify),
      notify_new_issues: form.notifyNew,
      notify_regressions: form.notifyRegressions,
      notify_monitors: form.notifyMonitors,
      redaction: {
        extra_keys: list(form.extraKeys),
        allowed_keys: form.allowOnly ? list(form.allowedKeys) : null,
        scrub_emails: form.scrubEmails,
        scrub_ip_addresses: form.scrubIps,
      },
      performance: {
        request_ms: millis(form.requestMs),
        job_ms: millis(form.jobMs),
        slow_query_ms: millis(form.slowQueryMs),
        n_plus_one: form.nPlusOne,
        overrides: thresholds(form.overrides),
      },
      retention: props.project.environments
        .map((e) => {
          const kept = form.retention[e.id]
          return {
            environment_id: e.id,
            logs_days: kept ? days(kept.logs) : null,
            events_days: kept ? days(kept.events) : null,
            traces_days: kept ? days(kept.traces) : null,
          }
        })
        .filter((r) => r.logs_days != null || r.events_days != null || r.traces_days != null),
    },
  })
  saved.value = true
}
</script>

<template>
  <form class="grid max-w-2xl gap-6" @submit.prevent="save().catch(() => undefined)">
    <FormField :label="t('projects.name')" for="settings-name">
      <Input id="settings-name" v-model="form.name" required />
    </FormField>

    <fieldset class="grid gap-3">
      <legend class="mb-2 font-medium">{{ t('settings.notifications') }}</legend>
      <FormField
        :label="t('settings.notify')"
        for="settings-notify"
        :hint="t('settings.notifyHint')"
      >
        <textarea
          id="settings-notify"
          v-model="form.notify"
          rows="3"
          class="w-full rounded-md border bg-transparent px-2.5 py-1.5 font-mono text-sm shadow-xs dark:bg-input/30"
        />
      </FormField>
      <label class="flex items-center gap-3 text-sm"
        ><Switch v-model="form.notifyNew" />{{ t('settings.notifyNew') }}</label
      >
      <label class="flex items-center gap-3 text-sm">
        <Switch v-model="form.notifyRegressions" />{{ t('settings.notifyRegressions') }}
      </label>
      <label class="flex items-center gap-3 text-sm"
        ><Switch v-model="form.notifyMonitors" />{{ t('settings.notifyMonitors') }}</label
      >
      <div class="flex flex-wrap items-center gap-3">
        <Button
          type="button"
          variant="outline"
          size="sm"
          :disabled="testMail.isPending.value"
          :title="t('settings.testMailHint')"
          @click="testMail.mutate()"
          >{{ t('settings.testMail') }}</Button
        >
        <span v-if="testMail.data.value" class="text-sm text-muted-foreground">{{
          t('settings.testMailSent', { count: testMail.data.value.recipients })
        }}</span>
        <span v-if="testMail.error.value" class="text-sm text-destructive">{{
          problemMessage(testMail.error.value)
        }}</span>
      </div>
    </fieldset>

    <fieldset class="grid gap-3">
      <legend class="mb-1 font-medium">{{ t('settings.redaction') }}</legend>
      <p class="text-sm text-muted-foreground">{{ t('settings.redactionHint') }}</p>
      <FormField
        :label="t('settings.extraKeys')"
        for="settings-extra"
        :hint="t('settings.extraKeysHint')"
      >
        <textarea
          id="settings-extra"
          v-model="form.extraKeys"
          rows="2"
          class="w-full rounded-md border bg-transparent px-2.5 py-1.5 font-mono text-sm shadow-xs dark:bg-input/30"
        />
      </FormField>
      <label class="flex items-center gap-3 text-sm"
        ><Switch v-model="form.allowOnly" />{{ t('settings.allowOnly') }}</label
      >
      <FormField
        v-if="form.allowOnly"
        :label="t('settings.allowedKeys')"
        for="settings-allowed"
        :hint="t('settings.allowedKeysHint')"
      >
        <textarea
          id="settings-allowed"
          v-model="form.allowedKeys"
          rows="3"
          class="w-full rounded-md border bg-transparent px-2.5 py-1.5 font-mono text-sm shadow-xs dark:bg-input/30"
        />
      </FormField>
      <label class="flex items-center gap-3 text-sm"
        ><Switch v-model="form.scrubEmails" />{{ t('settings.scrubEmails') }}</label
      >
      <label class="flex items-center gap-3 text-sm"
        ><Switch v-model="form.scrubIps" />{{ t('settings.scrubIps') }}</label
      >
    </fieldset>

    <fieldset class="grid gap-3">
      <legend class="mb-1 font-medium">{{ t('settings.performance') }}</legend>
      <p class="text-sm text-muted-foreground">{{ t('settings.performanceHint') }}</p>
      <div class="grid gap-3 sm:grid-cols-3">
        <FormField :label="t('settings.requestMs')" for="settings-request-ms">
          <Input
            id="settings-request-ms"
            v-model="form.requestMs"
            inputmode="numeric"
            placeholder="–"
          />
        </FormField>
        <FormField :label="t('settings.jobMs')" for="settings-job-ms">
          <Input id="settings-job-ms" v-model="form.jobMs" inputmode="numeric" placeholder="–" />
        </FormField>
        <FormField :label="t('settings.slowQueryMs')" for="settings-query-ms">
          <Input
            id="settings-query-ms"
            v-model="form.slowQueryMs"
            inputmode="numeric"
            placeholder="–"
          />
        </FormField>
      </div>
      <FormField
        :label="t('settings.overrides')"
        for="settings-overrides"
        :hint="t('settings.overridesHint')"
      >
        <textarea
          id="settings-overrides"
          v-model="form.overrides"
          rows="3"
          placeholder="POST /payments = 5000"
          class="w-full rounded-md border bg-transparent px-2.5 py-1.5 font-mono text-sm shadow-xs dark:bg-input/30"
        />
      </FormField>
      <label class="flex items-center gap-3 text-sm"
        ><Switch v-model="form.nPlusOne" />{{ t('settings.nPlusOne') }}</label
      >
    </fieldset>

    <fieldset class="grid gap-3">
      <legend class="mb-1 font-medium">{{ t('settings.retention') }}</legend>
      <p class="text-sm text-muted-foreground">{{ t('settings.retentionHint') }}</p>
      <div class="overflow-x-auto">
        <table class="w-full text-sm" :aria-label="t('settings.retention')">
          <thead class="text-xs text-muted-foreground">
            <tr>
              <th class="py-1 pr-3 text-left font-medium">{{ t('storage.stage') }}</th>
              <th v-for="kind in KINDS" :key="kind" class="px-1 py-1 text-left font-medium">
                {{ t(`storage.kinds.${kind}`) }}
              </th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="environment in project.environments" :key="environment.id">
              <td class="py-1 pr-3 font-mono">{{ environment.name }}</td>
              <td v-for="kind in KINDS" :key="kind" class="px-1 py-1">
                <Input
                  v-if="form.retention[environment.id]"
                  v-model="form.retention[environment.id]![kind]"
                  class="w-24"
                  inputmode="numeric"
                  :aria-label="`${environment.name}: ${t(`storage.kinds.${kind}`)}`"
                  :placeholder="
                    server.data.value?.[kind] != null
                      ? t('storage.days', { n: server.data.value[kind] }, server.data.value[kind]!)
                      : ''
                  "
                />
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </fieldset>

    <Alert v-if="update.error.value" variant="destructive">{{
      problemMessage(update.error.value)
    }}</Alert>
    <Alert v-if="saved">{{ t('settings.saved') }}</Alert>
    <div>
      <Button type="submit" :disabled="update.isPending.value">{{ t('common.save') }}</Button>
    </div>
  </form>
</template>
