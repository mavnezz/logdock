// Shapes of the issues API, for this feature's tests.

import type { ErrorEvent, Issue, IssueListItem } from './api'

export function issue(overrides: Partial<Issue> = {}): Issue {
  return {
    id: 5,
    project_id: 3,
    environment_id: 1,
    title: 'PaymentFailed: Payment for order 42 failed',
    culprit:
      'App\\Http\\Controllers\\PaymentController::pay in /app/Http/Controllers/PaymentController.php',
    level: 'error',
    status: 'unresolved',
    resolved_at: null,
    resolved_in_next_release: null,
    muted_until: null,
    muted_remaining: null,
    first_seen: '2026-11-18T09:00:00.000000Z',
    last_seen: '2026-11-18T09:15:02.000000Z',
    count: 12,
    first_release: '1.0.0',
    last_release: '1.0.1',
    regressed_at: null,
    last_event_id: 'b1',
    ...overrides,
  }
}

export function listed(overrides: Partial<IssueListItem> = {}): IssueListItem {
  return { ...issue(overrides), last_day: 3, users: 0, ...overrides }
}

export function event(overrides: Partial<ErrorEvent> = {}): ErrorEvent {
  return {
    event_id: 'b1',
    time: '2026-11-18T09:15:02.000000Z',
    level: 'error',
    platform: 'php',
    release: '1.0.1',
    environment: 'production',
    server_name: 'orders-web-1',
    transaction: 'POST /orders/{order}/pay',
    logger: null,
    message: null,
    exceptions: [
      {
        type: 'GuzzleHttp\\Exception\\ConnectException',
        value: 'cURL error 28',
        handled: true,
        frames: [
          {
            filename: '/vendor/guzzle/Client.php',
            function: 'GuzzleHttp\\Client::send',
            lineno: 120,
            in_app: false,
            pre_context: [],
            post_context: [],
          },
          {
            filename: '/app/Payments/Gateway.php',
            function: 'App\\Payments\\Gateway::charge',
            lineno: 58,
            in_app: true,
            context_line: '        $response = $this->http->post($url);',
            pre_context: ['    {'],
            post_context: ['    }'],
          },
        ],
      },
      {
        type: 'App\\Payments\\PaymentFailed',
        value: 'Payment for order 42 failed',
        handled: false,
        frames: [],
      },
    ],
    frames: [],
    tags: [
      ['service', 'web'],
      ['host', 'docker-host-01'],
    ],
    breadcrumbs: [
      {
        time: '2026-11-18T09:15:01.000000Z',
        type: 'default',
        category: 'db.sql.query',
        level: 'info',
        message: 'select * from orders where id = ?',
        data: {},
      },
    ],
    request: {
      method: 'POST',
      url: 'https://orders.example.com/orders/42/pay',
      query: null,
      headers: [
        ['User-Agent', 'Mozilla/5.0'],
        ['Authorization', '[redacted]'],
      ],
      data: { order: 42, password: '[redacted]' },
      cookies: ['laravel_session'],
    },
    user: { id: '17', email: null, username: null, ip_address: null },
    trace_id: '771a43a4192642f0b136d5159a501700',
    span_id: null,
    sdk: 'sentry.php.laravel 4.15.0',
    contexts: { runtime: { name: 'php', version: '8.4.15' } },
    extra: {},
    ...overrides,
  }
}
