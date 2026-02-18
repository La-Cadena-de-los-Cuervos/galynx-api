use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use tower_http::trace::TraceLayer;
use utoipa::{OpenApi, ToSchema};

use crate::{
    attachments, audit, auth, channels, config::Config, rate_limit, reactions, realtime, storage,
    threads,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub storage: Arc<storage::Storage>,
    pub auth: Arc<auth::AuthService>,
    pub channels: Arc<channels::ChannelService>,
    pub attachments: Arc<attachments::AttachmentService>,
    pub audit: Arc<audit::AuditService>,
    pub rate_limit: Arc<rate_limit::RateLimitService>,
    pub reactions: Arc<reactions::ReactionService>,
    pub realtime: Arc<realtime::RealtimeHub>,
}

pub async fn build_state(config: Config) -> AppState {
    let storage = Arc::new(
        storage::Storage::new(config.persistence_backend, config.mongo_uri.as_deref())
            .await
            .expect("failed to initialize storage"),
    );
    let auth_service = auth::AuthService::new(
        storage.clone(),
        &config.bootstrap_email,
        &config.bootstrap_password,
    );
    let channels_service = channels::ChannelService::new(
        storage.clone(),
        auth_service.bootstrap_workspace_id(),
        auth_service.bootstrap_user_id(),
    );
    let audit_service = audit::AuditService::new(storage.clone());
    let attachments_service = attachments::AttachmentService::new(storage.clone(), &config).await;
    let rate_limit_service = rate_limit::RateLimitService::new();
    let reactions_service = reactions::ReactionService::new(storage.clone());
    let realtime_hub = realtime::RealtimeHub::new(config.redis_url.as_deref());
    AppState {
        config: Arc::new(config),
        storage,
        auth: Arc::new(auth_service),
        channels: Arc::new(channels_service),
        attachments: Arc::new(attachments_service),
        audit: Arc::new(audit_service),
        rate_limit: Arc::new(rate_limit_service),
        reactions: Arc::new(reactions_service),
        realtime: Arc::new(realtime_hub),
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/ready", get(ready))
        .route("/api/v1/openapi.json", get(openapi_spec))
        .merge(auth::router())
        .merge(channels::router())
        .merge(attachments::router())
        .merge(threads::router())
        .merge(audit::router())
        .merge(realtime::router())
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

#[derive(Debug, Serialize, ToSchema)]
struct HealthResponse {
    status: &'static str,
}

#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "Service health", body = HealthResponse)
    )
)]
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[utoipa::path(
    get,
    path = "/api/v1/ready",
    responses(
        (status = 200, description = "Service readiness", body = HealthResponse)
    )
)]
async fn ready(State(_state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse { status: "ready" })
}

async fn openapi_spec() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        ready,
        crate::auth::login,
        crate::auth::refresh,
        crate::auth::logout,
        crate::auth::me,
        crate::channels::list_channels,
        crate::channels::create_channel,
        crate::channels::delete_channel,
        crate::channels::list_channel_members,
        crate::channels::add_channel_member,
        crate::channels::remove_channel_member,
        crate::channels::list_messages,
        crate::channels::create_message,
        crate::channels::update_message,
        crate::channels::delete_message,
        crate::threads::get_thread,
        crate::threads::list_replies,
        crate::threads::create_reply,
        crate::attachments::presign,
        crate::attachments::commit,
        crate::attachments::get_attachment,
        crate::audit::list_audit,
        crate::realtime::ws_upgrade
    ),
    components(
        schemas(
            HealthResponse,
            crate::auth::LoginRequest,
            crate::auth::RefreshRequest,
            crate::auth::LogoutRequest,
            crate::auth::AuthTokensResponse,
            crate::auth::MeResponse,
            crate::auth::WorkspaceRole,
            crate::channels::CreateChannelRequest,
            crate::channels::ChannelResponse,
            crate::channels::ChannelMemberResponse,
            crate::channels::AddChannelMemberRequest,
            crate::channels::CreateMessageRequest,
            crate::channels::UpdateMessageRequest,
            crate::channels::MessageResponse,
            crate::channels::MessageListResponse,
            crate::channels::ThreadSummaryResponse,
            crate::attachments::PresignRequest,
            crate::attachments::PresignResponse,
            crate::attachments::CommitRequest,
            crate::attachments::AttachmentResponse,
            crate::attachments::AttachmentGetResponse,
            crate::audit::AuditLogResponse,
            crate::audit::AuditListResponse,
            crate::reactions::ReactionUpdateResponse,
            crate::realtime::WsEventEnvelope,
            crate::errors::ErrorResponse
        )
    ),
    tags(
        (name = "system", description = "System and health endpoints"),
        (name = "auth", description = "Authentication and identity"),
        (name = "channels", description = "Channels and messages"),
        (name = "attachments", description = "File attachments"),
        (name = "audit", description = "Audit log")
    )
)]
struct ApiDoc;
