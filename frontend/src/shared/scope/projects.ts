import { useQuery } from '@tanstack/vue-query'
import { computed } from 'vue'

import { api, unwrap } from '@/shared/api/client'

/** Names of projects and stages by ID, for lists that show several. */
export function useProjectNames() {
  const projects = useQuery({
    queryKey: ['projects'],
    queryFn: () => unwrap(api.GET('/api/v1/projects')),
  })
  const names = computed(() => {
    const projectNames = new Map<number, string>()
    const environmentNames = new Map<number, string>()
    for (const project of projects.data.value ?? []) {
      projectNames.set(project.id, project.name)
      for (const environment of project.environments)
        environmentNames.set(environment.id, environment.name)
    }
    return { projectNames, environmentNames }
  })
  return {
    project: (id: number) => names.value.projectNames.get(id) ?? `#${id}`,
    environment: (id: number) => names.value.environmentNames.get(id) ?? `#${id}`,
    /** A project's settings, once loaded. */
    settings: (id: number) => projects.data.value?.find((p) => p.id === id)?.settings,
  }
}
