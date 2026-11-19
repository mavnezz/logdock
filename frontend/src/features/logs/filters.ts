// The log page's filters: as the URL holds them, and as the API takes them.

import type { LocationQuery, LocationQueryRaw } from 'vue-router'

export const LEVELS = ['trace', 'debug', 'info', 'warn', 'error', 'fatal'] as const
export type LevelName = (typeof LEVELS)[number]

export { levelGroup, type LevelGroup } from '@/shared/ui/level'

export const RANGES = ['15m', '1h', '6h', '24h', '7d', '30d'] as const
export type Range = (typeof RANGES)[number] | 'custom'

const RANGE_SECONDS: Record<(typeof RANGES)[number], number> = {
  '15m': 900,
  '1h': 3600,
  '6h': 6 * 3600,
  '24h': 86_400,
  '7d': 7 * 86_400,
  '30d': 30 * 86_400,
}

export interface LogFilters {
  q: string
  levels: LevelName[]
  services: string[]
  hosts: string[]
  range: Range
  /** RFC 3339, for a custom range. */
  from?: string
  to?: string
  trace?: string
}

export function defaultFilters(): LogFilters {
  return { q: '', levels: [], services: [], hosts: [], range: '24h' }
}

type QueryValue = LocationQuery[string] | undefined

const one = (value: QueryValue): string | undefined =>
  (Array.isArray(value) ? value[0] : value) ?? undefined

const many = (value: QueryValue): string[] =>
  (one(value) ?? '')
    .split(',')
    .map((v) => v.trim())
    .filter(Boolean)

/** The filters a URL's query names; the defaults for what it leaves out. */
export function fromQuery(query: LocationQuery): LogFilters {
  const range = one(query.range)
  const filters: LogFilters = {
    q: one(query.q) ?? '',
    levels: many(query.level).filter((l): l is LevelName =>
      (LEVELS as readonly string[]).includes(l),
    ),
    services: many(query.service),
    hosts: many(query.host),
    range:
      range === 'custom' || (RANGES as readonly string[]).includes(range ?? '')
        ? (range as Range)
        : '24h',
  }
  if (filters.range === 'custom') {
    filters.from = one(query.from)
    filters.to = one(query.to)
  }
  const trace = one(query.trace)
  if (trace) filters.trace = trace
  return filters
}

/** The filters as a URL's query, without the defaults. */
export function toQuery(filters: LogFilters): LocationQueryRaw {
  const query: LocationQueryRaw = {}
  if (filters.q) query.q = filters.q
  if (filters.levels.length) query.level = filters.levels.join(',')
  if (filters.services.length) query.service = filters.services.join(',')
  if (filters.hosts.length) query.host = filters.hosts.join(',')
  if (filters.range !== '24h') query.range = filters.range
  if (filters.range === 'custom') {
    if (filters.from) query.from = filters.from
    if (filters.to) query.to = filters.to
  }
  if (filters.trace) query.trace = filters.trace
  return query
}

/** The time range's bounds, RFC 3339. */
export function bounds(filters: LogFilters, now: Date): { from?: string; to?: string } {
  if (filters.range === 'custom') return { from: filters.from, to: filters.to }
  const from = new Date(now.getTime() - RANGE_SECONDS[filters.range] * 1000)
  return { from: from.toISOString() }
}

export interface Scope {
  project: number | null
  environment: number | null
}

/** The filters as the API's query parameters. */
export function toParams(filters: LogFilters, scope: Scope, now: Date): Record<string, string> {
  const params: Record<string, string> = {}
  if (scope.project !== null) params.project = String(scope.project)
  if (scope.environment !== null) params.environment = String(scope.environment)
  if (filters.q.trim()) params.q = filters.q.trim()
  if (filters.levels.length) params.level = filters.levels.join(',')
  if (filters.services.length) params.service = filters.services.join(',')
  if (filters.hosts.length) params.host = filters.hosts.join(',')
  if (filters.trace) {
    // A trace is found whenever it was.
    params.trace = filters.trace
    return params
  }
  const { from, to } = bounds(filters, now)
  if (from) params.from = from
  if (to) params.to = to
  return params
}
