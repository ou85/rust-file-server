mod app;
mod auth;
mod blob_store;
mod config;
mod crypto;
mod domain;
mod metadata;
mod tools;
mod web;

use app::App;
use std::sync::Arc;
use tools::hashgen;
use tools::keygen;
use web::create_router;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "Application failed");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().collect();

    if matches!(args.get(1).map(String::as_str), Some("--help") | Some("-h")) {
        println!("{}", config::usage());
        return Ok(());
    }

    match args.get(1).map(|s| s.as_str()) {
        Some("keygen") => {
            keygen::run(32);
        }
        Some("hashgen") => {
            hashgen::run(None)?;
        }
        Some("password") => {
            let reset = matches!(args.get(2).map(String::as_str), Some("reset"));
            if !reset && !matches!(args.get(2).map(String::as_str), Some("change")) {
                return Err("Usage: password change|reset [--local] [--data-dir <PATH>]".into());
            }
            let config = config::Config::from_args(&args[3..])?;
            let app = App::new(config)?;
            use std::io::{self, Write};
            let current = if reset {
                None
            } else {
                print!("Current password: ");
                io::stdout().flush()?;
                let mut value = String::new();
                io::stdin().read_line(&mut value)?;
                Some(value.trim().to_string())
            };
            print!("New password (minimum 12 characters): ");
            io::stdout().flush()?;
            let mut next = String::new();
            io::stdin().read_line(&mut next)?;
            auth::change_password(&app.metadata, current.as_deref(), next.trim())?;
            println!("Password changed. Existing sessions are now invalid.");
        }
        Some("demo") => {
            let config = config::Config::from_args(&args[2..])?;
            let app = App::new(config)?;
            app.demo("README.md")?;
        }
        _ => {
            let config = config::Config::from_args(&args[1..])?;
            let state = Arc::new(App::new(config)?);
            let router = create_router(state.clone());
            let addr = state.config.bind_address.clone();
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            app::App::print_banner(&addr, &state.config.metadata_path)?;
            axum::serve(listener, router).await?;
        }
    }

    Ok(())
}
