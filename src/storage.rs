use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use mongodb::{
    Client, Collection,
    bson::{Bson, Document, doc, from_bson, to_bson},
};
use serde_json::Value;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistenceBackend {
    Memory,
    Mongo,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageInitError {
    #[error("mongo backend requires MONGO_URI")]
    MissingMongoUri,
    #[error("mongo initialization failed: {0}")]
    MongoInit(#[from] mongodb::error::Error),
}

#[derive(Clone)]
pub struct Storage {
    backend: PersistenceBackend,
    mongo: Option<MongoState>,
    audit_entries: Arc<RwLock<Vec<AuditEntryRecord>>>,
    pending_uploads: Arc<RwLock<HashMap<Uuid, PendingUploadRecord>>>,
    attachments: Arc<RwLock<HashMap<Uuid, AttachmentRecordStore>>>,
    reactions: Arc<RwLock<HashSet<(Uuid, String, Uuid)>>>,
    channels: Arc<RwLock<HashMap<Uuid, ChannelRecordStore>>>,
    messages: Arc<RwLock<HashMap<Uuid, MessageRecordStore>>>,
    auth_users: Arc<RwLock<HashMap<Uuid, AuthUserRecordStore>>>,
    auth_users_by_email: Arc<RwLock<HashMap<String, Uuid>>>,
    auth_memberships: Arc<RwLock<HashMap<(Uuid, Uuid), String>>>,
    refresh_sessions: Arc<RwLock<HashMap<String, RefreshSessionRecordStore>>>,
}

#[derive(Clone)]
struct MongoState {
    audit_entries: Collection<Document>,
    pending_uploads: Collection<Document>,
    attachments: Collection<Document>,
    reactions: Collection<Document>,
    channels: Collection<Document>,
    messages: Collection<Document>,
    auth_users: Collection<Document>,
    auth_memberships: Collection<Document>,
    refresh_sessions: Collection<Document>,
}

#[derive(Debug, Clone)]
pub struct AuditEntryRecord {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub actor_id: Option<Uuid>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub metadata: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct PendingUploadRecord {
    pub workspace_id: Uuid,
    pub channel_id: Uuid,
    pub uploader_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub storage_key: String,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct AttachmentRecordStore {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub channel_id: Uuid,
    pub message_id: Option<Uuid>,
    pub uploader_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub bucket: String,
    pub key: String,
    pub region: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct ChannelRecordStore {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub is_private: bool,
    pub created_by: Uuid,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct MessageRecordStore {
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

#[derive(Debug, Clone)]
pub struct AuthUserRecordStore {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub password_hash: String,
}

#[derive(Debug, Clone)]
pub struct RefreshSessionRecordStore {
    pub user_id: Uuid,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
    pub replaced_by_hash: Option<String>,
}

impl Storage {
    pub async fn new(
        backend: PersistenceBackend,
        mongo_uri: Option<&str>,
    ) -> Result<Self, StorageInitError> {
        let mongo = if matches!(backend, PersistenceBackend::Mongo) {
            let uri = mongo_uri
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or(StorageInitError::MissingMongoUri)?;
            let client = Client::with_uri_str(uri).await?;
            let database = client.database("galynx");
            Some(MongoState {
                audit_entries: database.collection::<Document>("audit_log"),
                pending_uploads: database.collection::<Document>("pending_uploads"),
                attachments: database.collection::<Document>("attachments"),
                reactions: database.collection::<Document>("reactions"),
                channels: database.collection::<Document>("channels"),
                messages: database.collection::<Document>("messages"),
                auth_users: database.collection::<Document>("auth_users"),
                auth_memberships: database.collection::<Document>("auth_memberships"),
                refresh_sessions: database.collection::<Document>("refresh_sessions"),
            })
        } else {
            None
        };

        Ok(Self {
            backend,
            mongo,
            audit_entries: Arc::new(RwLock::new(Vec::new())),
            pending_uploads: Arc::new(RwLock::new(HashMap::new())),
            attachments: Arc::new(RwLock::new(HashMap::new())),
            reactions: Arc::new(RwLock::new(HashSet::new())),
            channels: Arc::new(RwLock::new(HashMap::new())),
            messages: Arc::new(RwLock::new(HashMap::new())),
            auth_users: Arc::new(RwLock::new(HashMap::new())),
            auth_users_by_email: Arc::new(RwLock::new(HashMap::new())),
            auth_memberships: Arc::new(RwLock::new(HashMap::new())),
            refresh_sessions: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn backend(&self) -> PersistenceBackend {
        self.backend
    }

    pub async fn append_audit_entry(&self, entry: AuditEntryRecord) {
        self.audit_entries.write().await.push(entry.clone());
        if let Some(mongo) = &self.mongo {
            let document = doc! {
                "_id": entry.id.to_string(),
                "workspace_id": entry.workspace_id.to_string(),
                "actor_id": entry.actor_id.map(|value| value.to_string()),
                "action": entry.action,
                "target_type": entry.target_type,
                "target_id": entry.target_id,
                "metadata": to_bson(&entry.metadata).unwrap_or(Bson::Null),
                "created_at": entry.created_at,
            };
            if let Err(error) = mongo.audit_entries.insert_one(document).await {
                tracing::warn!("failed to persist audit entry to mongo: {}", error);
            }
        }
    }

    pub async fn list_audit_entries(&self, workspace_id: Uuid) -> Vec<AuditEntryRecord> {
        if let Some(mongo) = &self.mongo {
            let filter = doc! { "workspace_id": workspace_id.to_string() };
            if let Ok(mut cursor) = mongo.audit_entries.find(filter).await {
                let mut items = Vec::new();
                while let Ok(true) = cursor.advance().await {
                    let Ok(document) = cursor.deserialize_current() else {
                        continue;
                    };

                    let id = document
                        .get_str("_id")
                        .ok()
                        .and_then(|value| Uuid::parse_str(value).ok());
                    let actor_id = document
                        .get_str("actor_id")
                        .ok()
                        .and_then(|value| Uuid::parse_str(value).ok());
                    let metadata = document
                        .get("metadata")
                        .cloned()
                        .and_then(|value| from_bson::<Value>(value).ok())
                        .unwrap_or(Value::Null);
                    let Some(id) = id else {
                        continue;
                    };

                    items.push(AuditEntryRecord {
                        id,
                        workspace_id,
                        actor_id,
                        action: document.get_str("action").unwrap_or_default().to_string(),
                        target_type: document
                            .get_str("target_type")
                            .unwrap_or_default()
                            .to_string(),
                        target_id: document.get_str("target_id").ok().map(ToString::to_string),
                        metadata,
                        created_at: document.get_i64("created_at").unwrap_or_default(),
                    });
                }
                return items;
            } else {
                tracing::warn!("failed to read audit entries from mongo, using memory fallback");
            }
        }

        self.audit_entries
            .read()
            .await
            .iter()
            .filter(|entry| entry.workspace_id == workspace_id)
            .cloned()
            .collect()
    }

    pub async fn put_pending_upload(&self, upload_id: Uuid, pending: PendingUploadRecord) {
        self.pending_uploads
            .write()
            .await
            .insert(upload_id, pending.clone());
        if let Some(mongo) = &self.mongo {
            let document = doc! {
                "_id": upload_id.to_string(),
                "workspace_id": pending.workspace_id.to_string(),
                "channel_id": pending.channel_id.to_string(),
                "uploader_id": pending.uploader_id.to_string(),
                "filename": pending.filename,
                "content_type": pending.content_type,
                "size_bytes": pending.size_bytes as i64,
                "storage_key": pending.storage_key,
                "expires_at": pending.expires_at,
                "created_at": pending.created_at,
            };
            let _ = mongo
                .pending_uploads
                .delete_one(doc! { "_id": upload_id.to_string() })
                .await;
            if let Err(error) = mongo.pending_uploads.insert_one(document).await {
                tracing::warn!("failed to persist pending upload to mongo: {}", error);
            }
        }
    }

    pub async fn take_pending_upload(&self, upload_id: &Uuid) -> Option<PendingUploadRecord> {
        let in_memory = self.pending_uploads.write().await.remove(upload_id);
        if let Some(mongo) = &self.mongo {
            let deleted = mongo
                .pending_uploads
                .find_one_and_delete(doc! { "_id": upload_id.to_string() })
                .await;
            if let Ok(Some(document)) = deleted {
                return Some(PendingUploadRecord {
                    workspace_id: uuid_field(&document, "workspace_id")?,
                    channel_id: uuid_field(&document, "channel_id")?,
                    uploader_id: uuid_field(&document, "uploader_id")?,
                    filename: string_field(&document, "filename").unwrap_or_default(),
                    content_type: string_field(&document, "content_type").unwrap_or_default(),
                    size_bytes: i64_field(&document, "size_bytes").unwrap_or_default() as u64,
                    storage_key: string_field(&document, "storage_key").unwrap_or_default(),
                    expires_at: i64_field(&document, "expires_at").unwrap_or_default(),
                    created_at: i64_field(&document, "created_at").unwrap_or_default(),
                });
            }
        }
        in_memory
    }

    pub async fn put_attachment(&self, attachment: AttachmentRecordStore) {
        self.attachments
            .write()
            .await
            .insert(attachment.id, attachment.clone());
        if let Some(mongo) = &self.mongo {
            let document = doc! {
                "_id": attachment.id.to_string(),
                "workspace_id": attachment.workspace_id.to_string(),
                "channel_id": attachment.channel_id.to_string(),
                "message_id": attachment.message_id.map(|value| value.to_string()),
                "uploader_id": attachment.uploader_id.to_string(),
                "filename": attachment.filename,
                "content_type": attachment.content_type,
                "size_bytes": attachment.size_bytes as i64,
                "bucket": attachment.bucket,
                "key": attachment.key,
                "region": attachment.region,
                "created_at": attachment.created_at,
            };
            let _ = mongo
                .attachments
                .delete_one(doc! { "_id": attachment.id.to_string() })
                .await;
            if let Err(error) = mongo.attachments.insert_one(document).await {
                tracing::warn!("failed to persist attachment to mongo: {}", error);
            }
        }
    }

    pub async fn get_attachment(&self, attachment_id: &Uuid) -> Option<AttachmentRecordStore> {
        if let Some(mongo) = &self.mongo {
            let found = mongo
                .attachments
                .find_one(doc! { "_id": attachment_id.to_string() })
                .await;
            if let Ok(Some(document)) = found {
                return Some(AttachmentRecordStore {
                    id: uuid_field(&document, "_id")?,
                    workspace_id: uuid_field(&document, "workspace_id")?,
                    channel_id: uuid_field(&document, "channel_id")?,
                    message_id: optional_uuid_field(&document, "message_id"),
                    uploader_id: uuid_field(&document, "uploader_id")?,
                    filename: string_field(&document, "filename").unwrap_or_default(),
                    content_type: string_field(&document, "content_type").unwrap_or_default(),
                    size_bytes: i64_field(&document, "size_bytes").unwrap_or_default() as u64,
                    bucket: string_field(&document, "bucket").unwrap_or_default(),
                    key: string_field(&document, "key").unwrap_or_default(),
                    region: string_field(&document, "region").unwrap_or_default(),
                    created_at: i64_field(&document, "created_at").unwrap_or_default(),
                });
            }
        }
        self.attachments.read().await.get(attachment_id).cloned()
    }

    pub async fn add_reaction(&self, message_id: Uuid, emoji: &str, user_id: Uuid) {
        self.reactions
            .write()
            .await
            .insert((message_id, emoji.to_string(), user_id));
        if let Some(mongo) = &self.mongo {
            let reaction_id = format!("{message_id}:{emoji}:{user_id}");
            let document = doc! {
                "_id": reaction_id,
                "message_id": message_id.to_string(),
                "emoji": emoji,
                "user_id": user_id.to_string(),
            };
            let _ = mongo
                .reactions
                .delete_one(doc! { "_id": format!("{message_id}:{emoji}:{user_id}") })
                .await;
            let _ = mongo.reactions.insert_one(document).await;
        }
    }

    pub async fn remove_reaction(&self, message_id: Uuid, emoji: &str, user_id: Uuid) {
        self.reactions
            .write()
            .await
            .remove(&(message_id, emoji.to_string(), user_id));
        if let Some(mongo) = &self.mongo {
            let _ = mongo
                .reactions
                .delete_one(doc! { "_id": format!("{message_id}:{emoji}:{user_id}") })
                .await;
        }
    }

    pub async fn list_reaction_users(&self, message_id: Uuid, emoji: &str) -> Vec<Uuid> {
        if let Some(mongo) = &self.mongo {
            let mut users = Vec::new();
            if let Ok(mut cursor) = mongo
                .reactions
                .find(doc! { "message_id": message_id.to_string(), "emoji": emoji })
                .await
            {
                while let Ok(true) = cursor.advance().await {
                    let Ok(document) = cursor.deserialize_current() else {
                        continue;
                    };
                    if let Some(user_id) = uuid_field(&document, "user_id") {
                        users.push(user_id);
                    }
                }
                return users;
            }
        }

        self.reactions
            .read()
            .await
            .iter()
            .filter_map(|(msg_id, stored_emoji, user_id)| {
                (*msg_id == message_id && stored_emoji == emoji).then_some(*user_id)
            })
            .collect()
    }

    pub async fn insert_channel(&self, channel: ChannelRecordStore) {
        self.channels
            .write()
            .await
            .insert(channel.id, channel.clone());
        if let Some(mongo) = &self.mongo {
            let document = doc! {
                "_id": channel.id.to_string(),
                "workspace_id": channel.workspace_id.to_string(),
                "name": channel.name,
                "is_private": channel.is_private,
                "created_by": channel.created_by.to_string(),
                "created_at": channel.created_at,
            };
            let _ = mongo
                .channels
                .delete_one(doc! { "_id": channel.id.to_string() })
                .await;
            let _ = mongo.channels.insert_one(document).await;
        }
    }

    pub async fn list_channels(&self, workspace_id: Uuid) -> Vec<ChannelRecordStore> {
        if let Some(mongo) = &self.mongo {
            let mut channels = Vec::new();
            if let Ok(mut cursor) = mongo
                .channels
                .find(doc! { "workspace_id": workspace_id.to_string() })
                .await
            {
                while let Ok(true) = cursor.advance().await {
                    let Ok(document) = cursor.deserialize_current() else {
                        continue;
                    };
                    if let (Some(id), Some(created_by)) = (
                        uuid_field(&document, "_id"),
                        uuid_field(&document, "created_by"),
                    ) {
                        channels.push(ChannelRecordStore {
                            id,
                            workspace_id,
                            name: string_field(&document, "name").unwrap_or_default(),
                            is_private: bool_field(&document, "is_private").unwrap_or(false),
                            created_by,
                            created_at: i64_field(&document, "created_at").unwrap_or_default(),
                        });
                    }
                }
                return channels;
            }
        }

        self.channels
            .read()
            .await
            .values()
            .filter(|channel| channel.workspace_id == workspace_id)
            .cloned()
            .collect()
    }

    pub async fn get_channel(&self, channel_id: &Uuid) -> Option<ChannelRecordStore> {
        if let Some(mongo) = &self.mongo {
            let found = mongo
                .channels
                .find_one(doc! { "_id": channel_id.to_string() })
                .await;
            if let Ok(Some(document)) = found {
                return Some(ChannelRecordStore {
                    id: uuid_field(&document, "_id")?,
                    workspace_id: uuid_field(&document, "workspace_id")?,
                    name: string_field(&document, "name").unwrap_or_default(),
                    is_private: bool_field(&document, "is_private").unwrap_or(false),
                    created_by: uuid_field(&document, "created_by")?,
                    created_at: i64_field(&document, "created_at").unwrap_or_default(),
                });
            }
        }
        self.channels.read().await.get(channel_id).cloned()
    }

    pub async fn remove_channel(&self, channel_id: &Uuid) -> Option<ChannelRecordStore> {
        let deleted = self.channels.write().await.remove(channel_id);
        if let Some(mongo) = &self.mongo {
            let result = mongo
                .channels
                .find_one_and_delete(doc! { "_id": channel_id.to_string() })
                .await;
            if let Ok(Some(document)) = result {
                return Some(ChannelRecordStore {
                    id: uuid_field(&document, "_id")?,
                    workspace_id: uuid_field(&document, "workspace_id")?,
                    name: string_field(&document, "name").unwrap_or_default(),
                    is_private: bool_field(&document, "is_private").unwrap_or(false),
                    created_by: uuid_field(&document, "created_by")?,
                    created_at: i64_field(&document, "created_at").unwrap_or_default(),
                });
            }
        }
        deleted
    }

    pub async fn channel_name_exists(&self, workspace_id: Uuid, name: &str) -> bool {
        if let Some(mongo) = &self.mongo {
            if let Ok(result) = mongo
                .channels
                .find_one(doc! { "workspace_id": workspace_id.to_string(), "name": name.to_ascii_lowercase() })
                .await
            {
                return result.is_some();
            }
        }

        self.channels.read().await.values().any(|channel| {
            channel.workspace_id == workspace_id && channel.name.eq_ignore_ascii_case(name)
        })
    }

    pub async fn insert_message(&self, message: MessageRecordStore) {
        self.messages
            .write()
            .await
            .insert(message.id, message.clone());
        if let Some(mongo) = &self.mongo {
            let document = doc! {
                "_id": message.id.to_string(),
                "workspace_id": message.workspace_id.to_string(),
                "channel_id": message.channel_id.to_string(),
                "sender_id": message.sender_id.to_string(),
                "body_md": message.body_md,
                "thread_root_id": message.thread_root_id.map(|value| value.to_string()),
                "created_at": message.created_at,
                "edited_at": message.edited_at,
                "deleted_at": message.deleted_at,
            };
            let _ = mongo
                .messages
                .delete_one(doc! { "_id": message.id.to_string() })
                .await;
            let _ = mongo.messages.insert_one(document).await;
        }
    }

    pub async fn get_message(&self, message_id: &Uuid) -> Option<MessageRecordStore> {
        if let Some(mongo) = &self.mongo {
            let found = mongo
                .messages
                .find_one(doc! { "_id": message_id.to_string() })
                .await;
            if let Ok(Some(document)) = found {
                return Some(MessageRecordStore {
                    id: uuid_field(&document, "_id")?,
                    workspace_id: uuid_field(&document, "workspace_id")?,
                    channel_id: uuid_field(&document, "channel_id")?,
                    sender_id: uuid_field(&document, "sender_id")?,
                    body_md: string_field(&document, "body_md").unwrap_or_default(),
                    thread_root_id: optional_uuid_field(&document, "thread_root_id"),
                    created_at: i64_field(&document, "created_at").unwrap_or_default(),
                    edited_at: optional_i64_field(&document, "edited_at"),
                    deleted_at: optional_i64_field(&document, "deleted_at"),
                });
            }
        }
        self.messages.read().await.get(message_id).cloned()
    }

    pub async fn list_messages(&self, workspace_id: Uuid) -> Vec<MessageRecordStore> {
        if let Some(mongo) = &self.mongo {
            let mut messages = Vec::new();
            if let Ok(mut cursor) = mongo
                .messages
                .find(doc! { "workspace_id": workspace_id.to_string() })
                .await
            {
                while let Ok(true) = cursor.advance().await {
                    let Ok(document) = cursor.deserialize_current() else {
                        continue;
                    };
                    if let (Some(id), Some(channel_id), Some(sender_id)) = (
                        uuid_field(&document, "_id"),
                        uuid_field(&document, "channel_id"),
                        uuid_field(&document, "sender_id"),
                    ) {
                        messages.push(MessageRecordStore {
                            id,
                            workspace_id,
                            channel_id,
                            sender_id,
                            body_md: string_field(&document, "body_md").unwrap_or_default(),
                            thread_root_id: optional_uuid_field(&document, "thread_root_id"),
                            created_at: i64_field(&document, "created_at").unwrap_or_default(),
                            edited_at: optional_i64_field(&document, "edited_at"),
                            deleted_at: optional_i64_field(&document, "deleted_at"),
                        });
                    }
                }
                return messages;
            }
        }

        self.messages
            .read()
            .await
            .values()
            .filter(|message| message.workspace_id == workspace_id)
            .cloned()
            .collect()
    }

    pub async fn update_message(&self, message: MessageRecordStore) {
        self.insert_message(message).await;
    }

    pub async fn remove_messages_for_channel(&self, channel_id: Uuid) {
        self.messages
            .write()
            .await
            .retain(|_, message| message.channel_id != channel_id);
        if let Some(mongo) = &self.mongo {
            let _ = mongo
                .messages
                .delete_many(doc! { "channel_id": channel_id.to_string() })
                .await;
        }
    }

    pub async fn put_auth_user(&self, user: AuthUserRecordStore) {
        self.auth_users_by_email
            .write()
            .await
            .insert(user.email.to_ascii_lowercase(), user.id);
        self.auth_users.write().await.insert(user.id, user.clone());
        if let Some(mongo) = &self.mongo {
            let document = doc! {
                "_id": user.id.to_string(),
                "email": user.email.to_ascii_lowercase(),
                "name": user.name,
                "password_hash": user.password_hash,
            };
            let _ = mongo
                .auth_users
                .delete_one(doc! { "_id": user.id.to_string() })
                .await;
            let _ = mongo.auth_users.insert_one(document).await;
        }
    }

    pub async fn get_auth_user_by_email(&self, email: &str) -> Option<AuthUserRecordStore> {
        if let Some(mongo) = &self.mongo {
            let normalized = email.trim().to_ascii_lowercase();
            let found = mongo
                .auth_users
                .find_one(doc! { "email": normalized })
                .await;
            if let Ok(Some(document)) = found {
                return Some(AuthUserRecordStore {
                    id: uuid_field(&document, "_id")?,
                    email: string_field(&document, "email").unwrap_or_default(),
                    name: string_field(&document, "name").unwrap_or_default(),
                    password_hash: string_field(&document, "password_hash").unwrap_or_default(),
                });
            }
        }

        let normalized = email.trim().to_ascii_lowercase();
        let user_id = self
            .auth_users_by_email
            .read()
            .await
            .get(&normalized)
            .copied()?;
        self.auth_users.read().await.get(&user_id).cloned()
    }

    pub async fn get_auth_user_by_id(&self, user_id: Uuid) -> Option<AuthUserRecordStore> {
        if let Some(mongo) = &self.mongo {
            let found = mongo
                .auth_users
                .find_one(doc! { "_id": user_id.to_string() })
                .await;
            if let Ok(Some(document)) = found {
                return Some(AuthUserRecordStore {
                    id: uuid_field(&document, "_id")?,
                    email: string_field(&document, "email").unwrap_or_default(),
                    name: string_field(&document, "name").unwrap_or_default(),
                    password_hash: string_field(&document, "password_hash").unwrap_or_default(),
                });
            }
        }
        self.auth_users.read().await.get(&user_id).cloned()
    }

    pub async fn put_membership_role(&self, workspace_id: Uuid, user_id: Uuid, role: &str) {
        self.auth_memberships
            .write()
            .await
            .insert((workspace_id, user_id), role.to_string());
        if let Some(mongo) = &self.mongo {
            let id = format!("{workspace_id}:{user_id}");
            let document = doc! {
                "_id": id.clone(),
                "workspace_id": workspace_id.to_string(),
                "user_id": user_id.to_string(),
                "role": role,
            };
            let _ = mongo.auth_memberships.delete_one(doc! { "_id": id }).await;
            let _ = mongo.auth_memberships.insert_one(document).await;
        }
    }

    pub async fn get_membership_role(&self, workspace_id: Uuid, user_id: Uuid) -> Option<String> {
        if let Some(mongo) = &self.mongo {
            let found = mongo
                .auth_memberships
                .find_one(doc! {
                    "workspace_id": workspace_id.to_string(),
                    "user_id": user_id.to_string()
                })
                .await;
            if let Ok(Some(document)) = found {
                return string_field(&document, "role");
            }
        }

        self.auth_memberships
            .read()
            .await
            .get(&(workspace_id, user_id))
            .cloned()
    }

    pub async fn find_primary_membership(&self, user_id: Uuid) -> Option<(Uuid, String)> {
        if let Some(mongo) = &self.mongo {
            if let Ok(mut cursor) = mongo
                .auth_memberships
                .find(doc! { "user_id": user_id.to_string() })
                .await
            {
                if let Ok(true) = cursor.advance().await {
                    let Ok(document) = cursor.deserialize_current() else {
                        return None;
                    };
                    return Some((
                        uuid_field(&document, "workspace_id")?,
                        string_field(&document, "role").unwrap_or_default(),
                    ));
                }
            }
        }

        self.auth_memberships
            .read()
            .await
            .iter()
            .find_map(|((workspace_id, member_id), role)| {
                (*member_id == user_id).then(|| (*workspace_id, role.clone()))
            })
    }

    pub async fn get_refresh_session(&self, token_hash: &str) -> Option<RefreshSessionRecordStore> {
        if let Some(mongo) = &self.mongo {
            let found = mongo
                .refresh_sessions
                .find_one(doc! { "_id": token_hash })
                .await;
            if let Ok(Some(document)) = found {
                return Some(RefreshSessionRecordStore {
                    user_id: uuid_field(&document, "user_id")?,
                    expires_at: i64_field(&document, "expires_at").unwrap_or_default(),
                    revoked_at: optional_i64_field(&document, "revoked_at"),
                    replaced_by_hash: string_field(&document, "replaced_by_hash"),
                });
            }
        }
        self.refresh_sessions.read().await.get(token_hash).cloned()
    }

    pub async fn put_refresh_session(
        &self,
        token_hash: String,
        session: RefreshSessionRecordStore,
    ) {
        self.refresh_sessions
            .write()
            .await
            .insert(token_hash.clone(), session.clone());
        if let Some(mongo) = &self.mongo {
            let document = doc! {
                "_id": token_hash.clone(),
                "user_id": session.user_id.to_string(),
                "expires_at": session.expires_at,
                "revoked_at": session.revoked_at,
                "replaced_by_hash": session.replaced_by_hash,
            };
            let _ = mongo
                .refresh_sessions
                .delete_one(doc! { "_id": token_hash })
                .await;
            let _ = mongo.refresh_sessions.insert_one(document).await;
        }
    }

    pub async fn update_refresh_session(
        &self,
        token_hash: &str,
        update_fn: impl FnOnce(&mut RefreshSessionRecordStore),
    ) -> Option<RefreshSessionRecordStore> {
        let mut session = self.get_refresh_session(token_hash).await?;
        update_fn(&mut session);
        self.put_refresh_session(token_hash.to_string(), session.clone())
            .await;
        Some(session)
    }
}

fn uuid_field(document: &Document, key: &str) -> Option<Uuid> {
    document
        .get_str(key)
        .ok()
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn optional_uuid_field(document: &Document, key: &str) -> Option<Uuid> {
    document
        .get_str(key)
        .ok()
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn string_field(document: &Document, key: &str) -> Option<String> {
    document.get_str(key).ok().map(ToString::to_string)
}

fn i64_field(document: &Document, key: &str) -> Option<i64> {
    document.get_i64(key).ok()
}

fn optional_i64_field(document: &Document, key: &str) -> Option<i64> {
    document.get_i64(key).ok()
}

fn bool_field(document: &Document, key: &str) -> Option<bool> {
    document.get_bool(key).ok()
}
