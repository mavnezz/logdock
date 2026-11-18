<script setup lang="ts">
import { Plus, Trash2 } from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { useSessionStore } from '@/shared/session/store'
import { problemMessage } from '@/shared/i18n'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { ConfirmDialog } from '@/shared/ui/confirm-dialog'
import { Input } from '@/shared/ui/input'
import { PageHeader } from '@/shared/ui/page'

import { useAddEnvironment, useDeleteProject, useProject, useRemoveEnvironment } from './api'
import IngestSection from './IngestSection.vue'
import KeysSection from './KeysSection.vue'
import SettingsSection from './SettingsSection.vue'

const props = defineProps<{ id: string }>()
const { t } = useI18n()
const router = useRouter()
const store = useSessionStore()
const projectId = computed(() => Number(props.id))
const project = useProject(projectId)
const addEnvironment = useAddEnvironment(projectId)
const removeEnvironment = useRemoveEnvironment(projectId)
const deleteProject = useDeleteProject()

const tab = ref<'overview' | 'keys' | 'settings'>('overview')
const newEnvironment = ref('')
const deleting = ref(false)

async function add(): Promise<void> {
  await addEnvironment.mutateAsync(newEnvironment.value)
  newEnvironment.value = ''
}

async function remove(): Promise<void> {
  await deleteProject.mutateAsync(projectId.value)
  await router.push('/projects')
}

const tabs = computed(() =>
  store.isAdmin ? (['overview', 'keys', 'settings'] as const) : (['overview'] as const),
)
</script>

<template>
  <div class="grid gap-6">
    <p v-if="project.isPending.value" class="text-sm text-muted-foreground">
      {{ t('common.loading') }}
    </p>
    <Alert v-else-if="project.error.value" variant="destructive">{{
      problemMessage(project.error.value)
    }}</Alert>
    <template v-else-if="project.data.value">
      <PageHeader :title="project.data.value.name" :description="t('projects.detailDescription')">
        <Button v-if="store.isAdmin" variant="outline" @click="deleting = true"
          ><Trash2 />{{ t('common.delete') }}</Button
        >
      </PageHeader>

      <nav class="flex gap-1 border-b">
        <button
          v-for="name in tabs"
          :key="name"
          type="button"
          class="-mb-px border-b-2 px-3 py-2 text-sm"
          :class="
            tab === name
              ? 'border-primary font-medium'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          "
          @click="tab = name"
        >
          {{ t(`projects.tabs.${name}`) }}
        </button>
      </nav>

      <template v-if="tab === 'overview'">
        <section class="grid gap-3">
          <h2 class="font-medium">{{ t('projects.environments') }}</h2>
          <div class="flex flex-wrap items-center gap-2">
            <span
              v-for="env in project.data.value.environments"
              :key="env.id"
              class="inline-flex items-center gap-1"
            >
              <Badge variant="secondary">{{ env.name }}</Badge>
              <button
                v-if="store.isAdmin"
                type="button"
                class="text-muted-foreground hover:text-destructive"
                :aria-label="t('projects.removeEnvironment', { name: env.name })"
                @click="removeEnvironment.mutate(env.id)"
              >
                <Trash2 class="size-3.5" />
              </button>
            </span>
          </div>
          <form
            v-if="store.isAdmin"
            class="flex max-w-sm gap-2"
            @submit.prevent="add().catch(() => undefined)"
          >
            <Input v-model="newEnvironment" :placeholder="t('projects.newEnvironment')" required />
            <Button type="submit" variant="outline" :disabled="addEnvironment.isPending.value"
              ><Plus />{{ t('common.add') }}</Button
            >
          </form>
          <Alert v-if="addEnvironment.error.value" variant="destructive">{{
            problemMessage(addEnvironment.error.value)
          }}</Alert>
          <Alert v-if="removeEnvironment.error.value" variant="destructive">{{
            problemMessage(removeEnvironment.error.value)
          }}</Alert>
        </section>
        <IngestSection :project-id="project.data.value.id" />
      </template>
      <KeysSection v-else-if="tab === 'keys'" :project="project.data.value" />
      <SettingsSection v-else :project="project.data.value" />

      <ConfirmDialog
        v-model:open="deleting"
        :title="t('projects.deleteTitle', { name: project.data.value.name })"
        :description="t('projects.deleteDescription')"
        :confirm-label="t('common.delete')"
        :cancel-label="t('common.cancel')"
        destructive
        @confirm="remove().catch(() => undefined)"
      />
    </template>
  </div>
</template>
