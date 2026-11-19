import { describe, expect, it } from 'vitest'

import { bounds, defaultFilters, fromQuery, levelGroup, toParams, toQuery } from './filters'

describe('the log filters', () => {
  it('round-trip through the URL without their defaults', () => {
    const filters = {
      ...defaultFilters(),
      q: 'payment',
      levels: ['warn' as const, 'error' as const],
      services: ['horizon'],
      hosts: ['h1'],
    }
    const query = toQuery(filters)
    expect(query).toEqual({ q: 'payment', level: 'warn,error', service: 'horizon', host: 'h1' })
    expect(fromQuery(query as Record<string, string>)).toEqual(filters)
    expect(toQuery(defaultFilters())).toEqual({})
  })

  it('reads custom ranges and traces, and ignores nonsense', () => {
    const filters = fromQuery({
      range: 'custom',
      from: '2026-11-18T09:00:00Z',
      to: '2026-11-18T10:00:00Z',
      level: 'loud,error',
      trace: 'abc',
    })
    expect(filters.range).toBe('custom')
    expect(filters.levels).toEqual(['error'])
    expect(filters.trace).toBe('abc')
    expect(toQuery(filters)).toMatchObject({
      range: 'custom',
      from: '2026-11-18T09:00:00Z',
      trace: 'abc',
    })
    expect(fromQuery({ range: 'forever' }).range).toBe('24h')
  })

  it('become the API parameters', () => {
    const now = new Date('2026-11-18T10:00:00Z')
    const params = toParams(
      { ...defaultFilters(), range: '1h', q: ' x ' },
      { project: 3, environment: 5 },
      now,
    )
    expect(params).toEqual({
      project: '3',
      environment: '5',
      q: 'x',
      from: '2026-11-18T09:00:00.000Z',
    })
    const trace = toParams(
      { ...defaultFilters(), trace: 't1' },
      { project: null, environment: null },
      now,
    )
    expect(trace).toEqual({ trace: 't1' })
    const custom = { ...defaultFilters(), range: 'custom' as const, from: 'a', to: 'b' }
    expect(bounds(custom, now)).toEqual({ from: 'a', to: 'b' })
  })

  it('groups levels into four colours', () => {
    expect(['trace', 'debug', 'info', 'warn', 'error', 'fatal', 'odd'].map(levelGroup)).toEqual([
      'debug',
      'debug',
      'info',
      'warn',
      'error',
      'error',
      'info',
    ])
  })
})
