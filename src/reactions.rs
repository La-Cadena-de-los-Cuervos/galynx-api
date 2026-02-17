use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::AuthContext,
    channels::ChannelService,
    errors::{ApiError, ApiResult},
    storage::Storage,
};

#[derive(Clone)]
pub struct ReactionService {
    storage: std::sync::Arc<Storage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReactionUpdateResponse {
    pub message_id: Uuid,
    pub channel_id: Uuid,
    pub workspace_id: Uuid,
    pub emoji: String,
    pub count: usize,
    pub user_ids: Vec<Uuid>,
    pub op: String,
}

impl ReactionService {
    pub fn new(storage: std::sync::Arc<Storage>) -> Self {
        Self { storage }
    }

    pub async fn add_reaction(
        &self,
        channels: &ChannelService,
        context: &AuthContext,
        message_id: Uuid,
        emoji: &str,
    ) -> ApiResult<ReactionUpdateResponse> {
        let emoji = normalize_emoji(emoji)?;
        let message = channels
            .get_message(context.workspace_id, message_id)
            .await?;

        self.storage
            .add_reaction(message_id, &emoji, context.user_id)
            .await;
        let user_ids = self.storage.list_reaction_users(message_id, &emoji).await;

        Ok(build_update(
            user_ids,
            message_id,
            message.channel_id,
            context.workspace_id,
            &emoji,
            "added",
        ))
    }

    pub async fn remove_reaction(
        &self,
        channels: &ChannelService,
        context: &AuthContext,
        message_id: Uuid,
        emoji: &str,
    ) -> ApiResult<ReactionUpdateResponse> {
        let emoji = normalize_emoji(emoji)?;
        let message = channels
            .get_message(context.workspace_id, message_id)
            .await?;

        self.storage
            .remove_reaction(message_id, &emoji, context.user_id)
            .await;
        let user_ids = self.storage.list_reaction_users(message_id, &emoji).await;

        Ok(build_update(
            user_ids,
            message_id,
            message.channel_id,
            context.workspace_id,
            &emoji,
            "removed",
        ))
    }
}

fn normalize_emoji(emoji: &str) -> ApiResult<String> {
    let normalized = emoji.trim().to_string();
    if normalized.is_empty() {
        return Err(ApiError::BadRequest("emoji is required".to_string()));
    }
    if normalized.chars().count() > 32 {
        return Err(ApiError::BadRequest("emoji is too long".to_string()));
    }
    Ok(normalized)
}

fn build_update(
    mut user_ids: Vec<Uuid>,
    message_id: Uuid,
    channel_id: Uuid,
    workspace_id: Uuid,
    emoji: &str,
    op: &str,
) -> ReactionUpdateResponse {
    user_ids.sort_unstable();
    user_ids.dedup();
    let count = user_ids.len();

    ReactionUpdateResponse {
        message_id,
        channel_id,
        workspace_id,
        emoji: emoji.to_string(),
        count,
        user_ids,
        op: op.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::WorkspaceRole;
    use crate::storage::{PersistenceBackend, Storage};

    #[tokio::test]
    async fn add_and_remove_reaction_updates_count() {
        let workspace_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let storage = std::sync::Arc::new(
            Storage::new(PersistenceBackend::Memory, None)
                .await
                .expect("memory storage should init"),
        );
        let channels = ChannelService::new(storage.clone(), workspace_id, user_id);
        let context = AuthContext {
            user_id,
            workspace_id,
            role: WorkspaceRole::Owner,
        };
        let channel_id = channels
            .list_channels(workspace_id)
            .await
            .first()
            .expect("channel should exist")
            .id;
        let message = channels
            .create_message(
                &context,
                channel_id,
                crate::channels::CreateMessageRequest {
                    body_md: "hello".to_string(),
                },
            )
            .await
            .expect("message should be created");

        let service = ReactionService::new(storage);
        let added = service
            .add_reaction(&channels, &context, message.id, "👍")
            .await
            .expect("reaction add should work");
        assert_eq!(added.count, 1);

        let removed = service
            .remove_reaction(&channels, &context, message.id, "👍")
            .await
            .expect("reaction remove should work");
        assert_eq!(removed.count, 0);
    }
}
