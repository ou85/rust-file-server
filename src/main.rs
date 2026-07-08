mod app;
mod auth;
mod config;
mod crypto;
mod database;
mod hashgen;
mod id;
mod keygen;
mod models;
mod routes;
mod storage;

use app::App;
use routes::create_router;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    // Console apps `cargo run hashgen`, `cargo run keygen`, `cargo run demo`
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("keygen") => {
            keygen::generate_key(32);
            return;
        }
        Some("hashgen") => {
            hashgen::generate_hash(None);
            return;
        }
        Some("demo") => {
            let app = App::new().unwrap();
            app.demo("README.md").unwrap();
            return;
        }
        _ => {}
    }

    // HTTP server
    let state = Arc::new(App::new().unwrap());
    let router = create_router(state.clone());
    let addr = config::new_port();
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    app::App::print_banner(&addr, &state.config.storage_path);
    axum::serve(listener, router).await.unwrap();
}
