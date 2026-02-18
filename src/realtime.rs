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
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    sync::{RwLock, broadcast, mpsc},
    time::{Duration, sleep},
};
use tracing::{info, warn};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::AuthContext,
    channels::{CreateMessageRequest, MessageQuery, UpdateMessageRequest},
    errors::{ApiError, ApiResult, ErrorResponse},
    rate_limit::client_ip_from_headers,
};

const REDIS_WS_CHANNEL: &str = "galynx:ws:events";

#[derive(Clone)]
pub struct RealtimeHub {
    workspaces: Arc<RwLock<HashMap<Uuid, broadcast::Sender<WsEventEnvelope>>>>,
    instance_id: String,
    redis_outbox: Option<mpsc::UnboundedSender<String>>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RedisEventEnvelope {
    source_instance_id: String,
    event: WsEventEnvelope,
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
    pub fn new(redis_url: Option<&str>) -> Self {
        let workspaces = Arc::new(RwLock::new(HashMap::new()));
        let instance_id = Uuid::new_v4().to_string();

        let redis_outbox = redis_url
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let (tx, rx) = mpsc::unbounded_channel::<String>();
                spawn_redis_publisher(value.to_string(), rx);
                spawn_redis_subscriber(value.to_string(), workspaces.clone(), instance_id.clone());
                tx
            });

        if redis_outbox.is_some() {
            info!("realtime redis bridge enabled");
        } else {
            info!("realtime redis bridge disabled (REDIS_URL not set)");
        }

        Self {
            workspaces,
            instance_id,
            redis_outbox,
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
        self.emit_local(workspace_id, event.clone()).await;

        let Some(redis_outbox) = &self.redis_outbox else {
            return;
        };

        let payload = RedisEventEnvelope {
            source_instance_id: self.instance_id.clone(),
            event,
        };
        match serde_json::to_string(&payload) {
            Ok(serialized) => {
                let _ = redis_outbox.send(serialized);
            }
            Err(error) => {
                warn!("failed to serialize realtime redis payload: {}", error);
            }
        }
    }

    async fn emit_local(&self, workspace_id: Uuid, event: WsEventEnvelope) {
        emit_workspace_event(&self.workspaces, workspace_id, event).await;
    }
}

fn spawn_redis_publisher(redis_url: String, mut rx: mpsc::UnboundedReceiver<String>) {
    tokio::spawn(async move {
        while let Some(payload) = rx.recv().await {
            loop {
                match publish_redis_event(&redis_url, &payload).await {
                    Ok(()) => break,
                    Err(error) => {
                        warn!("redis publish failed, retrying: {}", error);
                        sleep(Duration::from_millis(400)).await;
                    }
                }
            }
        }
    });
}

async fn publish_redis_event(redis_url: &str, payload: &str) -> Result<(), String> {
    let client =
        redis::Client::open(redis_url).map_err(|error| format!("invalid redis url: {error}"))?;
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|error| format!("redis connection error: {error}"))?;

    redis::cmd("PUBLISH")
        .arg(REDIS_WS_CHANNEL)
        .arg(payload)
        .query_async::<usize>(&mut connection)
        .await
        .map_err(|error| format!("redis publish command failed: {error}"))?;

    Ok(())
}

fn spawn_redis_subscriber(
    redis_url: String,
    workspaces: Arc<RwLock<HashMap<Uuid, broadcast::Sender<WsEventEnvelope>>>>,
    instance_id: String,
) {
    tokio::spawn(async move {
        loop {
            if let Err(error) =
                run_redis_subscriber(&redis_url, workspaces.clone(), &instance_id).await
            {
                warn!("redis subscriber failed, reconnecting: {}", error);
                sleep(Duration::from_secs(1)).await;
            }
        }
    });
}

async fn run_redis_subscriber(
    redis_url: &str,
    workspaces: Arc<RwLock<HashMap<Uuid, broadcast::Sender<WsEventEnvelope>>>>,
    instance_id: &str,
) -> Result<(), String> {
    let client =
        redis::Client::open(redis_url).map_err(|error| format!("invalid redis url: {error}"))?;
    let mut pubsub = client
        .get_async_pubsub()
        .await
        .map_err(|error| format!("redis pubsub connection error: {error}"))?;

    pubsub
        .subscribe(REDIS_WS_CHANNEL)
        .await
        .map_err(|error| format!("redis subscribe failed: {error}"))?;

    let mut stream = pubsub.on_message();
    while let Some(message) = stream.next().await {
        let payload = message
            .get_payload::<String>()
            .map_err(|error| format!("invalid redis payload: {error}"))?;

        let envelope: RedisEventEnvelope = match serde_json::from_str(&payload) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if envelope.source_instance_id == instance_id {
            continue;
        }

        let Some(workspace_id) = envelope.event.workspace_id else {
            continue;
        };

        emit_workspace_event(&workspaces, workspace_id, envelope.event).await;
    }

    Ok(())
}

async fn emit_workspace_event(
    workspaces: &Arc<RwLock<HashMap<Uuid, broadcast::Sender<WsEventEnvelope>>>>,
    workspace_id: Uuid,
    event: WsEventEnvelope,
) {
    let sender = {
        let mut map = workspaces.write().await;
        map.entry(workspace_id)
            .or_insert_with(|| broadcast::channel::<WsEventEnvelope>(1024).0)
            .clone()
    };
    let _ = sender.send(event);
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
            let dedup_client_msg_id = normalize_client_msg_id(command.client_msg_id.as_deref())?;
            if let Some(client_msg_id) = dedup_client_msg_id.as_deref() {
                if let Some(existing_message_id) = state
                    .storage
                    .get_ws_command_message_id(
                        context.workspace_id,
                        context.user_id,
                        payload.channel_id,
                        client_msg_id,
                    )
                    .await
                {
                    if state
                        .channels
                        .get_message(context.workspace_id, existing_message_id)
                        .await
                        .is_ok()
                    {
                        send_ack(
                            socket,
                            "SEND_MESSAGE",
                            command.client_msg_id,
                            json!({"message_id": existing_message_id, "deduped": true}),
                        )
                        .await?;
                        return Ok(());
                    }
                }
            }

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
            if let Some(client_msg_id) = dedup_client_msg_id.as_deref() {
                state
                    .storage
                    .put_ws_command_message_id(
                        context.workspace_id,
                        context.user_id,
                        payload.channel_id,
                        client_msg_id,
                        message.id,
                        Utc::now().timestamp_millis(),
                    )
                    .await;
            }
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
            if let Some(client_msg_id) = normalize_client_msg_id(command.client_msg_id.as_deref())?
            {
                let dedup_key = ws_command_once_key(
                    context.workspace_id,
                    context.user_id,
                    "EDIT_MESSAGE",
                    &format!("message:{}", payload.message_id),
                    &client_msg_id,
                );
                if state.storage.has_ws_command_once(&dedup_key).await {
                    send_ack(
                        socket,
                        "EDIT_MESSAGE",
                        command.client_msg_id,
                        json!({"message_id": payload.message_id, "deduped": true}),
                    )
                    .await?;
                    return Ok(());
                }
                state
                    .storage
                    .put_ws_command_once(&dedup_key, Utc::now().timestamp_millis())
                    .await;
            }
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
            if let Some(client_msg_id) = normalize_client_msg_id(command.client_msg_id.as_deref())?
            {
                let dedup_key = ws_command_once_key(
                    context.workspace_id,
                    context.user_id,
                    "DELETE_MESSAGE",
                    &format!("message:{}", payload.message_id),
                    &client_msg_id,
                );
                if state.storage.has_ws_command_once(&dedup_key).await {
                    send_ack(
                        socket,
                        "DELETE_MESSAGE",
                        command.client_msg_id,
                        json!({"message_id": payload.message_id, "deduped": true}),
                    )
                    .await?;
                    return Ok(());
                }
                state
                    .storage
                    .put_ws_command_once(&dedup_key, Utc::now().timestamp_millis())
                    .await;
            }
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
            if let Some(client_msg_id) = normalize_client_msg_id(command.client_msg_id.as_deref())?
            {
                let dedup_key = ws_command_once_key(
                    context.workspace_id,
                    context.user_id,
                    "ADD_REACTION",
                    &format!("reaction:{}:{}", payload.message_id, payload.emoji.trim()),
                    &client_msg_id,
                );
                if state.storage.has_ws_command_once(&dedup_key).await {
                    send_ack(
                        socket,
                        "ADD_REACTION",
                        command.client_msg_id,
                        json!({"ok": true, "deduped": true}),
                    )
                    .await?;
                    return Ok(());
                }
                state
                    .storage
                    .put_ws_command_once(&dedup_key, Utc::now().timestamp_millis())
                    .await;
            }
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
            if let Some(client_msg_id) = normalize_client_msg_id(command.client_msg_id.as_deref())?
            {
                let dedup_key = ws_command_once_key(
                    context.workspace_id,
                    context.user_id,
                    "REMOVE_REACTION",
                    &format!("reaction:{}:{}", payload.message_id, payload.emoji.trim()),
                    &client_msg_id,
                );
                if state.storage.has_ws_command_once(&dedup_key).await {
                    send_ack(
                        socket,
                        "REMOVE_REACTION",
                        command.client_msg_id,
                        json!({"ok": true, "deduped": true}),
                    )
                    .await?;
                    return Ok(());
                }
                state
                    .storage
                    .put_ws_command_once(&dedup_key, Utc::now().timestamp_millis())
                    .await;
            }
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

fn normalize_client_msg_id(value: Option<&str>) -> ApiResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(ApiError::BadRequest(
            "client_msg_id must not be empty".to_string(),
        ));
    }
    if normalized.len() > 128 {
        return Err(ApiError::BadRequest(
            "client_msg_id is too long".to_string(),
        ));
    }
    Ok(Some(normalized.to_string()))
}

fn ws_command_once_key(
    workspace_id: Uuid,
    user_id: Uuid,
    command: &str,
    target: &str,
    client_msg_id: &str,
) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        workspace_id,
        user_id,
        command.trim(),
        target.trim(),
        client_msg_id.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::normalize_client_msg_id;

    #[test]
    fn normalize_client_msg_id_accepts_trimmed_value() {
        let value = normalize_client_msg_id(Some("  abc-123  ")).expect("should be valid");
        assert_eq!(value.as_deref(), Some("abc-123"));
    }

    #[test]
    fn normalize_client_msg_id_rejects_empty_value() {
        let error = normalize_client_msg_id(Some("   ")).expect_err("should fail");
        assert_eq!(error.to_string(), "client_msg_id must not be empty");
    }
}
