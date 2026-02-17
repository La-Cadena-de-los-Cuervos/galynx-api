use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    app::AppState,
    channels::{CreateMessageRequest, MessageListResponse, MessageQuery, MessageResponse, ThreadSummaryResponse},
    errors::{ApiResult, ErrorResponse},
    realtime,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/threads/:root_id", get(get_thread))
        .route(
            "/api/v1/threads/:root_id/replies",
            get(list_replies).post(create_reply),
        )
}

#[utoipa::path(
    get,
    path = "/api/v1/threads/{root_id}",
    responses(
        (status = 200, description = "Thread summary", body = ThreadSummaryResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Thread not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(root_id): Path<Uuid>,
) -> ApiResult<Json<ThreadSummaryResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let summary = state
        .channels
        .thread_summary(context.workspace_id, root_id)
        .await?;
    Ok(Json(summary))
}

#[utoipa::path(
    get,
    path = "/api/v1/threads/{root_id}/replies",
    params(MessageQuery),
    responses(
        (status = 200, description = "Thread replies", body = MessageListResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Thread not found", body = ErrorResponse)
    )
)]
pub(crate) async fn list_replies(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(root_id): Path<Uuid>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Json<MessageListResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let page = state
        .channels
        .list_thread_replies(context.workspace_id, root_id, &query)
        .await?;
    Ok(Json(page))
}

#[utoipa::path(
    post,
    path = "/api/v1/threads/{root_id}/replies",
    request_body = CreateMessageRequest,
    responses(
        (status = 201, description = "Thread reply created", body = MessageResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Thread not found", body = ErrorResponse)
    )
)]
pub(crate) async fn create_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(root_id): Path<Uuid>,
    Json(payload): Json<CreateMessageRequest>,
) -> ApiResult<(StatusCode, Json<MessageResponse>)> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let reply = state
        .channels
        .create_thread_reply(&context, root_id, payload)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "THREAD_REPLY_CREATED",
            "message",
            Some(reply.id.to_string()),
            json!({ "root_id": root_id, "channel_id": reply.channel_id }),
        )
        .await;
    let summary = state
        .channels
        .thread_summary(context.workspace_id, root_id)
        .await?;
    state
        .realtime
        .emit(
            context.workspace_id,
            realtime::make_event(
                "THREAD_UPDATED",
                context.workspace_id,
                Some(reply.channel_id),
                None,
                serde_json::to_value(summary).unwrap_or_default(),
            ),
        )
        .await;
    Ok((StatusCode::CREATED, Json(reply)))
}
