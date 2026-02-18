use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct AuthTokensResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct MeResponse {
    workspace_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct WorkspaceResponse {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct ChannelResponse {
    id: Uuid,
}

struct TestServer {
    child: Child,
    base_url: String,
    ws_url: String,
    owner_email: String,
    owner_password: String,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pick_open_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("failed to bind ephemeral port")
        .local_addr()
        .expect("failed to get local address")
        .port()
}

async fn start_server(tag: &str) -> TestServer {
    let port = pick_open_port();
    let base_url = format!("http://127.0.0.1:{port}");
    let ws_url = format!("ws://127.0.0.1:{port}/api/v1/ws");

    let unique = Uuid::new_v4();
    let owner_email = format!("owner+{tag}+{unique}@galynx.local");
    let owner_password = "ChangeMe123!".to_string();
    let workspace_name = format!("ci-{tag}-{unique}");

    let mongo_uri = std::env::var("TEST_MONGO_URI").unwrap_or_else(|_| {
        "mongodb://root:password@127.0.0.1:27017/?authSource=admin".to_string()
    });
    let redis_url =
        std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let binary = std::env::var("CARGO_BIN_EXE_galynx-api")
        .expect("cargo did not provide CARGO_BIN_EXE_galynx-api");

    let child = Command::new(binary)
        .env("PORT", port.to_string())
        .env("JWT_SECRET", "ci-secret")
        .env("ACCESS_TTL_MINUTES", "15")
        .env("REFRESH_TTL_DAYS", "30")
        .env("PERSISTENCE_BACKEND", "mongo")
        .env("MONGO_URI", mongo_uri)
        .env("REDIS_URL", redis_url)
        .env("BOOTSTRAP_WORKSPACE_NAME", workspace_name)
        .env("BOOTSTRAP_EMAIL", &owner_email)
        .env("BOOTSTRAP_PASSWORD", &owner_password)
        .env("METRICS_ENABLED", "true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn galynx-api process");

    let mut server = TestServer {
        child,
        base_url,
        ws_url,
        owner_email,
        owner_password,
    };

    wait_until_ready(&mut server).await;
    server
}

async fn wait_until_ready(server: &mut TestServer) {
    let client = Client::new();
    for _ in 0..120 {
        if let Some(status) = server
            .child
            .try_wait()
            .expect("failed to poll child process")
        {
            panic!("galynx-api exited before ready: {status}");
        }

        let health = format!("{}/api/v1/health", server.base_url);
        if let Ok(response) = client.get(&health).send().await
            && response.status() == StatusCode::OK
        {
            return;
        }

        sleep(Duration::from_millis(100)).await;
    }

    panic!("galynx-api did not become ready in time");
}

async fn login(
    client: &Client,
    base_url: &str,
    email: &str,
    password: &str,
    workspace_id: Option<Uuid>,
) -> AuthTokensResponse {
    client
        .post(format!("{base_url}/api/v1/auth/login"))
        .json(&json!({
            "email": email,
            "password": password,
            "workspace_id": workspace_id,
        }))
        .send()
        .await
        .expect("login request failed")
        .error_for_status()
        .expect("login failed")
        .json::<AuthTokensResponse>()
        .await
        .expect("failed to decode login response")
}

#[tokio::test]
#[ignore = "CI integration suite; run with -- --ignored"]
async fn integration_http_flow() {
    let server = start_server("http").await;
    let client = Client::new();

    let tokens = login(
        &client,
        &server.base_url,
        &server.owner_email,
        &server.owner_password,
        None,
    )
    .await;

    let me = client
        .get(format!("{}/api/v1/me", server.base_url))
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .expect("/me request failed")
        .error_for_status()
        .expect("/me failed")
        .json::<MeResponse>()
        .await
        .expect("failed to decode me response");

    let channel = client
        .post(format!("{}/api/v1/channels", server.base_url))
        .bearer_auth(&tokens.access_token)
        .json(&json!({
            "name": format!("ci-http-{}", Uuid::new_v4().simple()),
            "is_private": false,
        }))
        .send()
        .await
        .expect("create channel request failed")
        .error_for_status()
        .expect("create channel failed")
        .json::<ChannelResponse>()
        .await
        .expect("failed to decode channel response");

    client
        .post(format!(
            "{}/api/v1/channels/{}/messages",
            server.base_url, channel.id
        ))
        .bearer_auth(&tokens.access_token)
        .json(&json!({ "body_md": "hello from integration" }))
        .send()
        .await
        .expect("create message request failed")
        .error_for_status()
        .expect("create message failed");

    let list_messages = client
        .get(format!(
            "{}/api/v1/channels/{}/messages",
            server.base_url, channel.id
        ))
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .expect("list messages request failed")
        .error_for_status()
        .expect("list messages failed")
        .json::<Value>()
        .await
        .expect("failed to decode message page");

    assert!(
        list_messages["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "expected at least one message"
    );

    let audit = client
        .get(format!("{}/api/v1/audit", server.base_url))
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .expect("audit request failed")
        .error_for_status()
        .expect("audit failed")
        .json::<Value>()
        .await
        .expect("failed to decode audit response");

    assert!(
        audit["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "expected non-empty audit log in workspace {}",
        me.workspace_id
    );
}

#[tokio::test]
#[ignore = "CI websocket suite; run with -- --ignored"]
async fn ws_command_flow() {
    let server = start_server("ws").await;
    let client = Client::new();

    let tokens = login(
        &client,
        &server.base_url,
        &server.owner_email,
        &server.owner_password,
        None,
    )
    .await;

    let channel = client
        .post(format!("{}/api/v1/channels", server.base_url))
        .bearer_auth(&tokens.access_token)
        .json(&json!({
            "name": format!("ci-ws-{}", Uuid::new_v4().simple()),
            "is_private": false,
        }))
        .send()
        .await
        .expect("create channel request failed")
        .error_for_status()
        .expect("create channel failed")
        .json::<ChannelResponse>()
        .await
        .expect("failed to decode channel response");

    let mut request = server
        .ws_url
        .as_str()
        .into_client_request()
        .expect("failed to build websocket request");
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", tokens.access_token)
            .parse()
            .expect("invalid auth header"),
    );

    let (mut ws, _response) = connect_async(request)
        .await
        .expect("failed to connect websocket");

    let welcome = ws
        .next()
        .await
        .expect("expected websocket frame")
        .expect("websocket read error");
    let welcome_text = welcome.into_text().expect("welcome frame must be text");
    let welcome_json: Value =
        serde_json::from_str(&welcome_text).expect("failed to decode welcome event");
    assert_eq!(welcome_json["event_type"], "WELCOME");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({
            "command": "SEND_MESSAGE",
            "client_msg_id": format!("ci-{}", Uuid::new_v4().simple()),
            "payload": {
                "channel_id": channel.id,
                "body_md": "hello from websocket",
            }
        })
        .to_string(),
    ))
    .await
    .expect("failed to send websocket command");

    let mut got_ack = false;
    for _ in 0..8 {
        let frame = ws
            .next()
            .await
            .expect("expected websocket response")
            .expect("websocket read failed");
        if let tokio_tungstenite::tungstenite::Message::Text(text) = frame {
            let event: Value = serde_json::from_str(&text).expect("invalid websocket json");
            if event["event_type"] == "ACK" && event["payload"]["command"] == "SEND_MESSAGE" {
                got_ack = true;
                break;
            }
        }
    }

    assert!(got_ack, "expected SEND_MESSAGE ack over websocket");
}

#[tokio::test]
#[ignore = "CI e2e smoke suite; run with -- --ignored"]
async fn e2e_smoke_flow() {
    let server = start_server("e2e").await;
    let client = Client::new();

    let owner_tokens = login(
        &client,
        &server.base_url,
        &server.owner_email,
        &server.owner_password,
        None,
    )
    .await;

    let workspace = client
        .post(format!("{}/api/v1/workspaces", server.base_url))
        .bearer_auth(&owner_tokens.access_token)
        .json(&json!({ "name": format!("e2e-{}", Uuid::new_v4().simple()) }))
        .send()
        .await
        .expect("create workspace request failed")
        .error_for_status()
        .expect("create workspace failed")
        .json::<WorkspaceResponse>()
        .await
        .expect("failed to decode workspace response");

    let member_email = format!("member+{}@galynx.local", Uuid::new_v4().simple());
    let member_password = "ChangeMe123!";

    client
        .post(format!(
            "{}/api/v1/workspaces/{}/members",
            server.base_url, workspace.id
        ))
        .bearer_auth(&owner_tokens.access_token)
        .json(&json!({
            "email": member_email,
            "name": "CI Member",
            "password": member_password,
            "role": "member",
        }))
        .send()
        .await
        .expect("onboard member request failed")
        .error_for_status()
        .expect("onboard member failed");

    let owner_workspace_tokens = login(
        &client,
        &server.base_url,
        &server.owner_email,
        &server.owner_password,
        Some(workspace.id),
    )
    .await;

    let channel = client
        .post(format!("{}/api/v1/channels", server.base_url))
        .bearer_auth(&owner_workspace_tokens.access_token)
        .json(&json!({
            "name": format!("e2e-ch-{}", Uuid::new_v4().simple()),
            "is_private": false,
        }))
        .send()
        .await
        .expect("create channel request failed")
        .error_for_status()
        .expect("create channel failed")
        .json::<ChannelResponse>()
        .await
        .expect("failed to decode channel response");

    let member_tokens = login(
        &client,
        &server.base_url,
        &member_email,
        member_password,
        Some(workspace.id),
    )
    .await;

    client
        .post(format!(
            "{}/api/v1/channels/{}/messages",
            server.base_url, channel.id
        ))
        .bearer_auth(&member_tokens.access_token)
        .json(&json!({ "body_md": "hello from e2e member" }))
        .send()
        .await
        .expect("member send message request failed")
        .error_for_status()
        .expect("member send message failed");

    let audit = client
        .get(format!("{}/api/v1/audit", server.base_url))
        .bearer_auth(&owner_workspace_tokens.access_token)
        .send()
        .await
        .expect("audit request failed")
        .error_for_status()
        .expect("audit failed")
        .json::<Value>()
        .await
        .expect("failed to decode audit response");

    assert!(
        audit["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "expected non-empty audit log in e2e workspace"
    );
}
