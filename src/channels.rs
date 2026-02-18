use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, patch},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthContext, WorkspaceRole},
    errors::{ApiError, ApiResult, ErrorResponse},
    realtime,
    storage::{ChannelRecordStore, MessageRecordStore, Storage},
};

#[derive(Clone)]
pub struct ChannelService {
    storage: Arc<Storage>,
    bootstrap_workspace_id: Uuid,
    bootstrap_creator_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub is_private: bool,
    pub created_by: Uuid,
    pub created_at: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateChannelRequest {
    pub name: String,
    pub is_private: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMessageRequest {
    pub body_md: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMessageRequest {
    pub body_md: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub channel_id: Uuid,
    pub sender_id: Uuid,
    pub body_md: String,
    pub thread_root_id: Option<Uuid>,
    pub created_at: i64,
    pub edited_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ThreadSummaryResponse {
    pub root_message: MessageResponse,
    pub reply_count: usize,
    pub last_reply_at: Option<i64>,
    pub participants: Vec<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageListResponse {
    pub items: Vec<MessageResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelMemberResponse {
    pub user_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddChannelMemberRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct MessageQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/channels", get(list_channels).post(create_channel))
        .route("/api/v1/channels/:id", delete(delete_channel))
        .route(
            "/api/v1/channels/:id/members",
            get(list_channel_members).post(add_channel_member),
        )
        .route(
            "/api/v1/channels/:id/members/:user_id",
            delete(remove_channel_member),
        )
        .route(
            "/api/v1/channels/:id/messages",
            get(list_messages).post(create_message),
        )
        .route(
            "/api/v1/messages/:id",
            patch(update_message).delete(delete_message),
        )
}

impl ChannelService {
    pub fn new(storage: Arc<Storage>, workspace_id: Uuid, creator_id: Uuid) -> Self {
        Self {
            storage,
            bootstrap_workspace_id: workspace_id,
            bootstrap_creator_id: creator_id,
        }
    }

    pub async fn list_channels(&self, workspace_id: Uuid) -> Vec<ChannelResponse> {
        self.ensure_bootstrap_seed().await;
        let channels = self.storage.list_channels(workspace_id).await;
        let mut items: Vec<ChannelResponse> = channels.iter().map(ChannelResponse::from).collect();
        items.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        items
    }

    pub async fn create_channel(
        &self,
        workspace_id: Uuid,
        created_by: Uuid,
        payload: CreateChannelRequest,
    ) -> ApiResult<ChannelResponse> {
        self.ensure_bootstrap_seed().await;
        let name = payload.name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err(ApiError::BadRequest("channel name is required".to_string()));
        }

        if self.storage.channel_name_exists(workspace_id, &name).await {
            return Err(ApiError::BadRequest(
                "channel name already exists".to_string(),
            ));
        }

        let channel = ChannelRecordStore {
            id: Uuid::new_v4(),
            workspace_id,
            name,
            is_private: payload.is_private,
            created_by,
            created_at: Utc::now().timestamp_millis(),
        };
        let response = ChannelResponse::from(&channel);
        self.storage.insert_channel(channel.clone()).await;
        if channel.is_private {
            self.storage.add_channel_member(channel.id, created_by).await;
        }
        Ok(response)
    }

    pub async fn delete_channel(&self, workspace_id: Uuid, channel_id: Uuid) -> ApiResult<()> {
        self.ensure_bootstrap_seed().await;
        let Some(channel) = self.storage.get_channel(&channel_id).await else {
            return Err(ApiError::NotFound("channel not found".to_string()));
        };
        if channel.workspace_id != workspace_id {
            return Err(ApiError::NotFound("channel not found".to_string()));
        }

        self.storage.remove_channel(&channel_id).await;
        self.storage.remove_channel_members(channel_id).await;
        self.storage.remove_messages_for_channel(channel_id).await;
        Ok(())
    }

    pub async fn list_channel_members(
        &self,
        workspace_id: Uuid,
        channel_id: Uuid,
    ) -> ApiResult<Vec<ChannelMemberResponse>> {
        self.ensure_bootstrap_seed().await;
        let channel = self
            .storage
            .get_channel(&channel_id)
            .await
            .ok_or_else(|| ApiError::NotFound("channel not found".to_string()))?;
        if channel.workspace_id != workspace_id {
            return Err(ApiError::NotFound("channel not found".to_string()));
        }

        let mut users = self.storage.list_channel_members(channel_id).await;
        users.sort_unstable();
        users.dedup();
        Ok(users
            .into_iter()
            .map(|user_id| ChannelMemberResponse { user_id })
            .collect())
    }

    pub async fn add_channel_member(
        &self,
        workspace_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> ApiResult<()> {
        self.ensure_bootstrap_seed().await;
        let channel = self
            .storage
            .get_channel(&channel_id)
            .await
            .ok_or_else(|| ApiError::NotFound("channel not found".to_string()))?;
        if channel.workspace_id != workspace_id {
            return Err(ApiError::NotFound("channel not found".to_string()));
        }

        let membership = self.storage.get_membership_role(workspace_id, user_id).await;
        if membership.is_none() {
            return Err(ApiError::BadRequest(
                "user does not belong to workspace".to_string(),
            ));
        }
        self.storage.add_channel_member(channel_id, user_id).await;
        Ok(())
    }

    pub async fn remove_channel_member(
        &self,
        workspace_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> ApiResult<()> {
        self.ensure_bootstrap_seed().await;
        let channel = self
            .storage
            .get_channel(&channel_id)
            .await
            .ok_or_else(|| ApiError::NotFound("channel not found".to_string()))?;
        if channel.workspace_id != workspace_id {
            return Err(ApiError::NotFound("channel not found".to_string()));
        }
        self.storage.remove_channel_member(channel_id, user_id).await;
        Ok(())
    }

    pub async fn create_message(
        &self,
        context: &AuthContext,
        channel_id: Uuid,
        payload: CreateMessageRequest,
    ) -> ApiResult<MessageResponse> {
        self.ensure_bootstrap_seed().await;
        let body = payload.body_md.trim().to_string();
        if body.is_empty() {
            return Err(ApiError::BadRequest("message body is required".to_string()));
        }

        self.assert_channel_access(context, channel_id).await?;

        let message = MessageRecordStore {
            id: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            workspace_id: context.workspace_id,
            channel_id,
            sender_id: context.user_id,
            body_md: body,
            thread_root_id: None,
            created_at: Utc::now().timestamp_millis(),
            edited_at: None,
            deleted_at: None,
        };

        let response = MessageResponse::from(&message);
        self.storage.insert_message(message).await;
        Ok(response)
    }

    pub async fn list_messages(
        &self,
        context: &AuthContext,
        channel_id: Uuid,
        query: &MessageQuery,
    ) -> ApiResult<MessageListResponse> {
        self.ensure_bootstrap_seed().await;
        self.assert_channel_access(context, channel_id).await?;

        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let before = query
            .cursor
            .as_deref()
            .map(parse_cursor)
            .transpose()
            .map_err(|error| ApiError::BadRequest(format!("invalid cursor: {error}")))?;

        let messages = self.storage.list_messages(context.workspace_id).await;
        let mut channel_messages: Vec<&MessageRecordStore> = messages
            .iter()
            .filter(|message| message.channel_id == channel_id && message.deleted_at.is_none())
            .collect();
        channel_messages.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.as_u128().cmp(&a.id.as_u128()))
        });

        let filtered = channel_messages
            .into_iter()
            .filter(|message| {
                before.is_none_or(|(cursor_ts, cursor_id)| {
                    (message.created_at, message.id.as_u128()) < (cursor_ts, cursor_id)
                })
            })
            .take(limit + 1)
            .collect::<Vec<_>>();

        let has_more = filtered.len() > limit;
        let items = filtered
            .into_iter()
            .take(limit)
            .map(MessageResponse::from)
            .collect::<Vec<_>>();
        let next_cursor = if has_more {
            items
                .last()
                .map(|message| format!("{}:{}", message.created_at, message.id.as_u128()))
        } else {
            None
        };

        Ok(MessageListResponse { items, next_cursor })
    }

    pub async fn update_message(
        &self,
        context: &AuthContext,
        message_id: Uuid,
        payload: UpdateMessageRequest,
    ) -> ApiResult<MessageResponse> {
        self.ensure_bootstrap_seed().await;
        let body = payload.body_md.trim().to_string();
        if body.is_empty() {
            return Err(ApiError::BadRequest("message body is required".to_string()));
        }

        let mut message = self
            .storage
            .get_message(&message_id)
            .await
            .ok_or_else(|| ApiError::NotFound("message not found".to_string()))?;

        if message.workspace_id != context.workspace_id {
            return Err(ApiError::NotFound("message not found".to_string()));
        }
        if message.sender_id != context.user_id {
            return Err(ApiError::Unauthorized(
                "you can only edit your own messages".to_string(),
            ));
        }

        message.body_md = body;
        message.edited_at = Some(Utc::now().timestamp_millis());
        self.storage.update_message(message.clone()).await;
        Ok(MessageResponse::from(&message))
    }

    pub async fn delete_message(&self, context: &AuthContext, message_id: Uuid) -> ApiResult<()> {
        self.ensure_bootstrap_seed().await;
        let mut message = self
            .storage
            .get_message(&message_id)
            .await
            .ok_or_else(|| ApiError::NotFound("message not found".to_string()))?;

        if message.workspace_id != context.workspace_id {
            return Err(ApiError::NotFound("message not found".to_string()));
        }
        let can_delete_other = matches!(context.role, WorkspaceRole::Owner | WorkspaceRole::Admin);
        if message.sender_id != context.user_id && !can_delete_other {
            return Err(ApiError::Unauthorized(
                "you do not have permission to delete this message".to_string(),
            ));
        }

        message.deleted_at = Some(Utc::now().timestamp_millis());
        self.storage.update_message(message).await;
        Ok(())
    }

    pub async fn get_message(
        &self,
        workspace_id: Uuid,
        message_id: Uuid,
    ) -> ApiResult<MessageResponse> {
        self.ensure_bootstrap_seed().await;
        let message = self
            .storage
            .get_message(&message_id)
            .await
            .ok_or_else(|| ApiError::NotFound("message not found".to_string()))?;
        if message.workspace_id != workspace_id || message.deleted_at.is_some() {
            return Err(ApiError::NotFound("message not found".to_string()));
        }
        Ok(MessageResponse::from(&message))
    }

    pub async fn ensure_channel_access(&self, context: &AuthContext, channel_id: Uuid) -> ApiResult<()> {
        self.assert_channel_access(context, channel_id).await
    }

    pub async fn thread_summary(
        &self,
        context: &AuthContext,
        root_id: Uuid,
    ) -> ApiResult<ThreadSummaryResponse> {
        self.ensure_bootstrap_seed().await;
        let root_message = self.assert_thread_root(context, root_id).await?;
        let messages = self.storage.list_messages(context.workspace_id).await;

        let mut reply_count = 0usize;
        let mut last_reply_at = None;
        let mut participants = vec![root_message.sender_id];
        for message in messages.iter().filter(|message| {
            message.thread_root_id == Some(root_id) && message.deleted_at.is_none()
        }) {
            reply_count += 1;
            if last_reply_at.is_none_or(|last| message.created_at > last) {
                last_reply_at = Some(message.created_at);
            }
            if !participants.contains(&message.sender_id) {
                participants.push(message.sender_id);
            }
        }

        Ok(ThreadSummaryResponse {
            root_message: MessageResponse::from(&root_message),
            reply_count,
            last_reply_at,
            participants,
        })
    }

    pub async fn list_thread_replies(
        &self,
        context: &AuthContext,
        root_id: Uuid,
        query: &MessageQuery,
    ) -> ApiResult<MessageListResponse> {
        self.ensure_bootstrap_seed().await;
        self.assert_thread_root(context, root_id).await?;

        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let before = query
            .cursor
            .as_deref()
            .map(parse_cursor)
            .transpose()
            .map_err(|error| ApiError::BadRequest(format!("invalid cursor: {error}")))?;

        let messages = self.storage.list_messages(context.workspace_id).await;
        let mut replies: Vec<&MessageRecordStore> = messages
            .iter()
            .filter(|message| {
                message.thread_root_id == Some(root_id) && message.deleted_at.is_none()
            })
            .collect();
        replies.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.as_u128().cmp(&a.id.as_u128()))
        });

        let filtered = replies
            .into_iter()
            .filter(|message| {
                before.is_none_or(|(cursor_ts, cursor_id)| {
                    (message.created_at, message.id.as_u128()) < (cursor_ts, cursor_id)
                })
            })
            .take(limit + 1)
            .collect::<Vec<_>>();
        let has_more = filtered.len() > limit;
        let items = filtered
            .into_iter()
            .take(limit)
            .map(MessageResponse::from)
            .collect::<Vec<_>>();
        let next_cursor = if has_more {
            items
                .last()
                .map(|message| format!("{}:{}", message.created_at, message.id.as_u128()))
        } else {
            None
        };

        Ok(MessageListResponse { items, next_cursor })
    }

    pub async fn create_thread_reply(
        &self,
        context: &AuthContext,
        root_id: Uuid,
        payload: CreateMessageRequest,
    ) -> ApiResult<MessageResponse> {
        self.ensure_bootstrap_seed().await;
        let body = payload.body_md.trim().to_string();
        if body.is_empty() {
            return Err(ApiError::BadRequest("message body is required".to_string()));
        }

        let (workspace_id, channel_id) = {
            let messages = self.storage.list_messages(context.workspace_id).await;
            let root = messages
                .iter()
                .find(|message| message.id == root_id)
                .ok_or_else(|| ApiError::NotFound("thread root not found".to_string()))?;
            if root.thread_root_id.is_some() {
                return Err(ApiError::BadRequest(
                    "thread replies must reference root message".to_string(),
                ));
            }
            (root.workspace_id, root.channel_id)
        };
        if workspace_id != context.workspace_id {
            return Err(ApiError::NotFound("thread root not found".to_string()));
        }
        self.assert_channel_access(context, channel_id).await?;

        let reply = MessageRecordStore {
            id: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            workspace_id: context.workspace_id,
            channel_id,
            sender_id: context.user_id,
            body_md: body,
            thread_root_id: Some(root_id),
            created_at: Utc::now().timestamp_millis(),
            edited_at: None,
            deleted_at: None,
        };

        let response = MessageResponse::from(&reply);
        self.storage.insert_message(reply).await;
        Ok(response)
    }

    async fn assert_channel_access(&self, context: &AuthContext, channel_id: Uuid) -> ApiResult<()> {
        let channel = self
            .storage
            .get_channel(&channel_id)
            .await
            .ok_or_else(|| ApiError::NotFound("channel not found".to_string()))?;

        if channel.workspace_id != context.workspace_id {
            return Err(ApiError::NotFound("channel not found".to_string()));
        }
        if channel.is_private {
            let can_bypass = matches!(context.role, WorkspaceRole::Owner | WorkspaceRole::Admin);
            if !can_bypass
                && !self
                    .storage
                    .is_channel_member(channel_id, context.user_id)
                    .await
            {
                return Err(ApiError::Unauthorized(
                    "you do not have access to this private channel".to_string(),
                ));
            }
        }

        Ok(())
    }

    async fn assert_thread_root(&self, context: &AuthContext, root_id: Uuid) -> ApiResult<MessageRecordStore> {
        let root = self
            .storage
            .get_message(&root_id)
            .await
            .ok_or_else(|| ApiError::NotFound("thread root not found".to_string()))?;
        if root.workspace_id != context.workspace_id || root.thread_root_id.is_some() {
            return Err(ApiError::NotFound("thread root not found".to_string()));
        }
        self.assert_channel_access(context, root.channel_id).await?;
        Ok(root)
    }

    async fn ensure_bootstrap_seed(&self) {
        if matches!(
            self.storage.backend(),
            crate::storage::PersistenceBackend::Mongo
        ) {
            return;
        }
        let has_bootstrap_channel = !self
            .storage
            .list_channels(self.bootstrap_workspace_id)
            .await
            .is_empty();
        if has_bootstrap_channel {
            return;
        }

        let channel = ChannelRecordStore {
            id: Uuid::new_v4(),
            workspace_id: self.bootstrap_workspace_id,
            name: "general".to_string(),
            is_private: false,
            created_by: self.bootstrap_creator_id,
            created_at: Utc::now().timestamp_millis(),
        };
        self.storage.insert_channel(channel).await;
    }
}

fn parse_cursor(cursor: &str) -> Result<(i64, u128), &'static str> {
    let mut segments = cursor.split(':');
    let created_at = segments
        .next()
        .ok_or("missing timestamp")?
        .parse::<i64>()
        .map_err(|_| "invalid timestamp")?;
    let id = segments
        .next()
        .ok_or("missing id")?
        .parse::<u128>()
        .map_err(|_| "invalid id")?;
    Ok((created_at, id))
}

impl From<&ChannelRecordStore> for ChannelResponse {
    fn from(channel: &ChannelRecordStore) -> Self {
        Self {
            id: channel.id,
            workspace_id: channel.workspace_id,
            name: channel.name.clone(),
            is_private: channel.is_private,
            created_by: channel.created_by,
            created_at: channel.created_at,
        }
    }
}

impl From<&MessageRecordStore> for MessageResponse {
    fn from(message: &MessageRecordStore) -> Self {
        Self {
            id: message.id,
            workspace_id: message.workspace_id,
            channel_id: message.channel_id,
            sender_id: message.sender_id,
            body_md: message.body_md.clone(),
            thread_root_id: message.thread_root_id,
            created_at: message.created_at,
            edited_at: message.edited_at,
            deleted_at: message.deleted_at,
        }
    }
}

fn ensure_channel_admin(context: &AuthContext) -> ApiResult<()> {
    match context.role {
        WorkspaceRole::Owner | WorkspaceRole::Admin => Ok(()),
        WorkspaceRole::Member => Err(ApiError::Unauthorized(
            "you do not have permission to manage channels".to_string(),
        )),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/channels",
    responses(
        (status = 200, description = "List channels", body = [ChannelResponse]),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn list_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<ChannelResponse>>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let items = state.channels.list_channels(context.workspace_id).await;
    Ok(Json(items))
}

#[utoipa::path(
    post,
    path = "/api/v1/channels",
    request_body = CreateChannelRequest,
    responses(
        (status = 201, description = "Channel created", body = ChannelResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 400, description = "Validation error", body = ErrorResponse)
    )
)]
pub(crate) async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateChannelRequest>,
) -> ApiResult<(StatusCode, Json<ChannelResponse>)> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_channel_admin(&context)?;
    let item = state
        .channels
        .create_channel(context.workspace_id, context.user_id, payload)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "CHANNEL_CREATED",
            "channel",
            Some(item.id.to_string()),
            json!({ "name": item.name, "is_private": item.is_private }),
        )
        .await;
    state
        .realtime
        .emit(
            context.workspace_id,
            realtime::make_event(
                "CHANNEL_CREATED",
                context.workspace_id,
                Some(item.id),
                None,
                serde_json::to_value(&item).unwrap_or_default(),
            ),
        )
        .await;
    Ok((StatusCode::CREATED, Json(item)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/channels/{id}",
    responses(
        (status = 204, description = "Channel deleted"),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_channel_admin(&context)?;
    state
        .channels
        .delete_channel(context.workspace_id, channel_id)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "CHANNEL_DELETED",
            "channel",
            Some(channel_id.to_string()),
            json!({}),
        )
        .await;
    state
        .realtime
        .emit(
            context.workspace_id,
            realtime::make_event(
                "CHANNEL_DELETED",
                context.workspace_id,
                Some(channel_id),
                None,
                json!({ "channel_id": channel_id }),
            ),
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/channels/{id}/members",
    responses(
        (status = 200, description = "List channel members", body = [ChannelMemberResponse]),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    )
)]
pub(crate) async fn list_channel_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
) -> ApiResult<Json<Vec<ChannelMemberResponse>>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_channel_admin(&context)?;
    let items = state
        .channels
        .list_channel_members(context.workspace_id, channel_id)
        .await?;
    Ok(Json(items))
}

#[utoipa::path(
    post,
    path = "/api/v1/channels/{id}/members",
    request_body = AddChannelMemberRequest,
    responses(
        (status = 204, description = "Channel member added"),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    )
)]
pub(crate) async fn add_channel_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
    Json(payload): Json<AddChannelMemberRequest>,
) -> ApiResult<StatusCode> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_channel_admin(&context)?;
    state
        .channels
        .add_channel_member(context.workspace_id, channel_id, payload.user_id)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "CHANNEL_MEMBER_ADDED",
            "channel",
            Some(channel_id.to_string()),
            json!({ "member_user_id": payload.user_id }),
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/v1/channels/{id}/members/{user_id}",
    responses(
        (status = 204, description = "Channel member removed"),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    )
)]
pub(crate) async fn remove_channel_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((channel_id, user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_channel_admin(&context)?;
    state
        .channels
        .remove_channel_member(context.workspace_id, channel_id, user_id)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "CHANNEL_MEMBER_REMOVED",
            "channel",
            Some(channel_id.to_string()),
            json!({ "member_user_id": user_id }),
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/channels/{id}/messages",
    request_body = CreateMessageRequest,
    responses(
        (status = 201, description = "Message created", body = MessageResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    )
)]
pub(crate) async fn create_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
    Json(payload): Json<CreateMessageRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let item = state
        .channels
        .create_message(&context, channel_id, payload)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "MESSAGE_CREATED",
            "message",
            Some(item.id.to_string()),
            json!({ "channel_id": item.channel_id, "thread_root_id": item.thread_root_id }),
        )
        .await;
    state
        .realtime
        .emit(
            context.workspace_id,
            realtime::make_event(
                "MESSAGE_CREATED",
                context.workspace_id,
                Some(item.channel_id),
                None,
                serde_json::to_value(&item).unwrap_or_default(),
            ),
        )
        .await;
    Ok((StatusCode::CREATED, Json(item)))
}

#[utoipa::path(
    get,
    path = "/api/v1/channels/{id}/messages",
    params(MessageQuery),
    responses(
        (status = 200, description = "Messages page", body = MessageListResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    )
)]
pub(crate) async fn list_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Json<MessageListResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let page = state
        .channels
        .list_messages(&context, channel_id, &query)
        .await?;
    Ok(Json(page))
}

#[utoipa::path(
    patch,
    path = "/api/v1/messages/{id}",
    request_body = UpdateMessageRequest,
    responses(
        (status = 200, description = "Message updated", body = MessageResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Message not found", body = ErrorResponse)
    )
)]
pub(crate) async fn update_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(message_id): Path<Uuid>,
    Json(payload): Json<UpdateMessageRequest>,
) -> ApiResult<Json<MessageResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let item = state
        .channels
        .update_message(&context, message_id, payload)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "MESSAGE_UPDATED",
            "message",
            Some(item.id.to_string()),
            json!({ "channel_id": item.channel_id }),
        )
        .await;
    state
        .realtime
        .emit(
            context.workspace_id,
            realtime::make_event(
                "MESSAGE_UPDATED",
                context.workspace_id,
                Some(item.channel_id),
                None,
                serde_json::to_value(&item).unwrap_or_default(),
            ),
        )
        .await;
    Ok(Json(item))
}

#[utoipa::path(
    delete,
    path = "/api/v1/messages/{id}",
    responses(
        (status = 204, description = "Message deleted"),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Message not found", body = ErrorResponse)
    )
)]
pub(crate) async fn delete_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(message_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    state.channels.delete_message(&context, message_id).await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "MESSAGE_DELETED",
            "message",
            Some(message_id.to_string()),
            json!({}),
        )
        .await;
    state
        .realtime
        .emit(
            context.workspace_id,
            realtime::make_event(
                "MESSAGE_DELETED",
                context.workspace_id,
                None,
                None,
                json!({ "message_id": message_id }),
            ),
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PersistenceBackend, Storage};

    #[tokio::test]
    async fn message_cursor_pagination_returns_next_cursor() {
        let workspace_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let service = ChannelService::new(
            Arc::new(
                Storage::new(PersistenceBackend::Memory, None)
                    .await
                    .expect("memory storage should init"),
            ),
            workspace_id,
            user_id,
        );
        let context = AuthContext {
            user_id,
            workspace_id,
            role: WorkspaceRole::Owner,
        };
        let channel_id = service
            .list_channels(workspace_id)
            .await
            .first()
            .expect("general channel should exist")
            .id;

        for idx in 0..3 {
            service
                .create_message(
                    &context,
                    channel_id,
                    CreateMessageRequest {
                        body_md: format!("message {idx}"),
                    },
                )
                .await
                .expect("message creation should succeed");
        }

        let first_page = service
            .list_messages(
                &context,
                channel_id,
                &MessageQuery {
                    cursor: None,
                    limit: Some(2),
                },
            )
            .await
            .expect("first page should work");
        assert_eq!(first_page.items.len(), 2);
        assert!(first_page.next_cursor.is_some());

        let second_page = service
            .list_messages(
                &context,
                channel_id,
                &MessageQuery {
                    cursor: first_page.next_cursor,
                    limit: Some(2),
                },
            )
            .await
            .expect("second page should work");
        assert_eq!(second_page.items.len(), 1);
        assert!(second_page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn thread_summary_counts_replies_and_participants() {
        let workspace_id = Uuid::new_v4();
        let owner_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let service = ChannelService::new(
            Arc::new(
                Storage::new(PersistenceBackend::Memory, None)
                    .await
                    .expect("memory storage should init"),
            ),
            workspace_id,
            owner_id,
        );

        let owner_ctx = AuthContext {
            user_id: owner_id,
            workspace_id,
            role: WorkspaceRole::Owner,
        };
        let member_ctx = AuthContext {
            user_id: member_id,
            workspace_id,
            role: WorkspaceRole::Member,
        };
        let channel_id = service
            .list_channels(workspace_id)
            .await
            .first()
            .expect("general channel should exist")
            .id;

        let root = service
            .create_message(
                &owner_ctx,
                channel_id,
                CreateMessageRequest {
                    body_md: "root".to_string(),
                },
            )
            .await
            .expect("root message should be created");

        service
            .create_thread_reply(
                &owner_ctx,
                root.id,
                CreateMessageRequest {
                    body_md: "reply 1".to_string(),
                },
            )
            .await
            .expect("owner reply should be created");
        service
            .create_thread_reply(
                &member_ctx,
                root.id,
                CreateMessageRequest {
                    body_md: "reply 2".to_string(),
                },
            )
            .await
            .expect("member reply should be created");

        let summary = service
            .thread_summary(&owner_ctx, root.id)
            .await
            .expect("thread summary should work");
        assert_eq!(summary.reply_count, 2);
        assert_eq!(summary.participants.len(), 2);
    }

    #[tokio::test]
    async fn private_channel_requires_membership_for_member_role() {
        let workspace_id = Uuid::new_v4();
        let owner_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let storage = Arc::new(
            Storage::new(PersistenceBackend::Memory, None)
                .await
                .expect("memory storage should init"),
        );
        let service = ChannelService::new(storage.clone(), workspace_id, owner_id);

        let owner_ctx = AuthContext {
            user_id: owner_id,
            workspace_id,
            role: WorkspaceRole::Owner,
        };
        let member_ctx = AuthContext {
            user_id: member_id,
            workspace_id,
            role: WorkspaceRole::Member,
        };

        let private_channel = service
            .create_channel(
                workspace_id,
                owner_id,
                CreateChannelRequest {
                    name: "private-team".to_string(),
                    is_private: true,
                },
            )
            .await
            .expect("private channel should be created");

        let denied = service
            .create_message(
                &member_ctx,
                private_channel.id,
                CreateMessageRequest {
                    body_md: "hi".to_string(),
                },
            )
            .await
            .expect_err("member should not access private channel");
        assert!(matches!(denied, ApiError::Unauthorized(_)));

        storage
            .add_channel_member(private_channel.id, member_id)
            .await;

        let created = service
            .create_message(
                &member_ctx,
                private_channel.id,
                CreateMessageRequest {
                    body_md: "hi".to_string(),
                },
            )
            .await
            .expect("member should access private channel after membership");
        assert_eq!(created.channel_id, private_channel.id);

        // owner/admin bypasses channel membership checks
        let owner_created = service
            .create_message(
                &owner_ctx,
                private_channel.id,
                CreateMessageRequest {
                    body_md: "owner".to_string(),
                },
            )
            .await
            .expect("owner should access private channel");
        assert_eq!(owner_created.channel_id, private_channel.id);
    }

    #[tokio::test]
    async fn channel_member_management_roundtrip() {
        let workspace_id = Uuid::new_v4();
        let owner_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let storage = Arc::new(
            Storage::new(PersistenceBackend::Memory, None)
                .await
                .expect("memory storage should init"),
        );
        storage
            .put_membership_role(workspace_id, member_id, "member")
            .await;
        let service = ChannelService::new(storage.clone(), workspace_id, owner_id);

        let private_channel = service
            .create_channel(
                workspace_id,
                owner_id,
                CreateChannelRequest {
                    name: "ops-private".to_string(),
                    is_private: true,
                },
            )
            .await
            .expect("private channel should be created");

        service
            .add_channel_member(workspace_id, private_channel.id, member_id)
            .await
            .expect("add member should work");

        let members = service
            .list_channel_members(workspace_id, private_channel.id)
            .await
            .expect("list members should work");
        assert!(members.iter().any(|item| item.user_id == member_id));

        service
            .remove_channel_member(workspace_id, private_channel.id, member_id)
            .await
            .expect("remove member should work");
        let members_after = service
            .list_channel_members(workspace_id, private_channel.id)
            .await
            .expect("list members should work after removal");
        assert!(!members_after.iter().any(|item| item.user_id == member_id));
    }
}
