mod app;
mod auth;
mod config;
mod crypto;
mod database;
mod id;
mod models;
mod routes;
mod storage;

use app::App;
use routes::create_router;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "demo" {
        let app = App::new().expect("Failed to initialize App");
        app.demo("real03.txt").expect("Demo failed");
        return;
    }

    let state = Arc::new(App::new().expect("Failed to initialize App"));
    let router = create_router(state.clone());
    let addr = config::new_port();

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect(&format!("Failed to bind to {}", addr));

    app::App::print_banner(&addr, &state.config.storage_path);

    axum::serve(listener, router).await.expect("Server error");
}
