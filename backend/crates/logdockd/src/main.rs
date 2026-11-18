//! `logdockd`: serves Logdock (the default), checks its health for Docker,
//! or prints the OpenAPI document.

use std::process::ExitCode;

use logdockd::config::{Config, DEFAULT_LISTEN};

const USAGE: &str = "usage: logdockd [serve | health | openapi | version]";

fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_else(|| "serve".to_owned());
    match command.as_str() {
        "serve" => serve(),
        "health" => health(),
        "openapi" => {
            match logdock_api::openapi().to_pretty_json() {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("cannot write the OpenAPI document: {error}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        "version" | "--version" => {
            println!("logdockd {}", logdock_kernel::VERSION);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn serve() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("logdockd: invalid configuration: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = logdockd::logging::init(&config.log_filter, config.log_format) {
        eprintln!("logdockd: {error:#}");
        return ExitCode::from(2);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("logdockd: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(logdockd::app::serve(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = format!("{error:#}"), "logdockd failed");
            ExitCode::FAILURE
        }
    }
}

fn health() -> ExitCode {
    // Only the address matters here; the rest of the configuration may be
    // whatever it is.
    let listen = std::env::var("LOGDOCK_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.to_owned());
    let Ok(listen) = listen.trim().parse() else {
        eprintln!("LOGDOCK_LISTEN: {listen:?} is no address and port");
        return ExitCode::FAILURE;
    };
    match logdockd::health::check(listen) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unhealthy: {error}");
            ExitCode::FAILURE
        }
    }
}
