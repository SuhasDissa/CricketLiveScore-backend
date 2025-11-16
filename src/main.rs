mod api;
mod models;
mod pubsub;
mod redis_client;
mod websocket;

use anyhow::{Context, Result};
use axum::{routing::get, Router};
use redis_client::RedisClient;
use std::net::SocketAddr;
use std::panic;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;
use websocket::WsState;

#[tokio::main]
async fn main() -> Result<()> {
    // Set custom panic hook to prevent crashes
    panic::set_hook(Box::new(|panic_info| {
        let msg = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "Unknown panic"
        };

        let location = if let Some(location) = panic_info.location() {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        } else {
            "Unknown location".to_string()
        };

        eprintln!("PANIC CAUGHT - This should never crash the server!");
        eprintln!("Message: {msg}");
        eprintln!("Location: {location}");
        eprintln!(
            "Backtrace: {:?}",
            std::backtrace::Backtrace::force_capture()
        );
    }));

    // Initialize tracing with environment-based log level
    let log_level = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "info".to_string())
        .parse::<Level>()
        .unwrap_or(Level::INFO);

    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting Cricket Live Score Backend (Rust)");

    // Get configuration from environment
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3001".to_string())
        .parse::<u16>()
        .context("Invalid PORT")?;

    // Create Redis client with retry logic
    let redis_client = connect_to_redis_with_retry(&redis_url, 10, 5).await?;

    info!("Connected to Redis");

    // Create WebSocket state
    let ws_state = WsState::new(redis_client.clone());

    // Start Redis Pub/Sub listener in background with auto-reconnect
    let ws_state_clone = ws_state.clone();
    let redis_url_clone = redis_url.clone();
    tokio::spawn(async move {
        loop {
            warn!("Starting Redis Pub/Sub listener...");
            match pubsub::start_pubsub_listener(&redis_url_clone, ws_state_clone.clone()).await {
                Ok(_) => {
                    warn!("Pub/Sub listener ended normally");
                }
                Err(e) => {
                    error!("Pub/Sub listener error: {}. Reconnecting in 5s...", e);
                }
            }

            // Wait before reconnecting
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            warn!("Attempting to reconnect Pub/Sub listener...");
        }
    });

    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router with nested routers for different states
    let api_routes = Router::new()
        .route("/api/matches/live", get(api::get_live_matches))
        .with_state(redis_client);

    let ws_routes = Router::new()
        .route("/ws", get(websocket::ws_handler))
        .with_state(ws_state);

    // Build middleware stack for resilience
    let middleware = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(Duration::from_secs(30))) // 30s timeout for all requests
        .layer(cors);

    let app = Router::new()
        .route("/health", get(api::health_check))
        .merge(api_routes)
        .merge(ws_routes)
        .layer(middleware);

    // Start server
    let addr = SocketAddr::new(host.parse()?, port);
    info!("Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind to address")?;

    // Setup graceful shutdown
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn signal handler
    tokio::spawn(async move {
        shutdown_signal().await;
        info!("Shutdown signal received, starting graceful shutdown...");
        let _ = shutdown_tx.send(());
    });

    // Run server with graceful shutdown
    info!("Server ready to accept connections");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            shutdown_rx.await.ok();
        })
        .await
        .context("Server error")?;

    info!("Server shutdown complete");
    Ok(())
}

/// Connect to Redis with retry logic
async fn connect_to_redis_with_retry(
    redis_url: &str,
    max_retries: u32,
    retry_delay_secs: u64,
) -> Result<RedisClient> {
    info!("Connecting to Redis at: {}", redis_url);

    for attempt in 1..=max_retries {
        match RedisClient::new(redis_url).await {
            Ok(client) => {
                info!("Successfully connected to Redis on attempt {}", attempt);
                return Ok(client);
            }
            Err(e) => {
                if attempt < max_retries {
                    warn!(
                        "Failed to connect to Redis (attempt {}/{}): {}. Retrying in {}s...",
                        attempt, max_retries, e, retry_delay_secs
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(retry_delay_secs)).await;
                } else {
                    error!(
                        "Failed to connect to Redis after {} attempts: {}",
                        max_retries, e
                    );
                    return Err(e);
                }
            }
        }
    }

    unreachable!()
}

/// Wait for shutdown signal (SIGTERM, SIGINT, or Ctrl+C)
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C signal");
        },
        _ = terminate => {
            info!("Received SIGTERM signal");
        },
    }
}
