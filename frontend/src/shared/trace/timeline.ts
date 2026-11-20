// What a request (or job, or command) did and how long each part took: its
// transactions and spans as a tree, as Nightwatch's timeline and Sentry's
// trace view show them.
import type { components } from '@/shared/api/schema'

export type TraceTransaction = components['schemas']['TraceTransaction']
export type TraceSpan = components['schemas']['TraceSpan']

/** Where time goes; the colour of a bar and a share of the breakdown. */
export const CATEGORIES = ['db', 'http', 'cache', 'view', 'queue', 'framework', 'app'] as const
export type Category = (typeof CATEGORIES)[number]

export function categoryOf(op: string): Category {
  if (op.startsWith('db')) return 'db'
  if (op === 'http.client') return 'http'
  if (op.startsWith('cache.')) return 'cache'
  if (op.startsWith('view.')) return 'view'
  if (op === 'queue.publish') return 'queue'
  if (
    ['app.bootstrap', 'app.php.autoload', 'middleware.handle', 'http.route.response'].includes(op)
  ) {
    return 'framework'
  }
  // The route's closure or controller, the job's handle, and spans the SDK
  // has no name for: the app's own code.
  return 'app'
}

export interface Node {
  key: string
  kind: 'transaction' | 'span' | 'group'
  op: string
  label: string
  /** Service and host of a transaction. */
  detail: string | null
  category: Category
  /** Microseconds since the timeline's start. */
  start: number
  duration: number
  /** Time not spent in any child: the node's own work. */
  self: number
  failed: boolean
  status: string | null
  data: Record<string, unknown>
  spanId: string | null
  children: Node[]
  /** A group's repeated spans. */
  count: number
}

export interface Timeline {
  roots: Node[]
  /** Microseconds of the whole timeline. */
  total: number
}

const FAILED = ['internal_error', 'unknown_error', 'deadline_exceeded', 'aborted', 'unavailable']

/** Microseconds since the epoch of an RFC 3339 time with microseconds. */
export function micros(iso: string): number {
  const [seconds, fraction = '0'] = iso.replace('Z', '').split('.')
  return new Date(`${seconds}Z`).getTime() * 1000 + Number(fraction.padEnd(6, '0').slice(0, 6))
}

/** Length of the union of `intervals`, cut to [from, to]. */
function covered(intervals: [number, number][], from: number, to: number): number {
  const sorted = intervals
    .map(([a, b]): [number, number] => [Math.max(a, from), Math.min(b, to)])
    .filter(([a, b]) => b > a)
    .sort((x, y) => x[0] - y[0])
  let total = 0
  let end = -Infinity
  for (const [a, b] of sorted) {
    if (a >= end) {
      total += b - a
      end = b
    } else if (b > end) {
      total += b - end
      end = b
    }
  }
  return total
}

/** At least this many of the same span in a row are shown as one group. */
export const GROUP_FROM = 3

function finish(node: Node): Node {
  node.children.sort((a, b) => a.start - b.start)
  node.children.forEach(finish)
  // Own time counts the spans themselves, not the gaps a group spans.
  node.self = Math.max(
    0,
    node.duration -
      covered(
        node.children.map((c) => [c.start, c.start + c.duration]),
        node.start,
        node.start + node.duration,
      ),
  )
  node.children = group(node.children)
  return node
}

/** Runs of the same span (an N+1 query, a loop of cache reads) as one row. */
function group(children: Node[]): Node[] {
  const out: Node[] = []
  let i = 0
  while (i < children.length) {
    const first = children[i]!
    let j = i + 1
    while (
      j < children.length &&
      first.kind === 'span' &&
      children[j]!.kind === 'span' &&
      children[j]!.op === first.op &&
      children[j]!.label === first.label &&
      children[j]!.children.length === 0
    ) {
      j++
    }
    const run = children.slice(i, j)
    if (run.length >= GROUP_FROM && first.children.length === 0) {
      const end = Math.max(...run.map((n) => n.start + n.duration))
      out.push({
        ...first,
        key: `${first.key}-group`,
        kind: 'group',
        duration: end - first.start,
        self: run.reduce((sum, n) => sum + n.duration, 0),
        failed: run.some((n) => n.failed),
        children: run,
        count: run.length,
      })
    } else {
      out.push(...run)
    }
    i = j
  }
  return out
}

/** The trace's transactions and spans as a tree; a transaction started by a
 * span of another (a job its request queued) hangs under that span. */
export function buildTimeline(transactions: TraceTransaction[]): Timeline {
  const all = transactions.flatMap((t) => [
    [micros(t.start), micros(t.start) + t.duration_us],
    ...t.spans.map((s) => [micros(s.start), micros(s.start) + s.duration_us]),
  ])
  if (all.length === 0) return { roots: [], total: 0 }
  const begin = Math.min(...all.map(([a]) => a!))
  const end = Math.max(begin + 1, ...all.map(([, b]) => b!))

  const bySpan = new Map<string, Node>()
  const tops: { node: Node; parent: string | null | undefined }[] = []
  for (const t of transactions) {
    const node: Node = {
      key: t.span_id,
      kind: 'transaction',
      op: t.op,
      label: t.name,
      detail: [t.service, t.host].filter(Boolean).join(' · ') || null,
      category: 'app',
      start: micros(t.start) - begin,
      duration: t.duration_us,
      self: 0,
      failed: t.failed,
      status: t.status ?? null,
      data: {},
      spanId: t.span_id,
      children: [],
      count: 1,
    }
    bySpan.set(t.span_id, node)
    const spans = t.spans.map((s): Node => ({
      key: `${t.span_id}-${s.span_id}`,
      kind: 'span',
      op: s.op,
      label: s.description || s.op,
      detail: null,
      category: categoryOf(s.op),
      start: micros(s.start) - begin,
      duration: s.duration_us,
      self: 0,
      failed: FAILED.includes(s.status ?? ''),
      status: s.status ?? null,
      data: s.data ?? {},
      spanId: s.span_id,
      children: [],
      count: 1,
    }))
    // A span's parent is one of its transaction's spans, or the transaction;
    // one whose parent is unknown hangs under the transaction.
    const own = new Map(spans.map((span) => [span.spanId!, span]))
    const parents = new Map(t.spans.map((s) => [s.span_id, s.parent_span_id ?? null]))
    // Broken data may name parents in a circle; such spans hang under the
    // transaction instead.
    const circles = (id: string) => {
      const seen = new Set<string>()
      for (let at: string | null | undefined = id; at && own.has(at); at = parents.get(at)) {
        if (seen.has(at)) return true
        seen.add(at)
      }
      return false
    }
    t.spans.forEach((s, n) => {
      const candidate = s.parent_span_id ? own.get(s.parent_span_id) : undefined
      const parent = candidate && !circles(s.span_id) ? candidate : node
      parent.children.push(spans[n]!)
    })
    spans.forEach((span) => bySpan.set(span.spanId!, span))
    tops.push({ node, parent: t.parent_span_id })
  }
  // The transaction each span belongs to, to keep transactions from hanging
  // under each other in a circle.
  const ownerOf = new Map<string, string>()
  for (const t of transactions) t.spans.forEach((s) => ownerOf.set(s.span_id, t.span_id))
  const parentTransaction = new Map(
    tops.map(({ node, parent }) => [node.spanId!, parent ? (ownerOf.get(parent) ?? null) : null]),
  )
  const circles = (id: string) => {
    const seen = new Set<string>()
    for (let at: string | null | undefined = id; at; at = parentTransaction.get(at)) {
      if (seen.has(at)) return true
      seen.add(at)
    }
    return false
  }
  const roots: Node[] = []
  for (const { node, parent } of tops) {
    const under = parent ? bySpan.get(parent) : undefined
    if (under && !circles(node.spanId!)) under.children.push(node)
    else roots.push(node)
  }
  roots.sort((a, b) => a.start - b.start)
  return { roots: roots.map(finish), total: end - begin }
}

/** How a node's time splits over categories: the self time of it and every
 * node below, so the shares add up to its duration. */
export function breakdown(node: Node): { category: Category; micros: number }[] {
  const sums = new Map<Category, number>()
  const visit = (n: Node) => {
    // A transaction's own time is the app's; a group's is its spans'.
    const own = n.kind === 'group' ? n.children.reduce((s, c) => s + c.duration, 0) : n.self
    sums.set(n.category, (sums.get(n.category) ?? 0) + own)
    if (n.kind !== 'group') n.children.forEach(visit)
  }
  visit(node)
  return CATEGORIES.map((category) => ({ category, micros: sums.get(category) ?? 0 })).filter(
    (part) => part.micros > 0,
  )
}

/** The transaction a span (of an error event) belongs to, or by name. */
export function transactionOf(
  timeline: Timeline,
  spanId: string | null | undefined,
  name: string | null | undefined,
): Node | null {
  const transactions: Node[] = []
  let found: Node | null = null
  const visit = (n: Node, owner: Node | null) => {
    const own = n.kind === 'transaction' ? n : owner
    if (n.kind === 'transaction') transactions.push(n)
    if (spanId && n.spanId === spanId) found ??= own
    n.children.forEach((c) => visit(c, own))
  }
  timeline.roots.forEach((r) => visit(r, null))
  return found ?? transactions.find((t) => t.label === name) ?? transactions[0] ?? null
}
