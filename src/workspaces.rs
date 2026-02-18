use std::sync::Arc;

use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthContext, WorkspaceRole},
    errors::{ApiError, ApiResult, ErrorResponse},
    storage::{AuthUserRecordStore, Storage, WorkspaceRecordStore},
};

#[derive(Clone)]
pub struct WorkspaceService {
    storage: Arc<Storage>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceResponse {
    pub id: Uuid,
    pub name: String,
    pub role: WorkspaceRole,
    pub created_by: Uuid,
    pub created_at: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspaceRequest {
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceMemberResponse {
    pub user_id: Uuid,
    pub email: String,
    pub name: String,
    pub role: WorkspaceRole,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OnboardWorkspaceMemberRequest {
    pub email: String,
    pub name: Option<String>,
    pub password: Option<String>,
    pub role: WorkspaceRole,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route(
            "/api/v1/workspaces/:id/members",
            get(list_workspace_members).post(onboard_workspace_member),
        )
}

impl WorkspaceService {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub async fn list_workspaces_for_user(
        &self,
        user_id: Uuid,
    ) -> ApiResult<Vec<WorkspaceResponse>> {
        let mut memberships = self.storage.list_user_memberships(user_id).await;
        memberships.sort_by(|a, b| a.0.cmp(&b.0));
        let mut items = Vec::new();

        for (workspace_id, role) in memberships {
            let Some(workspace) = self.storage.get_workspace(workspace_id).await else {
                continue;
            };
            items.push(WorkspaceResponse {
                id: workspace.id,
                name: workspace.name,
                role: parse_role(&role)?,
                created_by: workspace.created_by,
                created_at: workspace.created_at,
            });
        }

        Ok(items)
    }

    pub async fn create_workspace(
        &self,
        owner_id: Uuid,
        payload: CreateWorkspaceRequest,
    ) -> ApiResult<WorkspaceResponse> {
        let name = payload.name.trim().to_string();
        if name.is_empty() {
            return Err(ApiError::BadRequest(
                "workspace name is required".to_string(),
            ));
        }

        let workspace = WorkspaceRecordStore {
            id: Uuid::new_v4(),
            name: name.clone(),
            created_by: owner_id,
            created_at: Utc::now().timestamp_millis(),
        };

        self.storage.put_workspace(workspace.clone()).await;
        self.storage
            .put_membership_role(workspace.id, owner_id, "owner")
            .await;

        Ok(WorkspaceResponse {
            id: workspace.id,
            name,
            role: WorkspaceRole::Owner,
            created_by: owner_id,
            created_at: workspace.created_at,
        })
    }

    pub async fn list_members(
        &self,
        workspace_id: Uuid,
    ) -> ApiResult<Vec<WorkspaceMemberResponse>> {
        let memberships = self.storage.list_workspace_memberships(workspace_id).await;
        let mut users = Vec::new();

        for (user_id, role) in memberships {
            let Some(user) = self.storage.get_auth_user_by_id(user_id).await else {
                continue;
            };
            users.push(WorkspaceMemberResponse {
                user_id,
                email: user.email,
                name: user.name,
                role: parse_role(&role)?,
            });
        }

        users.sort_by(|a, b| {
            a.email
                .cmp(&b.email)
                .then_with(|| a.user_id.cmp(&b.user_id))
        });
        Ok(users)
    }

    pub async fn onboard_member(
        &self,
        workspace_id: Uuid,
        payload: OnboardWorkspaceMemberRequest,
    ) -> ApiResult<WorkspaceMemberResponse> {
        if matches!(payload.role, WorkspaceRole::Owner) {
            return Err(ApiError::BadRequest(
                "cannot onboard owner users via api".to_string(),
            ));
        }

        let email = payload.email.trim().to_ascii_lowercase();
        if email.is_empty() {
            return Err(ApiError::BadRequest("email is required".to_string()));
        }

        let user = if let Some(existing) = self.storage.get_auth_user_by_email(&email).await {
            existing
        } else {
            let name = payload.name.unwrap_or_default().trim().to_string();
            let password = payload.password.unwrap_or_default().trim().to_string();
            if name.is_empty() || password.is_empty() {
                return Err(ApiError::BadRequest(
                    "name and password are required for new users".to_string(),
                ));
            }
            if password.len() < 8 {
                return Err(ApiError::BadRequest(
                    "password must have at least 8 characters".to_string(),
                ));
            }

            let user = AuthUserRecordStore {
                id: Uuid::new_v4(),
                email: email.clone(),
                name,
                password_hash: hash_password(&password)?,
            };
            self.storage.put_auth_user(user.clone()).await;
            user
        };

        self.storage
            .put_membership_role(workspace_id, user.id, role_to_storage(&payload.role))
            .await;

        Ok(WorkspaceMemberResponse {
            user_id: user.id,
            email: user.email,
            name: user.name,
            role: payload.role,
        })
    }
}

fn role_to_storage(role: &WorkspaceRole) -> &'static str {
    match role {
        WorkspaceRole::Owner => "owner",
        WorkspaceRole::Admin => "admin",
        WorkspaceRole::Member => "member",
    }
}

fn parse_role(value: &str) -> ApiResult<WorkspaceRole> {
    match value.trim().to_ascii_lowercase().as_str() {
        "owner" => Ok(WorkspaceRole::Owner),
        "admin" => Ok(WorkspaceRole::Admin),
        "member" => Ok(WorkspaceRole::Member),
        _ => Err(ApiError::Internal("invalid membership role".to_string())),
    }
}

fn ensure_workspace_admin(context: &AuthContext) -> ApiResult<()> {
    match context.role {
        WorkspaceRole::Owner | WorkspaceRole::Admin => Ok(()),
        WorkspaceRole::Member => Err(ApiError::Unauthorized(
            "you do not have permission to manage workspace members".to_string(),
        )),
    }
}

fn ensure_context_workspace(context: &AuthContext, workspace_id: Uuid) -> ApiResult<()> {
    if context.workspace_id != workspace_id {
        return Err(ApiError::Unauthorized(
            "token workspace does not match requested workspace".to_string(),
        ));
    }
    Ok(())
}

fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| ApiError::Internal(format!("failed to hash password: {error}")))
        .map(|hash| hash.to_string())
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces",
    responses(
        (status = 200, description = "List workspaces for current user", body = [WorkspaceResponse]),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn list_workspaces(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<WorkspaceResponse>>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let workspaces = state
        .workspaces
        .list_workspaces_for_user(context.user_id)
        .await?;
    Ok(Json(workspaces))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces",
    request_body = CreateWorkspaceRequest,
    responses(
        (status = 201, description = "Workspace created", body = WorkspaceResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 400, description = "Validation error", body = ErrorResponse)
    )
)]
pub(crate) async fn create_workspace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateWorkspaceRequest>,
) -> ApiResult<(StatusCode, Json<WorkspaceResponse>)> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let workspace = state
        .workspaces
        .create_workspace(context.user_id, payload)
        .await?;

    state
        .audit
        .write(
            workspace.id,
            Some(context.user_id),
            "WORKSPACE_CREATED",
            "workspace",
            Some(workspace.id.to_string()),
            json!({ "name": workspace.name }),
        )
        .await;

    Ok((StatusCode::CREATED, Json(workspace)))
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{id}/members",
    responses(
        (status = 200, description = "List workspace members", body = [WorkspaceMemberResponse]),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Workspace not found", body = ErrorResponse)
    )
)]
pub(crate) async fn list_workspace_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<Vec<WorkspaceMemberResponse>>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_context_workspace(&context, workspace_id)?;
    ensure_workspace_admin(&context)?;

    if state.storage.get_workspace(workspace_id).await.is_none() {
        return Err(ApiError::NotFound("workspace not found".to_string()));
    }

    let members = state.workspaces.list_members(workspace_id).await?;
    Ok(Json(members))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{id}/members",
    request_body = OnboardWorkspaceMemberRequest,
    responses(
        (status = 201, description = "Workspace member onboarded", body = WorkspaceMemberResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 400, description = "Validation error", body = ErrorResponse),
        (status = 404, description = "Workspace not found", body = ErrorResponse)
    )
)]
pub(crate) async fn onboard_workspace_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
    Json(payload): Json<OnboardWorkspaceMemberRequest>,
) -> ApiResult<(StatusCode, Json<WorkspaceMemberResponse>)> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_context_workspace(&context, workspace_id)?;
    ensure_workspace_admin(&context)?;

    if state.storage.get_workspace(workspace_id).await.is_none() {
        return Err(ApiError::NotFound("workspace not found".to_string()));
    }

    let user = state
        .workspaces
        .onboard_member(workspace_id, payload)
        .await?;
    state
        .audit
        .write(
            workspace_id,
            Some(context.user_id),
            "WORKSPACE_MEMBER_ONBOARDED",
            "user",
            Some(user.user_id.to_string()),
            json!({ "email": user.email, "role": user.role }),
        )
        .await;

    Ok((StatusCode::CREATED, Json(user)))
}
