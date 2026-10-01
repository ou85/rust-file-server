mod app;
mod auth;
mod config;
mod crypto;
mod database;
mod id;
mod models;
mod routes;
mod storage;
mod tools;

use app::App;
use routes::create_router;
use std::sync::Arc;
use tools::hashgen;
use tools::keygen;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
            hashgen::run(None);
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
            app::App::print_banner(&addr, &state.config.data_dir);
            axum::serve(listener, router).await?;
        }
    }

    Ok(())
}
