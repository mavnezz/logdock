<?php

declare(strict_types=1);

use App\Exceptions\PaymentFailed;
use App\Jobs\ChargeOrder;
use Illuminate\Support\Facades\Log;
use Illuminate\Support\Facades\Route;

// What the end-to-end check (dev/e2e/run.mjs) asks of the app.

Route::get('/', fn () => 'Logdock test app');

Route::get('/scenario/log', function () {
    Log::debug('Debug detail that stays below SENTRY_LOG_LEVEL');
    Log::info('Order {order} placed', ['order' => 42, 'customer' => 'c-17']);
    Log::warning('Payment provider slow', ['password' => 'hunter2', 'took_ms' => 2300]);

    return 'logged';
});

Route::get('/scenario/fail', function () {
    Log::info('Charging order {order}', ['order' => 42]);

    try {
        throw new RuntimeException('cURL error 28: Operation timed out after 5001 milliseconds');
    } catch (RuntimeException $timeout) {
        throw new PaymentFailed('Payment for order 42 failed', previous: $timeout);
    }
});

Route::get('/scenario/handled', function () {
    try {
        throw new InvalidArgumentException('Coupon SUMMER is not valid any more');
    } catch (InvalidArgumentException $exception) {
        report($exception);
    }

    return 'reported';
});

Route::get('/scenario/job', function () {
    ChargeOrder::dispatch(43);

    return 'queued';
});
