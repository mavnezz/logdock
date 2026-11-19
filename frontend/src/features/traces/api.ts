import { useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

export type Trace = components['schemas']['Trace']
export type TraceTransaction = components['schemas']['TraceTransaction']
export type TraceSpan = components['schemas']['TraceSpan']

export function useTrace(id: MaybeRefOrGetter<string>) {
  return useQuery({
    queryKey: computed(() => ['traces', toValue(id)]),
    queryFn: () =>
      unwrap(api.GET('/api/v1/traces/{trace_id}', { params: { path: { trace_id: toValue(id) } } })),
  })
}

export interface Row {
  key: string
  depth: number
  label: string
  detail: string | null
  op: string
  /** Microseconds since the trace's start. */
  offset: number
  duration: number
  failed: boolean
  transaction: boolean
}

/** Transactions and their spans as rows of a waterfall, each under its parent. */
export function waterfall(transactions: TraceTransaction[]): { rows: Row[]; total: number } {
  const micros = (iso: string) => {
    const [seconds, fraction = '0'] = iso.replace('Z', '').split('.')
    return new Date(`${seconds}Z`).getTime() * 1000 + Number(fraction.padEnd(6, '0').slice(0, 6))
  }
  const starts = transactions.flatMap((t) => [
    micros(t.start),
    ...t.spans.map((s) => micros(s.start)),
  ])
  const begin = starts.length ? Math.min(...starts) : 0
  const end = Math.max(
    begin + 1,
    ...transactions.map((t) => micros(t.start) + t.duration_us),
    ...transactions.flatMap((t) => t.spans.map((s) => micros(s.start) + s.duration_us)),
  )
  const rows: Row[] = []
  for (const t of transactions) {
    rows.push({
      key: t.span_id,
      depth: 0,
      label: t.name,
      detail: [t.service, t.host].filter(Boolean).join(' · ') || null,
      op: t.op,
      offset: micros(t.start) - begin,
      duration: t.duration_us,
      failed: t.failed,
      transaction: true,
    })
    const children = new Map<string, TraceSpan[]>()
    for (const span of t.spans) {
      const parent = span.parent_span_id ?? t.span_id
      children.set(parent, [...(children.get(parent) ?? []), span])
    }
    const known = new Set(t.spans.map((s) => s.span_id))
    const visit = (parent: string, depth: number) => {
      for (const span of children.get(parent) ?? []) {
        rows.push({
          key: `${t.span_id}-${span.span_id}`,
          depth,
          label: span.description || span.op,
          detail: null,
          op: span.op,
          offset: micros(span.start) - begin,
          duration: span.duration_us,
          failed: ['internal_error', 'unknown_error', 'deadline_exceeded'].includes(
            span.status ?? '',
          ),
          transaction: false,
        })
        visit(span.span_id, depth + 1)
      }
    }
    visit(t.span_id, 1)
    // Spans whose parent is not among them hang under the transaction.
    for (const [parent, spans] of children) {
      if (parent === t.span_id || known.has(parent)) continue
      for (const span of spans) {
        rows.push({
          key: `${t.span_id}-${span.span_id}`,
          depth: 1,
          label: span.description || span.op,
          detail: null,
          op: span.op,
          offset: micros(span.start) - begin,
          duration: span.duration_us,
          failed: false,
          transaction: false,
        })
      }
    }
  }
  return { rows, total: end - begin }
}
