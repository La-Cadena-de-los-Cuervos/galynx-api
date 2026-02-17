use std::{collections::HashMap, sync::Arc};

use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{RwLock, broadcast};
use tracing::warn;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::AuthContext,
    channels::{CreateMessageRequest, MessageQuery, UpdateMessageRequest},
    errors::{ApiError, ApiResult, ErrorResponse},
    rate_limit::client_ip_from_headers,
};

#[derive(Clone)]
pub struct RealtimeHub {
    workspaces: Arc<RwLock<HashMap<Uuid, broadcast::Sender<WsEventEnvelope>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WsEventEnvelope {
    pub event_type: String,
    pub workspace_id: Option<Uuid>,
    pub channel_id: Option<Uuid>,
    pub correlation_id: Option<String>,
    pub server_ts: i64,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
struct WsCommandEnvelope {
    command: String,
    payload: Value,
    client_msg_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SendMessagePayload {
    channel_id: Uuid,
    body_md: String,
}

#[derive(Debug, Deserialize)]
struct EditMessagePayload {
    message_id: Uuid,
    body_md: String,
}

#[derive(Debug, Deserialize)]
struct DeleteMessagePayload {
    message_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct FetchMorePayload {
    channel_id: Uuid,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FetchThreadPayload {
    root_id: Uuid,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReactionPayload {
    message_id: Uuid,
    emoji: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/ws", get(ws_upgrade))
}

impl RealtimeHub {
    pub fn new() -> Self {
        Self {
            workspaces: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn subscribe(&self, workspace_id: Uuid) -> broadcast::Receiver<WsEventEnvelope> {
        let sender = {
            let mut workspaces = self.workspaces.write().await;
            workspaces
                .entry(workspace_id)
                .or_insert_with(|| broadcast::channel::<WsEventEnvelope>(1024).0)
                .clone()
        };
        sender.subscribe()
    }

    pub async fn emit(&self, workspace_id: Uuid, event: WsEventEnvelope) {
        let sender = {
            let mut workspaces = self.workspaces.write().await;
            workspaces
                .entry(workspace_id)
                .or_insert_with(|| broadcast::channel::<WsEventEnvelope>(1024).0)
                .clone()
        };
        let _ = sender.send(event);
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/ws",
    responses(
        (status = 101, description = "WebSocket upgraded"),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let client_ip = client_ip_from_headers(&headers);
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    state
        .rate_limit
        .check_ws_connect(&client_ip, context.user_id)
        .await?;

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, context)))
}

async fn handle_socket(mut socket: WebSocket, state: AppState, context: AuthContext) {
    let mut rx = state.realtime.subscribe(context.workspace_id).await;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "WS_CONNECTED",
            "session",
            None,
            json!({ "transport": "websocket" }),
        )
        .await;

    let welcome = WsEventEnvelope {
        event_type: "WELCOME".to_string(),
        workspace_id: Some(context.workspace_id),
        channel_id: None,
        correlation_id: None,
        server_ts: Utc::now().timestamp_millis(),
        payload: json!({
            "user_id": context.user_id,
            "role": context.role,
        }),
    };
    if socket
        .send(Message::Text(
            serde_json::to_string(&welcome).unwrap_or_default(),
        ))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            outbound = rx.recv() => {
                match outbound {
                    Ok(event) => {
                        if socket
                            .send(Message::Text(serde_json::to_string(&event).unwrap_or_default()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("websocket lagged, skipped {} messages", skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            inbound = socket.recv() => {
                let Some(inbound) = inbound else { return; };
                match inbound {
                    Ok(Message::Text(text)) => {
                        if let Err(error) = handle_client_text(&state, &context, &mut socket, &text).await {
                            let _ = socket.send(Message::Text(error_event(error))).await;
                        }
                    }
                    Ok(Message::Close(_)) => return,
                    Ok(Message::Ping(payload)) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        }
    }
}

async fn handle_client_text(
    state: &AppState,
    context: &AuthContext,
    socket: &mut WebSocket,
    text: &str,
) -> ApiResult<()> {
    state.rate_limit.check_ws_command(context.user_id).await?;

    let command: WsCommandEnvelope = serde_json::from_str(text)
        .map_err(|_| ApiError::BadRequest("invalid websocket command payload".to_string()))?;

    match command.command.as_str() {
        "SEND_MESSAGE" => {
            let payload: SendMessagePayload = serde_json::from_value(command.payload.clone())
                .map_err(|_| ApiError::BadRequest("invalid SEND_MESSAGE payload".to_string()))?;
            let message = state
                .channels
                .create_message(
                    context,
                    payload.channel_id,
                    CreateMessageRequest {
                        body_md: payload.body_md,
                    },
                )
                .await?;
            state
                .realtime
                .emit(
                    context.workspace_id,
                    event(
                        "MESSAGE_CREATED",
                        context.workspace_id,
                        Some(message.channel_id),
                        command.client_msg_id.clone(),
                        serde_json::to_value(&message).unwrap_or_default(),
                    ),
                )
                .await;
            state
                .audit
                .write(
                    context.workspace_id,
                    Some(context.user_id),
                    "MESSAGE_CREATED_WS",
                    "message",
                    Some(message.id.to_string()),
                    json!({ "channel_id": message.channel_id, "client_msg_id": command.client_msg_id.clone() }),
                )
                .await;
            send_ack(
                socket,
                "SEND_MESSAGE",
                command.client_msg_id,
                json!({"message_id": message.id}),
            )
            .await?;
        }
        "EDIT_MESSAGE" => {
            let payload: EditMessagePayload = serde_json::from_value(command.payload.clone())
                .map_err(|_| ApiError::BadRequest("invalid EDIT_MESSAGE payload".to_string()))?;
            let message = state
                .channels
                .update_message(
                    context,
                    payload.message_id,
                    UpdateMessageRequest {
                        body_md: payload.body_md,
                    },
                )
                .await?;
            state
                .realtime
                .emit(
                    context.workspace_id,
                    event(
                        "MESSAGE_UPDATED",
                        context.workspace_id,
                        Some(message.channel_id),
                        command.client_msg_id.clone(),
                        serde_json::to_value(&message).unwrap_or_default(),
                    ),
                )
                .await;
            state
                .audit
                .write(
                    context.workspace_id,
                    Some(context.user_id),
                    "MESSAGE_UPDATED_WS",
                    "message",
                    Some(message.id.to_string()),
                    json!({ "channel_id": message.channel_id, "client_msg_id": command.client_msg_id.clone() }),
                )
                .await;
            send_ack(
                socket,
                "EDIT_MESSAGE",
                command.client_msg_id,
                json!({"message_id": message.id}),
            )
            .await?;
        }
        "DELETE_MESSAGE" => {
            let payload: DeleteMessagePayload = serde_json::from_value(command.payload.clone())
                .map_err(|_| ApiError::BadRequest("invalid DELETE_MESSAGE payload".to_string()))?;
            let target = state
                .channels
                .get_message(context.workspace_id, payload.message_id)
                .await?;
            state
                .channels
                .delete_message(context, payload.message_id)
                .await?;
            state
                .realtime
                .emit(
                    context.workspace_id,
                    event(
                        "MESSAGE_DELETED",
                        context.workspace_id,
                        Some(target.channel_id),
                        command.client_msg_id.clone(),
                        json!({"message_id": payload.message_id}),
                    ),
                )
                .await;
            state
                .audit
                .write(
                    context.workspace_id,
                    Some(context.user_id),
                    "MESSAGE_DELETED_WS",
                    "message",
                    Some(payload.message_id.to_string()),
                    json!({ "channel_id": target.channel_id, "client_msg_id": command.client_msg_id.clone() }),
                )
                .await;
            send_ack(
                socket,
                "DELETE_MESSAGE",
                command.client_msg_id,
                json!({"message_id": payload.message_id}),
            )
            .await?;
        }
        "FETCH_MORE" => {
            let payload: FetchMorePayload = serde_json::from_value(command.payload.clone())
                .map_err(|_| ApiError::BadRequest("invalid FETCH_MORE payload".to_string()))?;
            let page = state
                .channels
                .list_messages(
                    context.workspace_id,
                    payload.channel_id,
                    &MessageQuery {
                        cursor: payload.cursor,
                        limit: payload.limit,
                    },
                )
                .await?;
            send_ack(
                socket,
                "FETCH_MORE",
                command.client_msg_id,
                serde_json::to_value(page).unwrap_or_default(),
            )
            .await?;
        }
        "FETCH_THREAD" => {
            let payload: FetchThreadPayload = serde_json::from_value(command.payload.clone())
                .map_err(|_| ApiError::BadRequest("invalid FETCH_THREAD payload".to_string()))?;
            let summary = state
                .channels
                .thread_summary(context.workspace_id, payload.root_id)
                .await?;
            let replies = state
                .channels
                .list_thread_replies(
                    context.workspace_id,
                    payload.root_id,
                    &MessageQuery {
                        cursor: payload.cursor,
                        limit: payload.limit,
                    },
                )
                .await?;
            send_ack(
                socket,
                "FETCH_THREAD",
                command.client_msg_id,
                json!({"summary": summary, "replies": replies}),
            )
            .await?;
        }
        "ADD_REACTION" => {
            let payload: ReactionPayload = serde_json::from_value(command.payload.clone())
                .map_err(|_| ApiError::BadRequest("invalid ADD_REACTION payload".to_string()))?;
            let update = state
                .reactions
                .add_reaction(&state.channels, context, payload.message_id, &payload.emoji)
                .await?;
            state
                .realtime
                .emit(
                    context.workspace_id,
                    event(
                        "REACTION_UPDATED",
                        context.workspace_id,
                        Some(update.channel_id),
                        command.client_msg_id.clone(),
                        serde_json::to_value(&update).unwrap_or_default(),
                    ),
                )
                .await;
            state
                .audit
                .write(
                    context.workspace_id,
                    Some(context.user_id),
                    "REACTION_ADDED_WS",
                    "message",
                    Some(update.message_id.to_string()),
                    json!({ "emoji": update.emoji, "client_msg_id": command.client_msg_id.clone() }),
                )
                .await;
            send_ack(
                socket,
                "ADD_REACTION",
                command.client_msg_id,
                json!({"ok": true}),
            )
            .await?;
        }
        "REMOVE_REACTION" => {
            let payload: ReactionPayload = serde_json::from_value(command.payload.clone())
                .map_err(|_| ApiError::BadRequest("invalid REMOVE_REACTION payload".to_string()))?;
            let update = state
                .reactions
                .remove_reaction(&state.channels, context, payload.message_id, &payload.emoji)
                .await?;
            state
                .realtime
                .emit(
                    context.workspace_id,
                    event(
                        "REACTION_UPDATED",
                        context.workspace_id,
                        Some(update.channel_id),
                        command.client_msg_id.clone(),
                        serde_json::to_value(&update).unwrap_or_default(),
                    ),
                )
                .await;
            state
                .audit
                .write(
                    context.workspace_id,
                    Some(context.user_id),
                    "REACTION_REMOVED_WS",
                    "message",
                    Some(update.message_id.to_string()),
                    json!({ "emoji": update.emoji, "client_msg_id": command.client_msg_id.clone() }),
                )
                .await;
            send_ack(
                socket,
                "REMOVE_REACTION",
                command.client_msg_id,
                json!({"ok": true}),
            )
            .await?;
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "unsupported websocket command: {other}"
            )));
        }
    }
    Ok(())
}

fn event(
    event_type: &str,
    workspace_id: Uuid,
    channel_id: Option<Uuid>,
    correlation_id: Option<String>,
    payload: Value,
) -> WsEventEnvelope {
    WsEventEnvelope {
        event_type: event_type.to_string(),
        workspace_id: Some(workspace_id),
        channel_id,
        correlation_id,
        server_ts: Utc::now().timestamp_millis(),
        payload,
    }
}

pub fn make_event(
    event_type: &str,
    workspace_id: Uuid,
    channel_id: Option<Uuid>,
    correlation_id: Option<String>,
    payload: Value,
) -> WsEventEnvelope {
    event(
        event_type,
        workspace_id,
        channel_id,
        correlation_id,
        payload,
    )
}

async fn send_ack(
    socket: &mut WebSocket,
    command: &str,
    correlation_id: Option<String>,
    payload: Value,
) -> ApiResult<()> {
    let ack = WsEventEnvelope {
        event_type: "ACK".to_string(),
        workspace_id: None,
        channel_id: None,
        correlation_id,
        server_ts: Utc::now().timestamp_millis(),
        payload: json!({
            "command": command,
            "result": payload,
        }),
    };

    socket
        .send(Message::Text(
            serde_json::to_string(&ack).unwrap_or_default(),
        ))
        .await
        .map_err(|_| ApiError::Internal("failed to send websocket ack".to_string()))?;
    Ok(())
}

fn error_event(error: ApiError) -> String {
    let body = json!({
        "event_type": "ERROR",
        "server_ts": Utc::now().timestamp_millis(),
        "payload": {
            "status": status_from_error(&error),
            "error": error.to_string(),
        }
    });
    serde_json::to_string(&body).unwrap_or_else(|_| "{\"event_type\":\"ERROR\"}".to_string())
}

fn status_from_error(error: &ApiError) -> u16 {
    match error {
        ApiError::Unauthorized(_) => StatusCode::UNAUTHORIZED.as_u16(),
        ApiError::BadRequest(_) => StatusCode::BAD_REQUEST.as_u16(),
        ApiError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS.as_u16(),
        ApiError::NotFound(_) => StatusCode::NOT_FOUND.as_u16(),
        ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
    }
}
