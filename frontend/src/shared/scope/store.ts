import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

const STORAGE_KEY = 'logdock.scope'

interface Stored {
  project: number | null
  environment: number | null
}

function stored(): Stored {
  try {
    const value = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as Partial<Stored>
    return { project: value.project ?? null, environment: value.environment ?? null }
  } catch {
    return { project: null, environment: null }
  }
}

/** The project and stage chosen above logs, issues and monitoring, kept in this browser. */
export const useScopeStore = defineStore('scope', () => {
  const initial = stored()
  const project = ref<number | null>(initial.project)
  const environment = ref<number | null>(initial.environment)

  watch([project, environment], ([p, e]) => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ project: p, environment: e }))
    } catch {
      // Storage unavailable: kept for this page only.
    }
  })

  function choose(next: number | null): void {
    if (next !== project.value) environment.value = null
    project.value = next
  }

  return { project, environment, choose }
})
