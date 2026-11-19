<?php

declare(strict_types=1);

namespace App\Logging;

use Illuminate\Log\Logger;
use Monolog\LogRecord;

/**
 * Adds the container's service and host to every entry of a channel, from
 * config/logging.php (LOG_SERVICE, LOG_SERVER).
 */
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
