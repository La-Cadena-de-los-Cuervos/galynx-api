#![allow(dead_code)]

use std::sync::Arc;

use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use chrono::Utc;
use clap::Parser;
use serde::Serialize;
use uuid::Uuid;

#[path = "../config.rs"]
mod config;
#[path = "../storage.rs"]
mod storage;

#[derive(Parser, Debug)]
#[command(
    name = "galynx-bootstrap",
    version,
    about = "Operational bootstrap for galynx-api"
)]
struct BootstrapCli {
    #[arg(long)]
    workspace_name: Option<String>,
    #[arg(long)]
    owner_email: Option<String>,
    #[arg(long)]
    owner_password: Option<String>,
}

#[derive(Debug, Serialize)]
struct BootstrapResult {
    completed_at: i64,
    backend: String,
    workspace_id: String,
    owner_user_id: String,
    owner_email: String,
    workspace_name: String,
    default_channel_id: String,
    created_workspace: bool,
    created_owner: bool,
    created_default_channel: bool,
}

#[tokio::main]
async fn main() {
    let cli = BootstrapCli::parse();

    let mut cfg = config::Config::from_env();
    if let Some(workspace_name) = cli.workspace_name {
        cfg.bootstrap_workspace_name = workspace_name;
    }
    if let Some(owner_email) = cli.owner_email {
        cfg.bootstrap_email = owner_email;
    }
    if let Some(owner_password) = cli.owner_password {
        cfg.bootstrap_password = owner_password;
    }

    let storage = Arc::new(
        storage::Storage::new(cfg.persistence_backend, cfg.mongo_uri.as_deref())
            .await
            .expect("failed to initialize storage"),
    );

    let email = cfg.bootstrap_email.trim().to_ascii_lowercase();
    let owner = if let Some(existing) = storage.get_auth_user_by_email(&email).await {
        (existing, false)
    } else {
        let user = storage::AuthUserRecordStore {
            id: Uuid::new_v4(),
            email: email.clone(),
            name: "Owner".to_string(),
            password_hash: hash_password(&cfg.bootstrap_password)
                .expect("failed to hash owner password"),
        };
        storage.put_auth_user(user.clone()).await;
        (user, true)
    };

    let workspace_name = cfg.bootstrap_workspace_name.trim().to_string();
    let workspace =
        if let Some((workspace_id, _)) = storage.find_primary_membership(owner.0.id).await {
            let existing = storage.get_workspace(workspace_id).await.unwrap_or(
                storage::WorkspaceRecordStore {
                    id: workspace_id,
                    name: workspace_name.clone(),
                    created_by: owner.0.id,
                    created_at: Utc::now().timestamp_millis(),
                },
            );
            storage.put_workspace(existing.clone()).await;
            (existing, false)
        } else {
            let workspace = storage::WorkspaceRecordStore {
                id: Uuid::new_v4(),
                name: workspace_name.clone(),
                created_by: owner.0.id,
                created_at: Utc::now().timestamp_millis(),
            };
            storage.put_workspace(workspace.clone()).await;
            (workspace, true)
        };

    storage
        .put_membership_role(workspace.0.id, owner.0.id, "owner")
        .await;

    let (default_channel_id, created_default_channel) =
        if storage.channel_name_exists(workspace.0.id, "general").await {
            let mut channels = storage.list_channels(workspace.0.id).await;
            channels.sort_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.id.cmp(&b.id))
            });
            let channel = channels
                .into_iter()
                .find(|channel| channel.name.eq_ignore_ascii_case("general"))
                .expect("general channel exists but was not found");
            (channel.id, false)
        } else {
            let channel_id = Uuid::new_v4();
            storage
                .insert_channel(storage::ChannelRecordStore {
                    id: channel_id,
                    workspace_id: workspace.0.id,
                    name: "general".to_string(),
                    is_private: false,
                    created_by: owner.0.id,
                    created_at: Utc::now().timestamp_millis(),
                })
                .await;
            (channel_id, true)
        };

    let backend = match cfg.persistence_backend {
        storage::PersistenceBackend::Memory => "memory",
        storage::PersistenceBackend::Mongo => "mongo",
    };

    let result = BootstrapResult {
        completed_at: Utc::now().timestamp_millis(),
        backend: backend.to_string(),
        workspace_id: workspace.0.id.to_string(),
        owner_user_id: owner.0.id.to_string(),
        owner_email: owner.0.email,
        workspace_name: workspace.0.name,
        default_channel_id: default_channel_id.to_string(),
        created_workspace: workspace.1,
        created_owner: owner.1,
        created_default_channel,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&result).expect("failed to serialize result")
    );
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| format!("failed to hash password: {error}"))
        .map(|hash| hash.to_string())
}
