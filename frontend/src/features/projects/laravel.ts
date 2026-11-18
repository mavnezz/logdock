// What a Laravel app needs to send to Logdock (docs/architecture.md, 8).

/** The `.env` lines for one container of an app and stage. */
export function laravelEnv(dsn: string, environment: string): string {
  return [
    `SENTRY_LARAVEL_DSN=${dsn}`,
    `SENTRY_ENVIRONMENT=${environment}`,
    'SENTRY_RELEASE=<release-or-commit>',
    'SENTRY_SEND_DEFAULT_PII=false',
    'LOG_STACK=daily,sentry_logs',
    'SENTRY_LOG_LEVEL=info',
    '# per container: web, horizon, scheduler',
    'LOG_SERVICE=web',
    'LOG_SERVER=<docker-host>',
  ].join('\n')
}
