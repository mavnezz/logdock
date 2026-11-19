# Laravel

How a Laravel app sends its logs and errors to Logdock. Everything here is set
up in the test app (`dev/laravel`) and checked by `just e2e` (see
[testing.md](testing.md#end-to-end)), with Laravel 13.43, PHP 8.4,
sentry/sentry-laravel 4.30.0 and sentry/sentry 4.37.1.

The app uses Sentry's SDK. Logdock speaks the protocol of Sentry's ingest
endpoint; nothing of Logdock is installed in the app. An app that sends to
Bugsink or Sentry today only changes its DSN.

## 1. The SDK

```bash
composer require sentry/sentry-laravel
php artisan vendor:publish --provider="Sentry\Laravel\ServiceProvider"   # config/sentry.php
```

`bootstrap/app.php` reports exceptions through it:

```php
use Sentry\Laravel\Integration;

->withExceptions(function (Exceptions $exceptions): void {
    Integration::handles($exceptions);
})
```

## 2. `.env`

The same for every container of an app and stage, except the last two lines.
The DSN comes from the project's *Keys* tab in Logdock; each stage has keys of
its own.

```dotenv
SENTRY_LARAVEL_DSN=https://<key>@logdock.example.com/<project-id>
SENTRY_ENVIRONMENT=production
SENTRY_RELEASE=<release-or-commit>
SENTRY_SEND_DEFAULT_PII=false
# Without it, the SDK sends errors only, no log entries.
SENTRY_ENABLE_LOGS=true
SENTRY_LOG_LEVEL=info
# Keep the channel you have, add sentry_logs.
LOG_STACK=daily,sentry_logs

# per container
LOG_SERVICE=horizon
LOG_SERVER=docker-host-02
```

`SENTRY_ENVIRONMENT` must be the stage of the key: Logdock refuses data that
says it comes from another stage (the project's *Overview* counts it as
*wrong stage*). `SENTRY_LOG_LEVEL` decides what goes to Logdock; entries below
it stay in the local channel.

## 3. `config/logging.php`

```php
'service' => env('LOG_SERVICE', 'web'),
'host' => env('LOG_SERVER', 'unknown'),

'channels' => [
    'stack' => [
        'driver' => 'stack',
        'channels' => explode(',', (string) env('LOG_STACK', 'single')),
        'ignore_exceptions' => true,
    ],

    'sentry_logs' => [
        'driver' => 'sentry_logs',
        'level' => env('SENTRY_LOG_LEVEL', env('LOG_LEVEL', 'info')),
        'tap' => [App\Logging\ContainerMetadata::class],
    ],

    // daily, single, stderr, … as before
],
```

`app/Logging/ContainerMetadata.php` puts the container's service and host on
every entry:

```php
<?php

declare(strict_types=1);

namespace App\Logging;

use Illuminate\Log\Logger;
use Monolog\LogRecord;

final class ContainerMetadata
{
    public function __invoke(Logger $logger): void
    {
        $metadata = ['service' => config('logging.service'), 'host' => config('logging.host')];

        $logger->pushProcessor(
            static fn (LogRecord $record): LogRecord => $record->with(extra: [...$record->extra, ...$metadata]),
        );
    }
}
```

The same goes on errors, as tags, in `AppServiceProvider::boot()`:

```php
\Sentry\configureScope(static function (\Sentry\State\Scope $scope): void {
    $scope->setTag('service', (string) config('logging.service'));
    $scope->setTag('host', (string) config('logging.host'));
});
```

For monitoring (requests, jobs, queries, outgoing requests, cache and
scheduled tasks), two more lines:

```dotenv
# The share of requests and jobs measured: 1.0 all, 0.1 one in ten.
SENTRY_TRACES_SAMPLE_RATE=0.2
# Every scheduled task as a monitor, with its crontab.
SENTRY_CRONS_MONITOR_SCHEDULED_TASKS=true
```

With a rate below 1, Logdock counts each measured request as 1/rate requests,
so counts are estimates; durations and percentiles stay real. Every measured
request costs the SDK one more send at its end.

## 4. `config/sentry.php`

The SDK waits up to 2 s to connect and 5 s in total for each send. Lower, a
slow Logdock holds a PHP worker less long:

```php
'http_connect_timeout' => (float) env('SENTRY_HTTP_CONNECT_TIMEOUT', 0.5),
'http_timeout' => (float) env('SENTRY_HTTP_TIMEOUT', 2),
```

## 5. The containers

Logdock shows an entry's *container* as the SDK names the server: the
container's hostname, which Docker sets to the container ID. A hostname per
container makes it readable:

```yaml
services:
  web:
    hostname: orders-web
  horizon:
    hostname: orders-horizon
```

## 6. Trying it

- `php artisan sentry:test` sends a test exception: it shows up under
  *Issues*.
- `Log::info('Hello Logdock')` in `php artisan tinker` shows up under *Logs*
  when the command ends.
- The project's *Overview* shows what arrived and what was refused, and why.

## Things to know

- **Per-container metadata comes from the container's environment**, through
  the tap above. Laravel's `Context` is not suitable for it: the dispatching
  process's context is restored inside a queued job, so a job's logs would say
  `web` instead of `horizon`. `just e2e` checks that a job's entries and errors
  say `horizon`.
- **`ignore_exceptions => true`.** Laravel's default is `false`. Without it, an
  exception thrown by a log handler reaches the code that logged.
- **When logs are sent.** The SDK collects a request's, command's or job's log
  entries and sends them when it ends, in one envelope. A failed send is not
  retried.
- **Placeholders.** `Log::info('Order {order} placed', ['order' => 42])`
  arrives as `Order 42 placed`; the SDK fills them in and sends no template.
  `order` is also kept as an attribute, so `order:42` finds it.
- **Traces.** Without tracing (`SENTRY_TRACES_SAMPLE_RATE` unset), the SDK
  still gives a request's log entries and its error the same trace ID: an
  issue's page shows the logs of the request that failed. A queued job gets a
  trace of its own.
- **Jobs and commands have no transaction name** without tracing; the issue's
  page shows the job's class in the frames and `command: queue:work` as a tag.
- **Cost per request.** Under PHP-FPM, Symfony calls `fastcgi_finish_request()`
  before Laravel runs its terminate callbacks. The SDK therefore sends after
  the visitor has the response, but the worker is busy until the send is done
  or times out.
- **Scheduled tasks** show up with their first run, under their name
  (`scheduled_` dropped). The time zone is the scheduler's (`APP_TIMEZONE` or
  `->timezone()`); Logdock reads the crontab in it, summer time included. A
  task removed from the schedule stays until it is deleted on its page.
- **What is measured** per request: the request itself, its SQL statements
  (with their values replaced), outgoing HTTP requests by host, and cache
  operations; a queued job continues the trace of the request that queued it.
- **What is not kept** from an error event: the request body, cookies and
  environment, headers other than a few harmless ones, and the code around the
  lines of libraries (it makes up most of an event's size). Personal data is
  only sent with `SENTRY_SEND_DEFAULT_PII=true`.
