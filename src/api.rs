use crate::redis_client::RedisClient;
use axum::{extract::State, http::StatusCode, Json};
use serde_json::json;
use tracing::error;

/// Handler for GET /api/matches/live
/// Protected with panic recovery to ensure no crashes
pub async fn get_live_matches(
    State(redis): State<RedisClient>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Wrap in panic recovery
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| redis.clone()));

    let redis = match result {
        Ok(r) => r,
        Err(e) => {
            error!("Panic in get_live_matches handler: {:?}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Internal server error",
                    "message": "An unexpected error occurred"
                })),
            ));
        }
    };

    match redis.get_live_matches().await {
        Ok(matches) => Ok(Json(json!(matches))),
        Err(e) => {
            error!("Failed to get live matches: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "Failed to fetch live matches",
                    "details": e.to_string()
                })),
            ))
        }
    }
}

/// Health check endpoint
pub async fn health_check() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "service": "cricket-live-score-backend"
    }))
}
