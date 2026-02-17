use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    routing::get,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::{AuthContext, WorkspaceRole},
    errors::{ApiError, ApiResult, ErrorResponse},
    storage::{AuditEntryRecord, Storage},
};

#[derive(Clone)]
pub struct AuditService {
    storage: Arc<Storage>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub actor_id: Option<Uuid>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub metadata: Value,
    pub created_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditListResponse {
    pub items: Vec<AuditLogResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/audit", get(list_audit))
}

impl AuditService {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    pub async fn write(
        &self,
        workspace_id: Uuid,
        actor_id: Option<Uuid>,
        action: &str,
        target_type: &str,
        target_id: Option<String>,
        metadata: Value,
    ) {
        let entry = AuditEntryRecord {
            id: Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)),
            workspace_id,
            actor_id,
            action: action.to_string(),
            target_type: target_type.to_string(),
            target_id,
            metadata,
            created_at: Utc::now().timestamp_millis(),
        };

        self.storage.append_audit_entry(entry).await;
    }

    pub async fn list(&self, workspace_id: Uuid, query: &AuditQuery) -> ApiResult<AuditListResponse> {
        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let before = query
            .cursor
            .as_deref()
            .map(parse_cursor)
            .transpose()
            .map_err(|error| ApiError::BadRequest(format!("invalid cursor: {error}")))?;

        let entries = self.storage.list_audit_entries(workspace_id).await;
        let mut filtered = entries
            .iter()
            .filter(|entry| {
                before.is_none_or(|(cursor_ts, cursor_id)| {
                    (entry.created_at, entry.id.as_u128()) < (cursor_ts, cursor_id)
                })
            })
            .collect::<Vec<_>>();
        filtered.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.as_u128().cmp(&a.id.as_u128()))
        });

        let has_more = filtered.len() > limit;
        let items = filtered
            .into_iter()
            .take(limit)
            .map(AuditLogResponse::from)
            .collect::<Vec<_>>();
        let next_cursor = if has_more {
            items
                .last()
                .map(|item| format!("{}:{}", item.created_at, item.id.as_u128()))
        } else {
            None
        };

        Ok(AuditListResponse { items, next_cursor })
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

impl From<&AuditEntryRecord> for AuditLogResponse {
    fn from(entry: &AuditEntryRecord) -> Self {
        Self {
            id: entry.id,
            workspace_id: entry.workspace_id,
            actor_id: entry.actor_id,
            action: entry.action.clone(),
            target_type: entry.target_type.clone(),
            target_id: entry.target_id.clone(),
            metadata: entry.metadata.clone(),
            created_at: entry.created_at,
        }
    }
}

fn ensure_audit_access(context: &AuthContext) -> ApiResult<()> {
    match context.role {
        WorkspaceRole::Owner | WorkspaceRole::Admin => Ok(()),
        WorkspaceRole::Member => Err(ApiError::Unauthorized(
            "you do not have permission to read audit logs".to_string(),
        )),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/audit",
    params(AuditQuery),
    responses(
        (status = 200, description = "Audit logs", body = AuditListResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse)
    )
)]
pub(crate) async fn list_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> ApiResult<Json<AuditListResponse>> {
    let context = state
        .auth
        .authenticate_headers(&headers, &state.config.jwt_secret)
        .await?;
    ensure_audit_access(&context)?;
    let page = state.audit.list(context.workspace_id, &query).await?;
    Ok(Json(page))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PersistenceBackend, Storage};

    #[tokio::test]
    async fn cursor_pagination_for_audit_entries() {
        let workspace_id = Uuid::new_v4();
        let service = AuditService::new(Arc::new(
            Storage::new(PersistenceBackend::Memory, None)
                .await
                .expect("memory storage should init"),
        ));
        for idx in 0..3 {
            service
                .write(
                    workspace_id,
                    None,
                    "TEST_ACTION",
                    "test",
                    Some(idx.to_string()),
                    serde_json::json!({ "idx": idx }),
                )
                .await;
        }

        let first_page = service
            .list(
                workspace_id,
                &AuditQuery {
                    cursor: None,
                    limit: Some(2),
                },
            )
            .await
            .expect("first page should work");
        assert_eq!(first_page.items.len(), 2);
        assert!(first_page.next_cursor.is_some());

        let second_page = service
            .list(
                workspace_id,
                &AuditQuery {
                    cursor: first_page.next_cursor,
                    limit: Some(2),
                },
            )
            .await
            .expect("second page should work");
        assert_eq!(second_page.items.len(), 1);
    }
}
