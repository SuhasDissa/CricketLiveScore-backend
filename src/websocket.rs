use crate::models::{ClientMessage, ServerMessage};
use crate::redis_client::RedisClient;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
};
use futures::{
    future::FutureExt,
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info};

/// Shared state for WebSocket connections
#[derive(Clone)]
pub struct WsState {
    /// Broadcast channels for each match_id
    /// match_id -> broadcast sender
    pub channels: Arc<RwLock<HashMap<String, broadcast::Sender<ServerMessage>>>>,
    /// Redis client for fetching data
    pub redis: RedisClient,
}

impl WsState {
    pub fn new(redis: RedisClient) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            redis,
        }
    }

    /// Get or create a broadcast channel for a match
    pub async fn get_or_create_channel(&self, match_id: &str) -> broadcast::Sender<ServerMessage> {
        let mut channels = self.channels.write().await;

        if let Some(sender) = channels.get(match_id) {
            sender.clone()
        } else {
            let (tx, _) = broadcast::channel(100);
            channels.insert(match_id.to_string(), tx.clone());
            tx
        }
    }

    /// Broadcast a message to all subscribers of a match
    pub async fn broadcast(&self, match_id: &str, message: ServerMessage) {
        let channels = self.channels.read().await;

        if let Some(sender) = channels.get(match_id) {
            // Ignore errors if no receivers
            let _ = sender.send(message);
        }
    }
}

/// WebSocket upgrade handler
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WsState>) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handle individual WebSocket connection
async fn handle_socket(socket: WebSocket, state: WsState) {
    let (sender, receiver) = socket.split();

    // Spawn a task to handle incoming messages with panic recovery
    let state_clone = state.clone();
    tokio::spawn(async move {
        // Catch any panics in the WebSocket handler
        let result =
            std::panic::AssertUnwindSafe(handle_client_messages(receiver, sender, state_clone))
                .catch_unwind()
                .await;

        match result {
            Ok(Ok(())) => {
                debug!("WebSocket connection closed normally");
            }
            Ok(Err(e)) => {
                error!("WebSocket connection error: {}", e);
            }
            Err(panic) => {
                error!("WebSocket handler panicked: {:?}", panic);
            }
        }
    });
}

/// Handle incoming WebSocket messages from client
async fn handle_client_messages(
    mut receiver: SplitStream<WebSocket>,
    mut sender: SplitSink<WebSocket, Message>,
    state: WsState,
) -> anyhow::Result<()> {
    // Track subscriptions for this connection
    let subscriptions: Arc<RwLock<HashSet<String>>> = Arc::new(RwLock::new(HashSet::new()));

    // Channels for each subscribed match
    let receivers: Arc<RwLock<HashMap<String, broadcast::Receiver<ServerMessage>>>> =
        Arc::new(RwLock::new(HashMap::new()));

    loop {
        tokio::select! {
            // Handle incoming messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_text_message(
                            &text,
                            &state,
                            &subscriptions,
                            &receivers,
                            &mut sender,
                        ).await {
                            error!("Error handling message: {}", e);
                            // Send error to client but don't disconnect
                            let error_msg = ServerMessage::Error {
                                message: format!("Failed to process message: {e}"),
                            };
                            if let Ok(json) = serde_json::to_string(&error_msg) {
                                let _ = sender.send(Message::Text(json)).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Client closed connection");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        // Respond to ping with pong
                        if let Err(e) = sender.send(Message::Pong(data)).await {
                            error!("Failed to send pong: {}", e);
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        info!("Connection closed");
                        break;
                    }
                    _ => {}
                }
            }

            // Handle broadcast messages for all subscriptions
            _ = async {
                let mut rcvs = receivers.write().await;
                for (_match_id, rx) in rcvs.iter_mut() {
                    if let Ok(msg) = rx.try_recv() {
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if let Err(e) = sender.send(Message::Text(json)).await {
                                error!("Failed to send broadcast message: {}", e);
                                // Don't break - try to continue
                            }
                        }
                    }
                }
            } => {}
        }
    }

    // Cleanup: unsubscribe from all channels
    info!("Cleaning up WebSocket connection");
    Ok(())
}

/// Handle text message from client
async fn handle_text_message(
    text: &str,
    state: &WsState,
    subscriptions: &Arc<RwLock<HashSet<String>>>,
    receivers: &Arc<RwLock<HashMap<String, broadcast::Receiver<ServerMessage>>>>,
    sender: &mut SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let client_msg: ClientMessage = serde_json::from_str(text)?;

    match client_msg {
        ClientMessage::Subscribe { match_id } => {
            debug!("Client subscribing to match: {}", match_id);

            // Add to subscriptions
            subscriptions.write().await.insert(match_id.clone());

            // Get or create channel
            let tx = state.get_or_create_channel(&match_id).await;
            let rx = tx.subscribe();
            receivers.write().await.insert(match_id.clone(), rx);

            // Fetch and send full state
            match state.redis.get_full_match_state(&match_id).await {
                Ok(full_state) => {
                    let msg = ServerMessage::FullState { data: Box::new(full_state) };
                    let json = serde_json::to_string(&msg)?;
                    sender.send(Message::Text(json)).await?;
                    info!("Sent full state for match: {}", match_id);
                }
                Err(e) => {
                    error!("Failed to get match state: {}", e);
                    let error_msg = ServerMessage::Error {
                        message: format!("Failed to get match state: {e}"),
                    };
                    let json = serde_json::to_string(&error_msg)?;
                    sender.send(Message::Text(json)).await?;
                }
            }
        }

        ClientMessage::Unsubscribe { match_id } => {
            debug!("Client unsubscribing from match: {}", match_id);

            // Remove from subscriptions
            subscriptions.write().await.remove(&match_id);
            receivers.write().await.remove(&match_id);

            info!("Client unsubscribed from match: {}", match_id);
        }
    }

    Ok(())
}
