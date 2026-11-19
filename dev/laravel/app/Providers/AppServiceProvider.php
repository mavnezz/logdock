<?php

declare(strict_types=1);

namespace App\Providers;

use Illuminate\Support\ServiceProvider;
use Sentry\State\Scope;

use function Sentry\configureScope;

class AppServiceProvider extends ServiceProvider
{
    public function register(): void
    {
    }

    public function boot(): void
    {
        // The same metadata as on log entries, on errors.
        configureScope(static function (Scope $scope): void {
            $scope->setTag('service', (string) config('logging.service'));
            $scope->setTag('host', (string) config('logging.host'));
        });
    }
}
