import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import ScopePicker from './ScopePicker.vue'
import { useScopeStore } from './store'

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
})

describe('the project and stage picker', () => {
  it('chooses a project, then one of its stages, and remembers both', async () => {
    stubApi({ 'GET /api/v1/projects': () => [project()] })
    const { wrapper } = mountWith(ScopePicker)
    await flushPromises()
    const [projects, stages] = wrapper.findAll('select')
    expect(stages!.attributes('disabled')).toBeDefined()
    await projects!.setValue('3')
    await stages!.setValue('2')
    const scope = useScopeStore()
    expect([scope.project, scope.environment]).toEqual([3, 2])
    await flushPromises()
    expect(JSON.parse(localStorage.getItem('logdock.scope')!)).toEqual({
      project: 3,
      environment: 2,
    })
    await projects!.setValue('')
    expect([scope.project, scope.environment]).toEqual([null, null])
  })
})
