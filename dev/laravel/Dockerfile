# The Laravel app Logdock is tried with (docs/testing.md): the skeleton of
# laravel/laravel 13, the dependencies pinned in composer.lock, and the few
# files here that docs/architecture.md, 8 asks of an app. For development
# only: php artisan serve, SQLite, no TLS.

FROM composer:2 AS build
WORKDIR /app
RUN composer create-project laravel/laravel:^13.0 . --no-install --no-scripts --no-interaction --prefer-dist
COPY composer.json composer.lock ./
RUN composer install --no-dev --no-interaction --prefer-dist --no-scripts --no-autoloader
COPY app/ app/
COPY bootstrap/app.php bootstrap/app.php
COPY config/ config/
COPY routes/ routes/
RUN composer dump-autoload --no-dev --optimize

FROM php:8.4-cli-alpine
WORKDIR /app
COPY --from=build --chown=www-data:www-data /app /app
RUN mkdir -p /data && chown www-data:www-data /data
USER www-data
ENV DB_CONNECTION=sqlite \
    DB_DATABASE=/data/database.sqlite \
    PHP_CLI_SERVER_WORKERS=4
EXPOSE 8000
CMD ["php", "artisan", "serve", "--host=0.0.0.0", "--port=8000", "--no-reload"]
