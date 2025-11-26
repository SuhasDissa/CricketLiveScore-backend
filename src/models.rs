use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Match information (static data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchInfo {
    pub team_a_name: String,
    pub team_a_short: String,
    pub team_b_name: String,
    pub team_b_short: String,
    pub venue: String,
    pub match_type: String,
    pub date: String,
    pub toss_winner: Option<String>,
    pub toss_decision: Option<String>,
    pub stage: Option<String>,
    pub group_id: Option<String>,
}

/// Live score data (highly dynamic)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveScore {
    pub current_inning: String,
    pub batting_team: String,
    pub bowling_team: String,
    pub runs: u32,
    pub wickets: u8,
    pub overs: String,
    pub target: Option<u32>,
    pub striker_id: String,
    pub striker_name: String,
    pub non_striker_id: String,
    pub non_striker_name: String,
    pub bowler_id: String,
    pub bowler_name: String,
    pub striker_runs: u32,
    pub striker_balls: u32,
    pub striker_fours: u8,
    pub striker_sixes: u8,
    pub non_striker_runs: u32,
    pub non_striker_balls: u32,
    pub non_striker_fours: u8,
    pub non_striker_sixes: u8,
    pub bowler_overs: String,
    pub bowler_runs: u32,
    pub bowler_wickets: u8,
    pub bowler_maidens: u8,
    pub last_ball: String,
    pub last_commentary: String,
    pub commentary: String,
    pub run_rate: String,
    pub req_run_rate: Option<String>,
    pub match_status: String,
}

/// Batsman statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatsmanStats {
    pub name: String,
    pub runs: u32,
    pub balls: u32,
    pub fours: u8,
    pub sixes: u8,
    pub strike_rate: f32,
    pub status: String, // "batting", "out", "not_out"
}

/// Bowler statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BowlerStats {
    pub name: String,
    pub overs: String,
    pub maidens: u8,
    pub runs: u32,
    pub wickets: u8,
    pub economy: f32,
}

/// Full scorecard for an inning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scorecard {
    pub batsmen: HashMap<String, BatsmanStats>,
    pub bowlers: HashMap<String, BowlerStats>,
}

/// Match summary for the live matches list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchSummary {
    pub match_id: String,
    pub team_a: String,
    pub team_b: String,
    pub team_a_score: String,
    pub team_b_score: String,
    pub overs: String,
    pub status: String,
    pub stage: Option<String>,
}

/// Full match state (sent on initial subscription)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullMatchState {
    pub match_id: String,
    pub info: MatchInfo,
    pub score: LiveScore,
    pub scorecard_inn_1: Option<Scorecard>,
    pub scorecard_inn_2: Option<Scorecard>,
}

/// WebSocket message types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum ClientMessage {
    #[serde(rename = "subscribe")]
    Subscribe { match_id: String },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { match_id: String },
}

/// Server-to-client WebSocket messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "full_state")]
    FullState { data: Box<FullMatchState> },
    #[serde(rename = "score_update")]
    ScoreUpdate { data: Box<LiveScore> },
    #[serde(rename = "scorecard_update")]
    ScorecardUpdate { data: Scorecard, inning: u8 },
    #[serde(rename = "error")]
    Error { message: String },
}

impl MatchInfo {
    /// Parse MatchInfo from Redis hash
    pub fn from_redis_hash(hash: HashMap<String, String>) -> Result<Self, anyhow::Error> {
        Ok(Self {
            team_a_name: hash.get("team_a_name").cloned().unwrap_or_default(),
            team_a_short: hash.get("team_a_short").cloned().unwrap_or_default(),
            team_b_name: hash.get("team_b_name").cloned().unwrap_or_default(),
            team_b_short: hash.get("team_b_short").cloned().unwrap_or_default(),
            venue: hash.get("venue").cloned().unwrap_or_default(),
            match_type: hash.get("match_type").cloned().unwrap_or_default(),
            date: hash.get("date").cloned().unwrap_or_default(), // Changed from start_time_utc
            toss_winner: hash.get("toss_winner").cloned(),
            toss_decision: hash.get("toss_decision").cloned(),
            stage: hash.get("stage").cloned(),
            group_id: hash.get("group_id").cloned(),
        })
    }
}

impl LiveScore {
    /// Parse LiveScore from Redis hash
    pub fn from_redis_hash(hash: HashMap<String, String>) -> Result<Self, anyhow::Error> {
        Ok(Self {
            current_inning: hash
                .get("current_inning")
                .cloned()
                .unwrap_or_else(|| "1".to_string()),
            batting_team: hash.get("batting_team").cloned().unwrap_or_default(),
            bowling_team: hash.get("bowling_team").cloned().unwrap_or_default(),
            runs: hash.get("runs").and_then(|s| s.parse().ok()).unwrap_or(0),
            wickets: hash
                .get("wickets")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            overs: hash.get("overs").cloned().unwrap_or_default(),
            target: hash.get("target").and_then(|s| s.parse().ok()),
            striker_id: hash.get("striker_id").cloned().unwrap_or_default(),
            striker_name: hash.get("striker_name").cloned().unwrap_or_default(),
            non_striker_id: hash.get("non_striker_id").cloned().unwrap_or_default(),
            non_striker_name: hash.get("non_striker_name").cloned().unwrap_or_default(),
            bowler_id: hash.get("bowler_id").cloned().unwrap_or_default(),
            bowler_name: hash.get("bowler_name").cloned().unwrap_or_default(),
            striker_runs: hash
                .get("striker_runs")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            striker_balls: hash
                .get("striker_balls")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            striker_fours: hash
                .get("striker_fours")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            striker_sixes: hash
                .get("striker_sixes")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            non_striker_runs: hash
                .get("non_striker_runs")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            non_striker_balls: hash
                .get("non_striker_balls")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            non_striker_fours: hash
                .get("non_striker_fours")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            non_striker_sixes: hash
                .get("non_striker_sixes")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            bowler_overs: hash.get("bowler_overs").cloned().unwrap_or_default(),
            bowler_runs: hash
                .get("bowler_runs")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            bowler_wickets: hash
                .get("bowler_wickets")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            bowler_maidens: hash
                .get("bowler_maidens")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            last_ball: hash.get("last_ball").cloned().unwrap_or_default(),
            last_commentary: hash.get("last_commentary").cloned().unwrap_or_default(),
            commentary: hash.get("commentary").cloned().unwrap_or_else(|| "[]".to_string()),
            run_rate: hash.get("run_rate").cloned().unwrap_or_default(),
            req_run_rate: hash.get("req_run_rate").cloned(),
            match_status: hash.get("match_status").cloned().unwrap_or_default(),
        })
    }
}

impl Scorecard {
    /// Parse Scorecard from Redis hash
    pub fn from_redis_hash(hash: HashMap<String, String>) -> Result<Self, anyhow::Error> {
        let batsmen = hash
            .get("batsmen")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        let bowlers = hash
            .get("bowlers")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        Ok(Self { batsmen, bowlers })
    }
}
