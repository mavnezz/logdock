import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, reply, stubApi } from '@/shared/testing'
import { account, session } from '@/shared/testing/fixtures'

import AccountsPage from './AccountsPage.vue'
import AuditPage from './AuditPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

function button(label: string): HTMLButtonElement {
  const found = [...document.body.querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!found) throw new Error(`no button ${label}`)
  return found
}

function type(selector: string, value: string): void {
  const input = document.body.querySelector<HTMLInputElement>(selector)!
  input.value = value
  input.dispatchEvent(new Event('input'))
}

describe('the accounts page', () => {
  it('lists accounts and manages the others', async () => {
    const bob = account({ id: 2, email: 'bob@example.com', name: 'Bob', role: 'member' })
    const api = stubApi({
      'GET /api/v1/accounts': () => [account(), bob],
      'POST /api/v1/accounts': () => bob,
      'PATCH /api/v1/accounts/2': () => ({ ...bob, disabled: true }),
      'POST /api/v1/accounts/2/password': () => reply(204, undefined),
      'DELETE /api/v1/accounts/2': () => reply(204, undefined),
    })
    const { wrapper } = mountWith(AccountsPage, { attachTo: document.body })
    useSessionStore().set(session())
    await flushPromises()
    expect(wrapper.text()).toContain('bob@example.com')
    expect(wrapper.text()).toContain('you')

    button('Disable').click()
    await flushPromises()
    button('Set password').click()
    await flushPromises()
    type('#reset-password', 'a new password')
    document.body
      .querySelector('#reset-password')!
      .closest('form')!
      .dispatchEvent(new Event('submit'))
    await flushPromises()

    button('New account').click()
    await flushPromises()
    type('#account-name', 'Carol')
    type('#account-email', 'carol@example.com')
    type('#account-password', 'correct horse')
    document.body
      .querySelector('#account-name')!
      .closest('form')!
      .dispatchEvent(new Event('submit'))
    await flushPromises()

    button('Delete').click()
    await flushPromises()
    ;[...document.body.querySelectorAll('[role="alertdialog"] button')]
      .find((b) => b.textContent?.trim() === 'Delete')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await flushPromises()

    const calls = api.calls.map((c) => `${c.method} ${new URL(c.url).pathname}`)
    expect(calls).toContain('PATCH /api/v1/accounts/2')
    expect(calls).toContain('POST /api/v1/accounts/2/password')
    expect(calls).toContain('POST /api/v1/accounts')
    expect(calls).toContain('DELETE /api/v1/accounts/2')
  })
})

describe('the audit page', () => {
  it('shows what happened', async () => {
    stubApi({
      'GET /api/v1/audit': () => [
        {
          id: 1,
          at: '2026-11-18T09:00:00Z',
          action: 'login',
          outcome: 'failure',
          actor: 'eve',
          target: null,
          reason: 'unknown_account',
          client_ip: '10.0.0.1',
        },
        {
          id: 2,
          at: '2026-11-18T09:01:00Z',
          action: 'key.revoke',
          outcome: 'success',
          actor: 'ada',
          target: 'orders',
          reason: null,
          client_ip: null,
        },
      ],
    })
    const { wrapper } = mountWith(AuditPage)
    await flushPromises()
    expect(wrapper.text()).toContain('unknown_account')
    expect(wrapper.text()).toContain('key.revoke')
    expect(wrapper.text()).toContain('failed')
  })
})
