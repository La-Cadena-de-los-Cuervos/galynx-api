use std::{
    env, fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_BASE_URL: &str = "http://localhost:3000";

#[derive(Parser, Debug)]
#[command(name = "galynx", version, about = "CLI for galynx-api")]
struct Cli {
    #[arg(long, global = true)]
    base_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    Channels {
        #[command(subcommand)]
        command: ChannelCommands,
    },
    Messages {
        #[command(subcommand)]
        command: MessageCommands,
    },
    Threads {
        #[command(subcommand)]
        command: ThreadCommands,
    },
    Attachments {
        #[command(subcommand)]
        command: AttachmentCommands,
    },
    Audit {
        #[command(subcommand)]
        command: AuditCommands,
    },
}

#[derive(Subcommand, Debug)]
enum AuthCommands {
    Login(LoginArgs),
    Me,
    Refresh,
    Logout,
}

#[derive(Args, Debug)]
struct LoginArgs {
    #[arg(long)]
    email: String,
    #[arg(long)]
    password: String,
}

#[derive(Subcommand, Debug)]
enum ChannelCommands {
    List,
    Create(CreateChannelArgs),
    Delete(DeleteChannelArgs),
}

#[derive(Args, Debug)]
struct CreateChannelArgs {
    #[arg(long)]
    name: String,
    #[arg(long = "private", default_value_t = false)]
    is_private: bool,
}

#[derive(Args, Debug)]
struct DeleteChannelArgs {
    channel_id: String,
}

#[derive(Subcommand, Debug)]
enum MessageCommands {
    List(ListMessagesArgs),
    Send(SendMessageArgs),
    Edit(EditMessageArgs),
    Delete(DeleteMessageArgs),
}

#[derive(Args, Debug)]
struct ListMessagesArgs {
    #[arg(long)]
    channel: String,
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Args, Debug)]
struct SendMessageArgs {
    #[arg(long)]
    channel: String,
    #[arg(long)]
    body: String,
}

#[derive(Args, Debug)]
struct EditMessageArgs {
    message_id: String,
    #[arg(long)]
    body: String,
}

#[derive(Args, Debug)]
struct DeleteMessageArgs {
    message_id: String,
}

#[derive(Subcommand, Debug)]
enum ThreadCommands {
    Get(ThreadGetArgs),
    Replies(ThreadRepliesArgs),
    Reply(ThreadReplyArgs),
}

#[derive(Args, Debug)]
struct ThreadGetArgs {
    root_id: String,
}

#[derive(Args, Debug)]
struct ThreadRepliesArgs {
    root_id: String,
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Args, Debug)]
struct ThreadReplyArgs {
    root_id: String,
    #[arg(long)]
    body: String,
}

#[derive(Subcommand, Debug)]
enum AttachmentCommands {
    Presign(AttachmentPresignArgs),
    Commit(AttachmentCommitArgs),
    Get(AttachmentGetArgs),
}

#[derive(Args, Debug)]
struct AttachmentPresignArgs {
    #[arg(long)]
    channel: String,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    filename: Option<String>,
    #[arg(long = "content-type")]
    content_type: Option<String>,
    #[arg(long = "size-bytes")]
    size_bytes: Option<u64>,
}

#[derive(Args, Debug)]
struct AttachmentCommitArgs {
    #[arg(long = "upload-id")]
    upload_id: String,
    #[arg(long = "message-id")]
    message_id: Option<String>,
}

#[derive(Args, Debug)]
struct AttachmentGetArgs {
    attachment_id: String,
}

#[derive(Subcommand, Debug)]
enum AuditCommands {
    List(AuditListArgs),
}

#[derive(Args, Debug)]
struct AuditListArgs {
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSession {
    base_url: String,
    access_token: String,
    refresh_token: String,
    access_expires_at: i64,
    refresh_expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct AuthTokensResponse {
    access_token: String,
    refresh_token: String,
    access_expires_at: i64,
    refresh_expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: String,
    message: String,
}

#[derive(Debug)]
struct CliError {
    message: String,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

type CliResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> CliResult<()> {
    let cli = Cli::parse();
    let client = Client::new();

    match cli.command {
        Command::Auth { command } => run_auth(command, cli.base_url, &client).await,
        Command::Channels { command } => run_channels(command, cli.base_url, &client).await,
        Command::Messages { command } => run_messages(command, cli.base_url, &client).await,
        Command::Threads { command } => run_threads(command, cli.base_url, &client).await,
        Command::Attachments { command } => run_attachments(command, cli.base_url, &client).await,
        Command::Audit { command } => run_audit(command, cli.base_url, &client).await,
    }
}

async fn run_auth(
    command: AuthCommands,
    base_url_flag: Option<String>,
    client: &Client,
) -> CliResult<()> {
    match command {
        AuthCommands::Login(args) => {
            let base_url = resolve_base_url(base_url_flag.as_deref(), None);
            let response = send_json(
                client,
                Method::POST,
                &base_url,
                "/auth/login",
                Some(json!({
                    "email": args.email,
                    "password": args.password,
                })),
                None,
                None,
            )
            .await?;
            let tokens: AuthTokensResponse = parse_json(response).await?;

            save_session(&StoredSession {
                base_url,
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
                access_expires_at: tokens.access_expires_at,
                refresh_expires_at: tokens.refresh_expires_at,
            })?;
            println!("login ok");
            Ok(())
        }
        AuthCommands::Me => {
            let mut session = load_session()?;
            session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));

            let response =
                send_authed_json(client, Method::GET, &mut session, "/me", None, None).await?;
            save_session(&session)?;
            print_json(response).await
        }
        AuthCommands::Refresh => {
            let mut session = load_session()?;
            session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));
            refresh_session(client, &mut session).await?;
            save_session(&session)?;
            println!("refresh ok");
            Ok(())
        }
        AuthCommands::Logout => {
            let mut session = load_session()?;
            session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));
            let refresh_token = session.refresh_token.clone();
            let response = send_authed_json(
                client,
                Method::POST,
                &mut session,
                "/auth/logout",
                Some(json!({ "refresh_token": refresh_token })),
                None,
            )
            .await?;
            if response.status() != StatusCode::NO_CONTENT {
                return Err(Box::new(cli_error(format!(
                    "unexpected logout response: {}",
                    response.status()
                ))));
            }
            clear_session_file()?;
            println!("logout ok");
            Ok(())
        }
    }
}

async fn run_channels(
    command: ChannelCommands,
    base_url_flag: Option<String>,
    client: &Client,
) -> CliResult<()> {
    let mut session = load_session()?;
    session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));

    let response = match command {
        ChannelCommands::List => {
            send_authed_json(client, Method::GET, &mut session, "/channels", None, None).await?
        }
        ChannelCommands::Create(args) => {
            send_authed_json(
                client,
                Method::POST,
                &mut session,
                "/channels",
                Some(json!({
                    "name": args.name,
                    "is_private": args.is_private,
                })),
                None,
            )
            .await?
        }
        ChannelCommands::Delete(args) => {
            let path = format!("/channels/{}", args.channel_id);
            send_authed_json(client, Method::DELETE, &mut session, &path, None, None).await?
        }
    };

    save_session(&session)?;
    print_or_ok(response).await
}

async fn run_messages(
    command: MessageCommands,
    base_url_flag: Option<String>,
    client: &Client,
) -> CliResult<()> {
    let mut session = load_session()?;
    session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));

    let response = match command {
        MessageCommands::List(args) => {
            let path = format!("/channels/{}/messages", args.channel);
            let query = cursor_limit_query(args.cursor, args.limit);
            send_authed_json(client, Method::GET, &mut session, &path, None, Some(query)).await?
        }
        MessageCommands::Send(args) => {
            let path = format!("/channels/{}/messages", args.channel);
            send_authed_json(
                client,
                Method::POST,
                &mut session,
                &path,
                Some(json!({ "body_md": args.body })),
                None,
            )
            .await?
        }
        MessageCommands::Edit(args) => {
            let path = format!("/messages/{}", args.message_id);
            send_authed_json(
                client,
                Method::PATCH,
                &mut session,
                &path,
                Some(json!({ "body_md": args.body })),
                None,
            )
            .await?
        }
        MessageCommands::Delete(args) => {
            let path = format!("/messages/{}", args.message_id);
            send_authed_json(client, Method::DELETE, &mut session, &path, None, None).await?
        }
    };

    save_session(&session)?;
    print_or_ok(response).await
}

async fn run_threads(
    command: ThreadCommands,
    base_url_flag: Option<String>,
    client: &Client,
) -> CliResult<()> {
    let mut session = load_session()?;
    session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));

    let response = match command {
        ThreadCommands::Get(args) => {
            let path = format!("/threads/{}", args.root_id);
            send_authed_json(client, Method::GET, &mut session, &path, None, None).await?
        }
        ThreadCommands::Replies(args) => {
            let path = format!("/threads/{}/replies", args.root_id);
            let query = cursor_limit_query(args.cursor, args.limit);
            send_authed_json(client, Method::GET, &mut session, &path, None, Some(query)).await?
        }
        ThreadCommands::Reply(args) => {
            let path = format!("/threads/{}/replies", args.root_id);
            send_authed_json(
                client,
                Method::POST,
                &mut session,
                &path,
                Some(json!({ "body_md": args.body })),
                None,
            )
            .await?
        }
    };

    save_session(&session)?;
    print_or_ok(response).await
}

async fn run_attachments(
    command: AttachmentCommands,
    base_url_flag: Option<String>,
    client: &Client,
) -> CliResult<()> {
    let mut session = load_session()?;
    session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));

    let response = match command {
        AttachmentCommands::Presign(args) => {
            let (filename, content_type, size_bytes) = resolve_attachment_presign_fields(&args)?;
            send_authed_json(
                client,
                Method::POST,
                &mut session,
                "/attachments/presign",
                Some(json!({
                    "channel_id": args.channel,
                    "filename": filename,
                    "content_type": content_type,
                    "size_bytes": size_bytes,
                })),
                None,
            )
            .await?
        }
        AttachmentCommands::Commit(args) => {
            send_authed_json(
                client,
                Method::POST,
                &mut session,
                "/attachments/commit",
                Some(json!({
                    "upload_id": args.upload_id,
                    "message_id": args.message_id,
                })),
                None,
            )
            .await?
        }
        AttachmentCommands::Get(args) => {
            let path = format!("/attachments/{}", args.attachment_id);
            send_authed_json(client, Method::GET, &mut session, &path, None, None).await?
        }
    };

    save_session(&session)?;
    print_or_ok(response).await
}

fn resolve_attachment_presign_fields(
    args: &AttachmentPresignArgs,
) -> CliResult<(String, String, u64)> {
    let mut filename = args.filename.clone();
    let mut content_type = args.content_type.clone();
    let mut size_bytes = args.size_bytes;

    if let Some(file_path) = &args.file {
        let file_meta = fs::metadata(file_path)?;
        if !file_meta.is_file() {
            return Err(Box::new(cli_error(format!(
                "{} is not a regular file",
                file_path.display()
            ))));
        }

        if filename.is_none() {
            filename = file_path
                .file_name()
                .map(|value| value.to_string_lossy().to_string());
        }
        if size_bytes.is_none() {
            size_bytes = Some(file_meta.len());
        }
        if content_type.is_none() {
            content_type = Some("application/octet-stream".to_string());
        }
    }

    let filename = filename.ok_or_else(|| {
        Box::new(cli_error(
            "missing filename: use --filename or provide --file".to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;
    let content_type = content_type.ok_or_else(|| {
        Box::new(cli_error(
            "missing content type: use --content-type or provide --file".to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;
    let size_bytes = size_bytes.ok_or_else(|| {
        Box::new(cli_error(
            "missing size: use --size-bytes or provide --file".to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    if size_bytes == 0 {
        return Err(Box::new(cli_error(
            "size_bytes must be greater than zero".to_string(),
        )));
    }

    Ok((filename, content_type, size_bytes))
}

async fn run_audit(
    command: AuditCommands,
    base_url_flag: Option<String>,
    client: &Client,
) -> CliResult<()> {
    let mut session = load_session()?;
    session.base_url = resolve_base_url(base_url_flag.as_deref(), Some(&session.base_url));

    let response = match command {
        AuditCommands::List(args) => {
            let query = cursor_limit_query(args.cursor, args.limit);
            send_authed_json(
                client,
                Method::GET,
                &mut session,
                "/audit",
                None,
                Some(query),
            )
            .await?
        }
    };

    save_session(&session)?;
    print_or_ok(response).await
}

fn cursor_limit_query(cursor: Option<String>, limit: Option<usize>) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(cursor) = cursor {
        query.push(("cursor".to_string(), cursor));
    }
    if let Some(limit) = limit {
        query.push(("limit".to_string(), limit.to_string()));
    }
    query
}

async fn send_authed_json(
    client: &Client,
    method: Method,
    session: &mut StoredSession,
    path: &str,
    body: Option<Value>,
    query: Option<Vec<(String, String)>>,
) -> CliResult<reqwest::Response> {
    if session.access_expires_at <= Utc::now().timestamp() {
        refresh_session(client, session).await?;
    }

    let first = send_json(
        client,
        method.clone(),
        &session.base_url,
        path,
        body.clone(),
        query.clone(),
        Some(&session.access_token),
    )
    .await;

    let response = match first {
        Ok(resp) => resp,
        Err(error) => {
            if let Some(status) = extract_status(&*error)
                && status == StatusCode::UNAUTHORIZED
            {
                refresh_session(client, session).await?;
                send_json(
                    client,
                    method,
                    &session.base_url,
                    path,
                    body,
                    query,
                    Some(&session.access_token),
                )
                .await?
            } else {
                return Err(error);
            }
        }
    };

    Ok(response)
}

async fn refresh_session(client: &Client, session: &mut StoredSession) -> CliResult<()> {
    let payload = json!({ "refresh_token": session.refresh_token });
    let response = send_json(
        client,
        Method::POST,
        &session.base_url,
        "/auth/refresh",
        Some(payload),
        None,
        None,
    )
    .await?;

    let tokens: AuthTokensResponse = parse_json(response).await?;
    session.access_token = tokens.access_token;
    session.refresh_token = tokens.refresh_token;
    session.access_expires_at = tokens.access_expires_at;
    session.refresh_expires_at = tokens.refresh_expires_at;
    Ok(())
}

async fn send_json(
    client: &Client,
    method: Method,
    base_url: &str,
    path: &str,
    body: Option<Value>,
    query: Option<Vec<(String, String)>>,
    bearer_token: Option<&str>,
) -> CliResult<reqwest::Response> {
    let mut request = client.request(method, endpoint(base_url, path));
    if let Some(body) = body {
        request = request.json(&body);
    }
    if let Some(query) = query {
        request = request.query(&query);
    }

    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }

    let response = request.send().await?;
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = match serde_json::from_str::<ApiErrorResponse>(&body) {
        Ok(parsed) => format!("{} ({}): {}", status.as_u16(), parsed.error, parsed.message),
        Err(_) => format!("{}: {}", status.as_u16(), body),
    };

    Err(Box::new(StatusedCliError { status, message }))
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}/api/v1{}", normalize_base_url(base_url), path)
}

fn resolve_base_url(flag: Option<&str>, stored: Option<&str>) -> String {
    if let Some(value) = flag {
        return normalize_base_url(value);
    }
    if let Some(value) = env::var("GALYNX_API_BASE_URL").ok().as_deref() {
        return normalize_base_url(value);
    }
    if let Some(value) = stored {
        return normalize_base_url(value);
    }
    normalize_base_url(DEFAULT_BASE_URL)
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn save_session(session: &StoredSession) -> CliResult<()> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_vec_pretty(session)?;
    fs::write(path, payload)?;
    Ok(())
}

fn load_session() -> CliResult<StoredSession> {
    load_session_if_exists()?.ok_or_else(|| {
        Box::new(cli_error(
            "no active session found; run `galynx auth login` first".to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })
}

fn load_session_if_exists() -> CliResult<Option<StoredSession>> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read(path)?;
    let session: StoredSession = serde_json::from_slice(&raw)?;
    Ok(Some(session))
}

fn clear_session_file() -> CliResult<()> {
    let path = credentials_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn credentials_path() -> CliResult<PathBuf> {
    if let Ok(value) = env::var("GALYNX_CREDENTIALS_FILE") {
        let path = PathBuf::from(value);
        if path.is_relative() {
            return Err(Box::new(cli_error(
                "GALYNX_CREDENTIALS_FILE must be an absolute path".to_string(),
            )));
        }
        return Ok(path);
    }

    let home = env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| cli_error("HOME environment variable is not set".to_string()))?;
    Ok(Path::new(&home)
        .join(".config")
        .join("galynx")
        .join("credentials.json"))
}

async fn parse_json<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> CliResult<T> {
    let parsed = response.json::<T>().await?;
    Ok(parsed)
}

async fn print_json(response: reqwest::Response) -> CliResult<()> {
    let body = response.text().await?;
    let value: Value = serde_json::from_str(&body)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

async fn print_or_ok(response: reqwest::Response) -> CliResult<()> {
    if response.status() == StatusCode::NO_CONTENT {
        println!("ok");
        return Ok(());
    }
    print_json(response).await
}

#[derive(Debug)]
struct StatusedCliError {
    status: StatusCode,
    message: String,
}

impl std::fmt::Display for StatusedCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for StatusedCliError {}

fn extract_status(error: &(dyn std::error::Error + 'static)) -> Option<StatusCode> {
    if let Some(statused) = error.downcast_ref::<StatusedCliError>() {
        return Some(statused.status);
    }
    None
}

fn cli_error(message: String) -> CliError {
    CliError { message }
}
