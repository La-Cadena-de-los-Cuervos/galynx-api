use std::{sync::Arc, time::Duration};

use aws_config::{BehaviorVersion, Region, meta::region::RegionProviderChain};
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    Client as S3Client, config::Builder as S3ConfigBuilder, presigning::PresigningConfig,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::AuthContext,
    config::Config,
    errors::{ApiError, ApiResult, ErrorResponse},
    storage::{AttachmentRecordStore, PendingUploadRecord, Storage},
};

const MAX_ATTACHMENT_SIZE_BYTES: u64 = 100 * 1024 * 1024;
const PRESIGN_TTL_SECONDS: i64 = 900;
const DOWNLOAD_TTL_SECONDS: i64 = 600;

#[derive(Clone)]
pub struct AttachmentService {
    storage: Arc<Storage>,
    object_storage: Option<Arc<S3ObjectStorage>>,
}

#[derive(Clone)]
struct S3ObjectStorage {
    presign_client: S3Client,
    bucket: String,
    region: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PresignRequest {
    pub channel_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PresignResponse {
    pub upload_id: Uuid,
    pub upload_url: String,
    pub bucket: String,
    pub key: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CommitRequest {
    pub upload_id: Uuid,
    pub message_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AttachmentResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub channel_id: Uuid,
    pub message_id: Option<Uuid>,
    pub uploader_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub storage_bucket: String,
    pub storage_key: String,
    pub storage_region: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AttachmentGetResponse {
    pub attachment: AttachmentResponse,
    pub download_url: String,
    pub expires_at: i64,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/attachments/presign", post(presign))
        .route("/api/v1/attachments/commit", post(commit))
        .route("/api/v1/attachments/:id", get(get_attachment))
}

impl AttachmentService {
    pub async fn new(storage: Arc<Storage>, config: &Config) -> Self {
        let object_storage = S3ObjectStorage::from_config(config).await.map(Arc::new);
        Self {
            storage,
            object_storage,
        }
    }

    #[cfg(test)]
    pub fn new_without_object_storage(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            object_storage: None,
        }
    }

    pub async fn presign(
        &self,
        context: &AuthContext,
        payload: PresignRequest,
    ) -> ApiResult<PresignResponse> {
        let filename = payload.filename.trim().to_string();
        let content_type = payload.content_type.trim().to_string();
        if filename.is_empty() {
            return Err(ApiError::BadRequest("filename is required".to_string()));
        }
        if content_type.is_empty() {
            return Err(ApiError::BadRequest("content_type is required".to_string()));
        }
        if payload.size_bytes == 0 {
            return Err(ApiError::BadRequest("size_bytes must be > 0".to_string()));
        }
        if payload.size_bytes > MAX_ATTACHMENT_SIZE_BYTES {
            return Err(ApiError::BadRequest(
                "file size exceeds 100MB limit".to_string(),
            ));
        }

        let now = Utc::now().timestamp();
        let upload_id = Uuid::new_v4();
        let key = format!(
            "workspace/{}/channel/{}/uploads/{}-{}",
            context.workspace_id,
            payload.channel_id,
            upload_id,
            sanitize_filename(&filename)
        );

        let (bucket, upload_url) = if let Some(object_storage) = &self.object_storage {
            let url = object_storage
                .presign_upload_url(&key, &content_type, payload.size_bytes)
                .await?;
            (object_storage.bucket.clone(), url)
        } else {
            (
                "galynx-attachments".to_string(),
                format!("https://storage.galynx.local/upload/{upload_id}"),
            )
        };

        let pending = PendingUploadRecord {
            workspace_id: context.workspace_id,
            channel_id: payload.channel_id,
            uploader_id: context.user_id,
            filename,
            content_type,
            size_bytes: payload.size_bytes,
            storage_key: key.clone(),
            expires_at: now + PRESIGN_TTL_SECONDS,
            created_at: now,
        };

        self.storage.put_pending_upload(upload_id, pending).await;
        Ok(PresignResponse {
            upload_id,
            upload_url,
            bucket,
            key,
            expires_at: now + PRESIGN_TTL_SECONDS,
        })
    }

    pub async fn commit(
        &self,
        context: &AuthContext,
        payload: CommitRequest,
    ) -> ApiResult<AttachmentResponse> {
        let now = Utc::now().timestamp();
        let pending = self
            .storage
            .take_pending_upload(&payload.upload_id)
            .await
            .ok_or_else(|| {
                ApiError::NotFound("upload_id not found or already committed".to_string())
            })?;
        if pending.workspace_id != context.workspace_id {
            return Err(ApiError::NotFound("upload_id not found".to_string()));
        }
        if pending.uploader_id != context.user_id {
            return Err(ApiError::Unauthorized(
                "cannot commit upload from another user".to_string(),
            ));
        }
        if pending.expires_at < now {
            return Err(ApiError::BadRequest(
                "presigned upload has expired".to_string(),
            ));
        }

        let (bucket, region) = if let Some(object_storage) = &self.object_storage {
            (object_storage.bucket.clone(), object_storage.region.clone())
        } else {
            ("galynx-attachments".to_string(), "us-east-1".to_string())
        };

        let attachment = AttachmentRecordStore {
            id: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            workspace_id: pending.workspace_id,
            channel_id: pending.channel_id,
            message_id: payload.message_id,
            uploader_id: pending.uploader_id,
            filename: pending.filename,
            content_type: pending.content_type,
            size_bytes: pending.size_bytes,
            bucket,
            key: pending.storage_key,
            region,
            created_at: pending.created_at,
        };
        let response = AttachmentResponse::from(&attachment);
        self.storage.put_attachment(attachment).await;
        Ok(response)
    }

    pub async fn get(
        &self,
        workspace_id: Uuid,
        attachment_id: Uuid,
    ) -> ApiResult<AttachmentGetResponse> {
        let attachment = self
            .storage
            .get_attachment(&attachment_id)
            .await
            .ok_or_else(|| ApiError::NotFound("attachment not found".to_string()))?;
        if attachment.workspace_id != workspace_id {
            return Err(ApiError::NotFound("attachment not found".to_string()));
        }

        let expires_at = Utc::now().timestamp() + DOWNLOAD_TTL_SECONDS;
        let download_url = if let Some(object_storage) = &self.object_storage {
            object_storage
                .presign_download_url(&attachment.key)
                .await
                .map_err(|error| {
                    ApiError::Internal(format!("failed to presign download url: {error}"))
                })?
        } else {
            format!(
                "https://storage.galynx.local/download/{}/{}?exp={}",
                attachment.bucket, attachment.id, expires_at
            )
        };

        Ok(AttachmentGetResponse {
            attachment: AttachmentResponse::from(&attachment),
            download_url,
            expires_at,
        })
    }
}

impl S3ObjectStorage {
    async fn from_config(config: &Config) -> Option<Self> {
        let bucket = config.s3_bucket.clone()?;

        let region_provider =
            RegionProviderChain::first_try(Some(Region::new(config.s3_region.clone())))
                .or_default_provider();

        let mut loader = aws_config::defaults(BehaviorVersion::latest()).region(region_provider);

        if let (Some(access_key), Some(secret_key)) = (
            config.s3_access_key_id.clone(),
            config.s3_secret_access_key.clone(),
        ) {
            loader = loader.credentials_provider(Credentials::new(
                access_key,
                secret_key,
                None,
                None,
                "galynx-config",
            ));
        }

        let shared_config = loader.load().await;
        let presign_client = build_s3_client(
            &shared_config,
            config
                .s3_public_endpoint
                .as_deref()
                .or(config.s3_endpoint.as_deref()),
            config.s3_force_path_style,
        );

        Some(Self {
            presign_client,
            bucket,
            region: config.s3_region.clone(),
        })
    }

    async fn presign_upload_url(
        &self,
        key: &str,
        _content_type: &str,
        _size_bytes: u64,
    ) -> ApiResult<String> {
        let expires = Duration::from_secs(PRESIGN_TTL_SECONDS as u64);
        // Keep presign upload compatible with S3-compatible providers (e.g. RustFS)
        // that can be strict/inconsistent validating additional signed headers.
        // We still validate metadata in API, but only sign host for upload URL.
        let presigned = self
            .presign_client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(
                PresigningConfig::expires_in(expires)
                    .map_err(|error| ApiError::Internal(format!("invalid presign ttl: {error}")))?,
            )
            .await
            .map_err(|error| {
                ApiError::Internal(format!("failed to presign upload url: {error}"))
            })?;

        Ok(presigned.uri().to_string())
    }

    async fn presign_download_url(&self, key: &str) -> Result<String, String> {
        let expires = Duration::from_secs(DOWNLOAD_TTL_SECONDS as u64);
        let presigned = self
            .presign_client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(PresigningConfig::expires_in(expires).map_err(|error| error.to_string())?)
            .await
            .map_err(|error| error.to_string())?;

        Ok(presigned.uri().to_string())
    }
}

fn build_s3_client(
    shared_config: &aws_config::SdkConfig,
    endpoint: Option<&str>,
    force_path_style: bool,
) -> S3Client {
    let mut s3_builder = S3ConfigBuilder::from(shared_config);
    if let Some(endpoint) = endpoint {
        s3_builder = s3_builder.endpoint_url(endpoint);
    }
    s3_builder = s3_builder.force_path_style(force_path_style);
    S3Client::from_conf(s3_builder.build())
}

impl From<&AttachmentRecordStore> for AttachmentResponse {
    fn from(record: &AttachmentRecordStore) -> Self {
        Self {
            id: record.id,
            workspace_id: record.workspace_id,
            channel_id: record.channel_id,
            message_id: record.message_id,
            uploader_id: record.uploader_id,
            filename: record.filename.clone(),
            content_type: record.content_type.clone(),
            size_bytes: record.size_bytes,
            storage_bucket: record.bucket.clone(),
            storage_key: record.key.clone(),
            storage_region: record.region.clone(),
            created_at: record.created_at,
        }
    }
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || char == '.' || char == '-' || char == '_' {
                char
            } else {
                '_'
            }
        })
        .collect()
}

#[utoipa::path(
    post,
    path = "/api/v1/attachments/presign",
    request_body = PresignRequest,
    responses(
        (status = 200, description = "Generated presigned upload URL", body = PresignResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 400, description = "Validation error", body = ErrorResponse)
    )
)]
pub(crate) async fn presign(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PresignRequest>,
) -> ApiResult<Json<PresignResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    state
        .channels
        .ensure_channel_access(&context, payload.channel_id)
        .await?;
    let response = state.attachments.presign(&context, payload).await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "ATTACHMENT_PRESIGN",
            "attachment",
            Some(response.upload_id.to_string()),
            json!({ "key": response.key, "expires_at": response.expires_at }),
        )
        .await;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/attachments/commit",
    request_body = CommitRequest,
    responses(
        (status = 200, description = "Committed uploaded attachment", body = AttachmentResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Upload not found", body = ErrorResponse)
    )
)]
pub(crate) async fn commit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CommitRequest>,
) -> ApiResult<Json<AttachmentResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let response = state.attachments.commit(&context, payload).await?;
    state
        .audit
        .write(
            context.workspace_id,
            Some(context.user_id),
            "ATTACHMENT_COMMIT",
            "attachment",
            Some(response.id.to_string()),
            json!({ "channel_id": response.channel_id, "message_id": response.message_id }),
        )
        .await;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/attachments/{id}",
    responses(
        (status = 200, description = "Attachment metadata + download URL", body = AttachmentGetResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Attachment not found", body = ErrorResponse)
    )
)]
pub(crate) async fn get_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attachment_id): Path<Uuid>,
) -> ApiResult<Json<AttachmentGetResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    let response = state
        .attachments
        .get(context.workspace_id, attachment_id)
        .await?;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::WorkspaceRole;
    use crate::storage::{PersistenceBackend, Storage};

    #[tokio::test]
    async fn presign_and_commit_attachment_success() {
        let service = AttachmentService::new_without_object_storage(Arc::new(
            Storage::new(PersistenceBackend::Memory, None)
                .await
                .expect("memory storage should init"),
        ));
        let context = AuthContext {
            user_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            role: WorkspaceRole::Owner,
        };
        let presign = service
            .presign(
                &context,
                PresignRequest {
                    channel_id: Uuid::new_v4(),
                    filename: "design doc.pdf".to_string(),
                    content_type: "application/pdf".to_string(),
                    size_bytes: 1024,
                },
            )
            .await
            .expect("presign should succeed");
        let commit = service
            .commit(
                &context,
                CommitRequest {
                    upload_id: presign.upload_id,
                    message_id: None,
                },
            )
            .await
            .expect("commit should succeed");
        assert_eq!(commit.filename, "design doc.pdf");
    }
}
