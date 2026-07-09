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

    match args.get(1).map(|s| s.as_str()) {
        Some("keygen") => {
            keygen::run(32);
        }
        Some("hashgen") => {
            hashgen::run(None);
        }
        Some("demo") => {
            let app = App::new()?;
            app.demo("README.md")?;
        }
        _ => {
            let state = Arc::new(App::new()?);
            let router = create_router(state.clone());
            let addr = config::new_port();
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            app::App::print_banner(
                &addr,
                &state.config.storage_path,
                &state.config.database_path,
            );
            axum::serve(listener, router).await?;
        }
    }

    Ok(())
}
