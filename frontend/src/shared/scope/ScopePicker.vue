<script setup lang="ts">
import { useQuery } from '@tanstack/vue-query'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { api, unwrap } from '@/shared/api/client'
import { NativeSelect } from '@/shared/ui/native-select'

import { useScopeStore } from './store'

const { t } = useI18n()
const scope = useScopeStore()
const projects = useQuery({
  queryKey: ['projects'],
  queryFn: () => unwrap(api.GET('/api/v1/projects')),
})
const environments = computed(
  () => projects.data.value?.find((p) => p.id === scope.project)?.environments ?? [],
)
const project = computed({
  get: () => scope.project ?? '',
  set: (value: string | number) => scope.choose(value === '' ? null : Number(value)),
})
const environment = computed({
  get: () => scope.environment ?? '',
  set: (value: string | number) => (scope.environment = value === '' ? null : Number(value)),
})
</script>

<!-- Which project and stage the page shows: all, or one. -->
<template>
  <div class="flex flex-wrap items-center gap-2">
    <NativeSelect v-model="project" class="w-48" :aria-label="t('scope.project')">
      <option value="">{{ t('scope.allProjects') }}</option>
      <option v-for="p in projects.data.value ?? []" :key="p.id" :value="p.id">{{ p.name }}</option>
    </NativeSelect>
    <NativeSelect
      v-model="environment"
      class="w-40"
      :disabled="scope.project === null"
      :aria-label="t('scope.environment')"
    >
      <option value="">{{ t('scope.allEnvironments') }}</option>
      <option v-for="e in environments" :key="e.id" :value="e.id">{{ e.name }}</option>
    </NativeSelect>
  </div>
</template>
