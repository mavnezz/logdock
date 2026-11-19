<?php

declare(strict_types=1);

use Illuminate\Support\Facades\Log;
use Illuminate\Support\Facades\Schedule;

// Run by the scheduler container (schedule:work), or at once with
// `docker compose -f compose.dev.yaml exec scheduler php artisan schedule:run`.
Schedule::call(fn () => Log::info('Scheduled heartbeat'))->everyMinute()->name('heartbeat');

Schedule::call(function (): void {
    throw new RuntimeException('The nightly report could not reach the warehouse');
})->everyMinute()->name('nightly-report');
