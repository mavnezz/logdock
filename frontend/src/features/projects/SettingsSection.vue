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

import { useTestMail, useUpdateProject } from './api'

const props = defineProps<{ project: Project }>()
const { t } = useI18n()
const update = useUpdateProject(() => props.project.id)
const testMail = useTestMail(() => props.project.id)

const lines = (values: string[] | null | undefined) => (values ?? []).join('\n')
const list = (text: string) =>
  text
    .split(/[\n,]/)
    .map((v) => v.trim())
    .filter(Boolean)

function fromProject(project: Project) {
  const s = project.settings
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

    <Alert v-if="update.error.value" variant="destructive">{{
      problemMessage(update.error.value)
    }}</Alert>
    <Alert v-if="saved">{{ t('settings.saved') }}</Alert>
    <div>
      <Button type="submit" :disabled="update.isPending.value">{{ t('common.save') }}</Button>
    </div>
  </form>
</template>
