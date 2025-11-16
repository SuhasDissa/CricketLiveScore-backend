use crate::models::ServerMessage;
use crate::websocket::WsState;
use anyhow::{Context, Result};
use futures::StreamExt;
use tracing::{debug, error, info, warn};

/// Start the Redis Pub/Sub listener
pub async fn start_pubsub_listener(redis_url: &str, ws_state: WsState) -> Result<()> {
    let client = redis::Client::open(redis_url).context("Failed to create Redis client")?;
    let mut pubsub = client
        .get_async_pubsub()
        .await
        .context("Failed to get async pubsub")?;

    // Subscribe to pattern: match_updates:*
    pubsub
        .psubscribe("match_updates:*")
        .await
        .context("Failed to subscribe to pattern")?;

    info!("Redis Pub/Sub listener started, listening for match_updates:*");

    // Start listening for messages
    let mut stream = pubsub.on_message();

    while let Some(msg) = stream.next().await {
        // Wrap the entire message handling in a catch block to prevent crashes
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let channel = msg.get_channel_name();
            (channel.to_string(), msg)
        }));

        let (channel, _msg) = match result {
            Ok((ch, m)) => (ch, m),
            Err(e) => {
                error!("Panic while processing message: {:?}", e);
                continue;
            }
        };

        debug!("Received message on channel: {}", channel);

        // Extract match_id from channel name (match_updates:{match_id})
        let parts: Vec<&str> = channel.split(':').collect();
        if parts.len() != 2 {
            warn!("Invalid channel name format: {}", channel);
            continue;
        }

        let match_id = parts[1];

        // Fetch updated score from Redis with error handling
        match ws_state.redis.get_live_score(match_id).await {
            Ok(score) => {
                debug!("Fetched updated score for match: {}", match_id);

                // Broadcast score update to all subscribers
                let score_message = ServerMessage::ScoreUpdate { data: Box::new(score) };
                ws_state.broadcast(match_id, score_message).await;

                debug!("Broadcasted score update for match: {}", match_id);
            }
            Err(e) => {
                error!("Failed to fetch score for match {}: {}", match_id, e);
                // Don't break the loop - continue processing other messages
            }
        }

        // Fetch and broadcast scorecard updates for both innings
        for inning in 1..=2 {
            match ws_state.redis.get_scorecard(match_id, inning).await {
                Ok(Some(scorecard)) => {
                    debug!(
                        "Fetched scorecard for match: {}, inning: {}",
                        match_id, inning
                    );

                    // Broadcast scorecard update
                    let scorecard_message = ServerMessage::ScorecardUpdate {
                        data: scorecard,
                        inning,
                    };
                    ws_state.broadcast(match_id, scorecard_message).await;

                    debug!(
                        "Broadcasted scorecard update for match: {}, inning: {}",
                        match_id, inning
                    );
                }
                Ok(None) => {
                    // No scorecard for this inning yet - this is normal
                    debug!(
                        "No scorecard available for match: {}, inning: {}",
                        match_id, inning
                    );
                }
                Err(e) => {
                    error!(
                        "Failed to fetch scorecard for match {}, inning {}: {}",
                        match_id, inning, e
                    );
                    // Don't break the loop - continue processing
                }
            }
        }
    }

    warn!("Pub/Sub stream ended");
    Ok(())
}
