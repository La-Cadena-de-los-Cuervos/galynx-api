use std::sync::Arc;

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::{get, post},
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    app::AppState,
    errors::{ApiError, ApiResult, ErrorResponse},
    rate_limit::client_ip_from_headers,
    storage::{AuthUserRecordStore, RefreshSessionRecordStore, Storage},
};

#[derive(Clone)]
pub struct AuthService {
    storage: Arc<Storage>,
    bootstrap_workspace_id: Uuid,
    bootstrap_user_id: Uuid,
    bootstrap_email: String,
    bootstrap_name: String,
    bootstrap_password_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
}

impl WorkspaceRole {
    fn from_storage_role(value: &str) -> Result<Self, &'static str> {
        match value.trim().to_ascii_lowercase().as_str() {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            _ => Err("invalid role"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccessClaims {
    sub: String,
    email: String,
    workspace_id: String,
    role: WorkspaceRole,
    token_type: String,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthTokensResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: i64,
    pub refresh_expires_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MeResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub workspace_id: Uuid,
    pub role: WorkspaceRole,
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub workspace_id: Uuid,
    pub role: WorkspaceRole,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/me", get(me))
}

impl AuthService {
    pub fn new(storage: Arc<Storage>, bootstrap_email: &str, bootstrap_password: &str) -> Self {
        let normalized_email = bootstrap_email.to_ascii_lowercase();
        let bootstrap_name = "Owner".to_string();
        let bootstrap_password_hash =
            hash_password(bootstrap_password).expect("failed to create bootstrap password hash");
        let bootstrap_workspace_id = Uuid::new_v4();
        let bootstrap_user_id = Uuid::new_v4();

        Self {
            storage,
            bootstrap_workspace_id,
            bootstrap_user_id,
            bootstrap_email: normalized_email,
            bootstrap_name,
            bootstrap_password_hash,
        }
    }

    pub fn bootstrap_workspace_id(&self) -> Uuid {
        self.bootstrap_workspace_id
    }

    pub fn bootstrap_user_id(&self) -> Uuid {
        self.bootstrap_user_id
    }

    async fn primary_membership(&self, user_id: Uuid) -> Option<(Uuid, WorkspaceRole)> {
        self.ensure_bootstrap_seed().await;
        let (workspace_id, role) = self.storage.find_primary_membership(user_id).await?;
        Some((workspace_id, WorkspaceRole::from_storage_role(&role).ok()?))
    }

    pub async fn login(
        &self,
        email: &str,
        password: &str,
        jwt_secret: &str,
        access_ttl_minutes: i64,
        refresh_ttl_days: i64,
    ) -> ApiResult<AuthTokensResponse> {
        self.ensure_bootstrap_seed().await;
        let email = email.trim().to_ascii_lowercase();
        let user = self
            .storage
            .get_auth_user_by_email(&email)
            .await
            .ok_or_else(|| ApiError::Unauthorized("invalid credentials".to_string()))?;

        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|_| ApiError::Internal("invalid stored password hash".to_string()))?;

        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| ApiError::Unauthorized("invalid credentials".to_string()))?;

        self.issue_tokens(user, jwt_secret, access_ttl_minutes, refresh_ttl_days)
            .await
    }

    async fn issue_tokens(
        &self,
        user: AuthUserRecordStore,
        jwt_secret: &str,
        access_ttl_minutes: i64,
        refresh_ttl_days: i64,
    ) -> ApiResult<AuthTokensResponse> {
        let now = Utc::now();
        let access_exp = now + Duration::minutes(access_ttl_minutes);
        let refresh_exp = now + Duration::days(refresh_ttl_days);
        let (workspace_id, role) = self.primary_membership(user.id).await.ok_or_else(|| {
            ApiError::Unauthorized("user has no workspace membership".to_string())
        })?;

        let claims = AccessClaims {
            sub: user.id.to_string(),
            email: user.email,
            workspace_id: workspace_id.to_string(),
            role,
            token_type: "access".to_string(),
            iat: now.timestamp(),
            exp: access_exp.timestamp(),
        };

        let access_token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(jwt_secret.as_bytes()),
        )
        .map_err(|error| ApiError::Internal(format!("failed to create access token: {error}")))?;

        let refresh_token = generate_refresh_token();
        let refresh_hash = token_hash(&refresh_token);
        let session = RefreshSessionRecordStore {
            user_id: user.id,
            expires_at: refresh_exp.timestamp(),
            revoked_at: None,
            replaced_by_hash: None,
        };

        self.storage
            .put_refresh_session(refresh_hash, session)
            .await;

        Ok(AuthTokensResponse {
            access_token,
            refresh_token,
            access_expires_at: access_exp.timestamp(),
            refresh_expires_at: refresh_exp.timestamp(),
        })
    }

    pub async fn refresh(
        &self,
        refresh_token: &str,
        jwt_secret: &str,
        access_ttl_minutes: i64,
        refresh_ttl_days: i64,
    ) -> ApiResult<AuthTokensResponse> {
        self.ensure_bootstrap_seed().await;
        let now = Utc::now().timestamp();
        let incoming_hash = token_hash(refresh_token);

        let snapshot = self
            .storage
            .get_refresh_session(&incoming_hash)
            .await
            .ok_or_else(|| ApiError::Unauthorized("invalid refresh token".to_string()))?;

        if snapshot.expires_at <= now {
            return Err(ApiError::Unauthorized("refresh token expired".to_string()));
        }

        if snapshot.revoked_at.is_some() {
            if let Some(replaced_hash) = snapshot.replaced_by_hash.clone() {
                let _ = self
                    .storage
                    .update_refresh_session(&replaced_hash, |session| {
                        session.revoked_at = Some(now);
                    })
                    .await;
            }
            return Err(ApiError::Unauthorized(
                "refresh token reuse detected".to_string(),
            ));
        }

        self.storage
            .update_refresh_session(&incoming_hash, |session| {
                session.revoked_at = Some(now);
            })
            .await
            .ok_or_else(|| ApiError::Unauthorized("invalid refresh token".to_string()))?;
        let refresh_token = generate_refresh_token();
        let refresh_hash = token_hash(&refresh_token);
        self.storage
            .update_refresh_session(&incoming_hash, |session| {
                session.replaced_by_hash = Some(refresh_hash.clone());
            })
            .await;

        let refresh_exp = Utc::now() + Duration::days(refresh_ttl_days);
        let rotated = RefreshSessionRecordStore {
            user_id: snapshot.user_id,
            expires_at: refresh_exp.timestamp(),
            revoked_at: None,
            replaced_by_hash: None,
        };
        self.storage
            .put_refresh_session(refresh_hash, rotated)
            .await;

        let user = self
            .storage
            .get_auth_user_by_id(snapshot.user_id)
            .await
            .ok_or_else(|| ApiError::Unauthorized("user not found".to_string()))?;

        let access_exp = Utc::now() + Duration::minutes(access_ttl_minutes);
        let (workspace_id, role) = self.primary_membership(user.id).await.ok_or_else(|| {
            ApiError::Unauthorized("user has no workspace membership".to_string())
        })?;
        let claims = AccessClaims {
            sub: user.id.to_string(),
            email: user.email,
            workspace_id: workspace_id.to_string(),
            role,
            token_type: "access".to_string(),
            iat: Utc::now().timestamp(),
            exp: access_exp.timestamp(),
        };

        let access_token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(jwt_secret.as_bytes()),
        )
        .map_err(|error| ApiError::Internal(format!("failed to create access token: {error}")))?;

        Ok(AuthTokensResponse {
            access_token,
            refresh_token,
            access_expires_at: access_exp.timestamp(),
            refresh_expires_at: refresh_exp.timestamp(),
        })
    }

    pub async fn logout(&self, refresh_token: &str) -> ApiResult<()> {
        self.ensure_bootstrap_seed().await;
        let hash = token_hash(refresh_token);
        let now = Utc::now().timestamp();
        self.storage
            .update_refresh_session(&hash, |session| {
                session.revoked_at = Some(now);
            })
            .await
            .ok_or_else(|| ApiError::Unauthorized("invalid refresh token".to_string()))?;
        Ok(())
    }

    pub async fn me_from_context(&self, context: &AuthContext) -> ApiResult<MeResponse> {
        self.ensure_bootstrap_seed().await;
        let user = self
            .storage
            .get_auth_user_by_id(context.user_id)
            .await
            .ok_or_else(|| ApiError::Unauthorized("user not found".to_string()))?;

        Ok(MeResponse {
            id: user.id,
            email: user.email.clone(),
            name: user.name.clone(),
            workspace_id: context.workspace_id,
            role: context.role.clone(),
        })
    }

    pub async fn authenticate_headers(
        &self,
        headers: &HeaderMap,
        jwt_secret: &str,
    ) -> ApiResult<AuthContext> {
        let access_token = bearer_from_headers(headers)?;
        self.authenticate_access_token(&access_token, jwt_secret)
            .await
    }

    pub async fn context_from_access_token(
        &self,
        access_token: &str,
        jwt_secret: &str,
    ) -> ApiResult<AuthContext> {
        self.authenticate_access_token(access_token, jwt_secret)
            .await
    }

    async fn authenticate_access_token(
        &self,
        access_token: &str,
        jwt_secret: &str,
    ) -> ApiResult<AuthContext> {
        self.ensure_bootstrap_seed().await;
        let token_data = decode::<AccessClaims>(
            access_token,
            &DecodingKey::from_secret(jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| ApiError::Unauthorized("invalid access token".to_string()))?;

        if token_data.claims.token_type != "access" {
            return Err(ApiError::Unauthorized("invalid token type".to_string()));
        }

        let user_id = Uuid::parse_str(&token_data.claims.sub)
            .map_err(|_| ApiError::Unauthorized("invalid access token subject".to_string()))?;
        let workspace_id = Uuid::parse_str(&token_data.claims.workspace_id)
            .map_err(|_| ApiError::Unauthorized("invalid workspace id in token".to_string()))?;

        let role = self
            .storage
            .get_membership_role(workspace_id, user_id)
            .await
            .ok_or_else(|| ApiError::Unauthorized("membership no longer valid".to_string()))?;
        let role = WorkspaceRole::from_storage_role(&role)
            .map_err(|_| ApiError::Unauthorized("invalid membership role".to_string()))?;

        Ok(AuthContext {
            user_id,
            workspace_id,
            role,
        })
    }

    async fn ensure_bootstrap_seed(&self) {
        if let Some(existing) = self
            .storage
            .get_auth_user_by_email(&self.bootstrap_email)
            .await
        {
            if self
                .storage
                .find_primary_membership(existing.id)
                .await
                .is_none()
            {
                self.storage
                    .put_membership_role(self.bootstrap_workspace_id, existing.id, "owner")
                    .await;
            }
            return;
        }

        let user = AuthUserRecordStore {
            id: self.bootstrap_user_id,
            email: self.bootstrap_email.clone(),
            name: self.bootstrap_name.clone(),
            password_hash: self.bootstrap_password_hash.clone(),
        };
        self.storage.put_auth_user(user).await;
        self.storage
            .put_membership_role(self.bootstrap_workspace_id, self.bootstrap_user_id, "owner")
            .await;
    }
}

fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| ApiError::Internal(format!("failed to hash password: {error}")))
        .map(|hash| hash.to_string())
}

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    BASE64_STANDARD.encode(bytes)
}

fn bearer_from_headers(headers: &HeaderMap) -> ApiResult<String> {
    let value = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| ApiError::Unauthorized("missing authorization header".to_string()))?;
    let value = value
        .to_str()
        .map_err(|_| ApiError::Unauthorized("invalid authorization header".to_string()))?;

    let Some(token) = value.strip_prefix("Bearer ") else {
        return Err(ApiError::Unauthorized("expected bearer token".to_string()));
    };

    Ok(token.trim().to_string())
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthTokensResponse),
        (status = 401, description = "Invalid credentials", body = ErrorResponse)
    )
)]
pub(crate) async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> ApiResult<Json<AuthTokensResponse>> {
    if payload.email.trim().is_empty() || payload.password.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "email and password are required".to_string(),
        ));
    }
    let client_ip = client_ip_from_headers(&headers);
    state
        .rate_limit
        .check_auth(&client_ip, Some(&payload.email))
        .await?;

    let response = state
        .auth
        .login(
            &payload.email,
            &payload.password,
            &state.config.jwt_secret,
            state.config.access_ttl_minutes,
            state.config.refresh_ttl_days,
        )
        .await?;
    let context = state
        .auth
        .context_from_access_token(&response.access_token, &state.config.jwt_secret)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "AUTH_LOGIN",
            "user",
            Some(context.user_id.to_string()),
            json!({ "email": payload.email.trim().to_ascii_lowercase() }),
        )
        .await;

    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Refresh successful", body = AuthTokensResponse),
        (status = 401, description = "Invalid refresh token", body = ErrorResponse)
    )
)]
pub(crate) async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RefreshRequest>,
) -> ApiResult<Json<AuthTokensResponse>> {
    let client_ip = client_ip_from_headers(&headers);
    state.rate_limit.check_auth(&client_ip, None).await?;

    let response = state
        .auth
        .refresh(
            &payload.refresh_token,
            &state.config.jwt_secret,
            state.config.access_ttl_minutes,
            state.config.refresh_ttl_days,
        )
        .await?;
    let context = state
        .auth
        .context_from_access_token(&response.access_token, &state.config.jwt_secret)
        .await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "AUTH_REFRESH",
            "session",
            None,
            json!({ "reason": "token_rotation" }),
        )
        .await;

    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    request_body = LogoutRequest,
    responses(
        (status = 204, description = "Logout successful"),
        (status = 401, description = "Invalid refresh token", body = ErrorResponse)
    )
)]
pub(crate) async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LogoutRequest>,
) -> ApiResult<StatusCode> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let client_ip = client_ip_from_headers(&headers);
    state.rate_limit.check_auth(&client_ip, None).await?;

    state.auth.logout(&payload.refresh_token).await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "AUTH_LOGOUT",
            "session",
            None,
            json!({}),
        )
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/me",
    responses(
        (status = 200, description = "Current user", body = MeResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<MeResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let me = state.auth.me_from_context(&context).await?;
    Ok(Json(me))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PersistenceBackend, Storage};

    #[tokio::test]
    async fn refresh_rotation_invalidates_previous_token() {
        let service = AuthService::new(
            Arc::new(
                Storage::new(PersistenceBackend::Memory, None)
                    .await
                    .expect("memory storage should init"),
            ),
            "owner@galynx.local",
            "ChangeMe123!",
        );
        let first = service
            .login("owner@galynx.local", "ChangeMe123!", "secret", 15, 30)
            .await
            .expect("login should succeed");

        let second = service
            .refresh(&first.refresh_token, "secret", 15, 30)
            .await
            .expect("refresh should succeed");

        let reused = service
            .refresh(&first.refresh_token, "secret", 15, 30)
            .await
            .expect_err("reusing token should fail");

        assert!(matches!(reused, ApiError::Unauthorized(_)));
        assert!(!second.refresh_token.is_empty());
    }
}
