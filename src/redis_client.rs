use crate::models::{FullMatchState, LiveScore, MatchInfo, MatchSummary, Scorecard};
use anyhow::{Context, Result};
use redis::{aio::ConnectionManager, AsyncCommands};
use std::collections::HashMap;
use tracing::debug;

/// Redis client for fetching match data
#[derive(Clone)]
pub struct RedisClient {
    conn: ConnectionManager,
}

impl RedisClient {
    /// Create a new Redis client
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url).context("Failed to create Redis client")?;

        let conn = ConnectionManager::new(client)
            .await
            .context("Failed to connect to Redis")?;

        Ok(Self { conn })
    }

    /// Get all live matches with retry logic
    pub async fn get_live_matches(&self) -> Result<Vec<MatchSummary>> {
        self.with_retry(|| async {
            let mut conn = self.conn.clone();
            let mut matches = Vec::new();

            // Use KEYS command to find all match:*:score keys
            // Note: In production, use SCAN for large datasets
            let keys: Vec<String> = conn
                .keys("match:*:score")
                .await
                .context("Failed to get match keys")?;

            for key in keys {
                // Extract match_id from key (match:{match_id}:score)
                let parts: Vec<&str> = key.split(':').collect();
                if parts.len() != 3 {
                    continue;
                }
                let match_id = parts[1].to_string();

                // Get the score hash
                let score_hash: HashMap<String, String> =
                    conn.hgetall(&key).await.unwrap_or_default();

                // Only include "Live" matches
                if let Some(status) = score_hash.get("match_status") {
                    if status != "Live" && status != "in_progress" && status != "active" {
                        continue;
                    }
                }

                // Get match info
                let info_key = format!("match:{match_id}:info");
                let info_hash: HashMap<String, String> =
                    conn.hgetall(&info_key).await.unwrap_or_default();

                // Build match summary
                let team_a = info_hash.get("team_a_short").cloned().unwrap_or_default();
                let team_b = info_hash.get("team_b_short").cloned().unwrap_or_default();

                let runs = score_hash.get("runs").cloned().unwrap_or_default();
                let wickets = score_hash.get("wickets").cloned().unwrap_or_default();
                let overs = score_hash.get("overs").cloned().unwrap_or_default();
                let current_inning = score_hash
                    .get("current_inning")
                    .cloned()
                    .unwrap_or_default();

                let batting_team = score_hash.get("batting_team").cloned().unwrap_or_default();

                // Format scores
                let (team_a_score, team_b_score) = if current_inning == "1" {
                    if batting_team == info_hash.get("team_a_name").cloned().unwrap_or_default() {
                        (format!("{runs}/{wickets}"), "-".to_string())
                    } else {
                        ("-".to_string(), format!("{runs}/{wickets}"))
                    }
                } else {
                    // Second inning - need to get first inning score
                    // For simplicity, we'll show current batting score
                    if batting_team == info_hash.get("team_a_name").cloned().unwrap_or_default() {
                        (format!("{runs}/{wickets}"), "-".to_string())
                    } else {
                        ("-".to_string(), format!("{runs}/{wickets}"))
                    }
                };

                matches.push(MatchSummary {
                    match_id,
                    team_a,
                    team_b,
                    team_a_score,
                    team_b_score,
                    overs,
                    status: score_hash.get("match_status").cloned().unwrap_or_default(),
                    stage: info_hash.get("stage").cloned(),
                });
            }
            debug!("Found {} live matches", matches.len());
            Ok(matches)
        })
        .await
    }

    /// Get full match state (info + score + scorecards)
    pub async fn get_full_match_state(&self, match_id: &str) -> Result<FullMatchState> {
        let mut conn = self.conn.clone();

        // Get match info
        let info_key = format!("match:{match_id}:info");
        let info_hash: HashMap<String, String> = conn
            .hgetall(&info_key)
            .await
            .context("Failed to get match info")?;

        if info_hash.is_empty() {
            anyhow::bail!("Match not found: {match_id}");
        }

        let info = MatchInfo::from_redis_hash(info_hash)?;

        // Get live score
        let score_key = format!("match:{match_id}:score");
        let score_hash: HashMap<String, String> = conn
            .hgetall(&score_key)
            .await
            .context("Failed to get match score")?;

        let score = LiveScore::from_redis_hash(score_hash)?;

        // Get scorecards
        let scorecard_1_key = format!("match:{match_id}:scorecard:1");
        let scorecard_1_hash: HashMap<String, String> =
            conn.hgetall(&scorecard_1_key).await.unwrap_or_default();

        let scorecard_inn_1 = if !scorecard_1_hash.is_empty() {
            Some(Scorecard::from_redis_hash(scorecard_1_hash)?)
        } else {
            None
        };

        let scorecard_2_key = format!("match:{match_id}:scorecard:2");
        let scorecard_2_hash: HashMap<String, String> =
            conn.hgetall(&scorecard_2_key).await.unwrap_or_default();

        let scorecard_inn_2 = if !scorecard_2_hash.is_empty() {
            Some(Scorecard::from_redis_hash(scorecard_2_hash)?)
        } else {
            None
        };

        Ok(FullMatchState {
            match_id: match_id.to_string(),
            info,
            score,
            scorecard_inn_1,
            scorecard_inn_2,
        })
    }

    /// Get only the live score for a match
    pub async fn get_live_score(&self, match_id: &str) -> Result<LiveScore> {
        let mut conn = self.conn.clone();
        let score_key = format!("match:{match_id}:score");
        let score_hash: HashMap<String, String> = conn
            .hgetall(&score_key)
            .await
            .context("Failed to get match score")?;

        LiveScore::from_redis_hash(score_hash)
    }

    /// Get scorecard for a specific inning
    pub async fn get_scorecard(&self, match_id: &str, inning: u8) -> Result<Option<Scorecard>> {
        let mut conn = self.conn.clone();
        let scorecard_key = format!("match:{match_id}:scorecard:{inning}");
        let scorecard_hash: HashMap<String, String> =
            conn.hgetall(&scorecard_key).await.unwrap_or_default();

        if scorecard_hash.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Scorecard::from_redis_hash(scorecard_hash)?))
        }
    }

    /// Execute an operation with retry logic
    async fn with_retry<F, Fut, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY_MS: u64 = 100;

        let mut last_error = None;

        for attempt in 1..=MAX_RETRIES {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        tracing::warn!(
                            "Redis operation failed (attempt {}/{}): {}. Retrying in {}ms...",
                            attempt,
                            MAX_RETRIES,
                            e,
                            RETRY_DELAY_MS
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS))
                            .await;
                        last_error = Some(e);
                    } else {
                        tracing::error!(
                            "Redis operation failed after {} attempts: {}",
                            MAX_RETRIES,
                            e
                        );
                        last_error = Some(e);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Operation failed with no error")))
    }
}
