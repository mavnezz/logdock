<script setup lang="ts">
import { Plus } from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, useRouter } from 'vue-router'

import { useSessionStore } from '@/shared/session/store'
import { problemMessage } from '@/shared/i18n'
import { formatDateTime } from '@/shared/lib/format'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { ModalDialog } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { EmptyState, FormField, PageHeader } from '@/shared/ui/page'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'
import { Loading } from '@/shared/ui/skeleton'

import { useCreateProject, useProjects } from './api'

const { t } = useI18n()
const router = useRouter()
const store = useSessionStore()
const projects = useProjects()
const create = useCreateProject()

const open = ref(false)
const name = ref('')
const environments = ref('production, staging')
const stages = computed(() =>
  environments.value
    .split(',')
    .map((e) => e.trim())
    .filter(Boolean),
)

async function submit(): Promise<void> {
  const project = await create.mutateAsync({ name: name.value, environments: stages.value })
  open.value = false
  name.value = ''
  await router.push(`/projects/${project.id}`)
}
</script>

<template>
  <div class="grid gap-6">
    <PageHeader :title="t('projects.title')" :description="t('projects.description')">
      <Button v-if="store.isAdmin" @click="open = true"><Plus />{{ t('projects.create') }}</Button>
    </PageHeader>

    <Loading v-if="projects.isPending.value" />
    <Alert v-else-if="projects.error.value" variant="destructive">{{
      problemMessage(projects.error.value)
    }}</Alert>
    <EmptyState
      v-else-if="projects.data.value?.length === 0"
      :title="t('projects.empty')"
      :description="t('projects.emptyHint')"
    >
      <Button v-if="store.isAdmin" @click="open = true"><Plus />{{ t('projects.create') }}</Button>
    </EmptyState>
    <Table v-else>
      <TableHeader>
        <TableRow>
          <TableHead>{{ t('projects.name') }}</TableHead>
          <TableHead>{{ t('projects.environments') }}</TableHead>
          <TableHead>{{ t('projects.created') }}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow v-for="project in projects.data.value" :key="project.id">
          <TableCell>
            <RouterLink :to="`/projects/${project.id}`" class="font-medium hover:underline">{{
              project.name
            }}</RouterLink>
          </TableCell>
          <TableCell>
            <div class="flex flex-wrap gap-1">
              <Badge v-for="env in project.environments" :key="env.id" variant="secondary">{{
                env.name
              }}</Badge>
            </div>
          </TableCell>
          <TableCell class="text-muted-foreground">{{
            formatDateTime(project.created_at)
          }}</TableCell>
        </TableRow>
      </TableBody>
    </Table>

    <ModalDialog v-model:open="open" :title="t('projects.create')" :close-label="t('common.close')">
      <form class="grid gap-4" @submit.prevent="submit().catch(() => undefined)">
        <FormField :label="t('projects.name')" for="project-name" :hint="t('projects.nameHint')">
          <Input id="project-name" v-model="name" required autofocus />
        </FormField>
        <FormField
          :label="t('projects.environments')"
          for="project-envs"
          :hint="t('projects.environmentsHint')"
        >
          <Input id="project-envs" v-model="environments" />
        </FormField>
        <Alert v-if="create.error.value" variant="destructive">{{
          problemMessage(create.error.value)
        }}</Alert>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="outline" @click="open = false">{{
            t('common.cancel')
          }}</Button>
          <Button type="submit" :disabled="create.isPending.value">{{ t('common.create') }}</Button>
        </div>
      </form>
    </ModalDialog>
  </div>
</template>
