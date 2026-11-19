# sentry-laravel 4.30.0 (sentry-php 4.37.1)

Envelopes as the SDK sent them from the test app (`dev/laravel`, Laravel
13.43, PHP 8.4), captured by `dev/e2e/capture.mjs` during `just e2e` and
stored decompressed:

- `logs.envelope`: two log entries of `/scenario/log` (`info` with context,
  `warn` with a password in its context);
- `event-request.envelope`: the exception of `/scenario/fail`, with its
  cause, the request and two breadcrumbs;
- `event-job.envelope`: the exception of a queued job, run by the worker.

The DSN in them belongs to a test instance that no longer exists. To renew
them, run `just e2e` and copy from `dev/captured/`.
