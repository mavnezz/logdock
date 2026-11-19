<?php

declare(strict_types=1);

namespace App\Jobs;

use App\Exceptions\PaymentFailed;
use Illuminate\Contracts\Queue\ShouldQueue;
use Illuminate\Foundation\Queue\Queueable;
use Illuminate\Support\Facades\Log;

/** A job that logs and fails, run by the worker container. */
final class ChargeOrder implements ShouldQueue
{
    use Queueable;

    public int $tries = 1;

    public function __construct(public readonly int $order)
    {
    }

    public function handle(): void
    {
        Log::info('Charging order {order} in the queue', ['order' => $this->order]);

        throw new PaymentFailed("The queue could not charge order {$this->order}");
    }
}
