use std::sync::Arc;

use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthContext, WorkspaceRole},
    errors::{ApiError, ApiResult, ErrorResponse},
    storage::{AuthUserRecordStore, Storage},
};

#[derive(Clone)]
pub struct UserService {
    storage: Arc<Storage>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub email: String,
    pub name: String,
    pub password: String,
    pub role: WorkspaceRole,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub workspace_id: Uuid,
    pub role: WorkspaceRole,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/users", get(list_users).post(create_user))
}

impl UserService {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub async fn list_users(&self, workspace_id: Uuid) -> ApiResult<Vec<UserResponse>> {
        let memberships = self.storage.list_workspace_memberships(workspace_id).await;
        let mut users = Vec::new();

        for (user_id, role) in memberships {
            let Some(user) = self.storage.get_auth_user_by_id(user_id).await else {
                continue;
            };
            let role = parse_role(&role)?;
            users.push(UserResponse {
                id: user.id,
                email: user.email,
                name: user.name,
                workspace_id,
                role,
            });
        }

        users.sort_by(|a, b| a.email.cmp(&b.email).then_with(|| a.id.cmp(&b.id)));
        Ok(users)
    }

    pub async fn create_user(
        &self,
        workspace_id: Uuid,
        payload: CreateUserRequest,
    ) -> ApiResult<UserResponse> {
        let email = payload.email.trim().to_ascii_lowercase();
        let name = payload.name.trim().to_string();
        let password = payload.password.trim().to_string();

        if email.is_empty() || name.is_empty() || password.is_empty() {
            return Err(ApiError::BadRequest(
                "email, name and password are required".to_string(),
            ));
        }
        if password.len() < 8 {
            return Err(ApiError::BadRequest(
                "password must have at least 8 characters".to_string(),
            ));
        }
        if matches!(payload.role, WorkspaceRole::Owner) {
            return Err(ApiError::BadRequest(
                "cannot create owner users via api".to_string(),
            ));
        }

        if self.storage.get_auth_user_by_email(&email).await.is_some() {
            return Err(ApiError::BadRequest("email already exists".to_string()));
        }

        let user_id = Uuid::new_v4();
        let user = AuthUserRecordStore {
            id: user_id,
            email: email.clone(),
            name: name.clone(),
            password_hash: hash_password(&password)?,
        };
        self.storage.put_auth_user(user).await;
        self.storage
            .put_membership_role(workspace_id, user_id, role_to_storage(&payload.role))
            .await;

        Ok(UserResponse {
            id: user_id,
            email,
            name,
            workspace_id,
            role: payload.role,
        })
    }
}

fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| ApiError::Internal(format!("failed to hash password: {error}")))
        .map(|hash| hash.to_string())
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

fn ensure_user_admin(context: &AuthContext) -> ApiResult<()> {
    match context.role {
        WorkspaceRole::Owner | WorkspaceRole::Admin => Ok(()),
        WorkspaceRole::Member => Err(ApiError::Unauthorized(
            "you do not have permission to manage users".to_string(),
        )),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/users",
    responses(
        (status = 200, description = "List workspace users", body = [UserResponse]),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<UserResponse>>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_user_admin(&context)?;
    let users = state.users.list_users(context.workspace_id).await?;
    Ok(Json(users))
}

#[utoipa::path(
    post,
    path = "/api/v1/users",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = UserResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 400, description = "Validation error", body = ErrorResponse)
    )
)]
pub(crate) async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserRequest>,
) -> ApiResult<(StatusCode, Json<UserResponse>)> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_user_admin(&context)?;
    let user = state
        .users
        .create_user(context.workspace_id, payload)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "USER_CREATED",
            "user",
            Some(user.id.to_string()),
            json!({ "email": user.email, "role": user.role }),
        )
        .await;
    Ok((StatusCode::CREATED, Json(user)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::PersistenceBackend;

    #[tokio::test]
    async fn create_and_list_workspace_users() {
        let storage = Arc::new(
            Storage::new(PersistenceBackend::Memory, None)
                .await
                .expect("memory storage should init"),
        );
        let service = UserService::new(storage);
        let workspace_id = Uuid::new_v4();

        let created = service
            .create_user(
                workspace_id,
                CreateUserRequest {
                    email: "member@galynx.local".to_string(),
                    name: "Member User".to_string(),
                    password: "ChangeMe123!".to_string(),
                    role: WorkspaceRole::Member,
                },
            )
            .await
            .expect("create user should succeed");

        let listed = service
            .list_users(workspace_id)
            .await
            .expect("list users should succeed");
        assert!(listed.iter().any(|item| item.id == created.id));
    }
}
