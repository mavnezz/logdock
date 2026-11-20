// Shapes the API answers with, for component tests.

import type { Account, Project, Session } from '@/shared/api/types'

export function account(overrides: Partial<Account> = {}): Account {
  return {
    id: 1,
    email: 'ada@example.com',
    name: 'Ada',
    role: 'admin',
    locale: 'en',
    disabled: false,
    created_at: '2026-11-18T09:00:00Z',
    ...overrides,
  }
}

export function session(overrides: Partial<Account> = {}): Session {
  return { account: account(overrides), csrf_token: 'csrf', expires_at: '2026-11-19T09:00:00Z' }
}

export function project(overrides: Partial<Project> = {}): Project {
  return {
    id: 3,
    name: 'orders-backend',
    created_at: '2026-11-18T09:00:00Z',
    environments: [
      { id: 1, name: 'production', created_at: '2026-11-18T09:00:00Z' },
      { id: 2, name: 'staging', created_at: '2026-11-18T09:00:00Z' },
    ],
    settings: {
      notify: ['ops@example.com'],
      notify_new_issues: true,
      notify_regressions: true,
      notify_monitors: false,
      redaction: {
        extra_keys: ['pin'],
        allowed_keys: null,
        scrub_emails: false,
        scrub_ip_addresses: false,
      },
      performance: {
        request_ms: 2000,
        job_ms: null,
        overrides: [{ name: 'GET /health', ms: 0 }],
        n_plus_one: true,
        slow_query_ms: 1000,
      },
    },
    ...overrides,
  }
}
