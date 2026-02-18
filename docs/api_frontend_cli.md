# Galynx API - Documentacion para Frontend y CLI

Este documento describe la API actual de `galynx-api` para integracion de frontend y para base del CLI.

## 1) Base URL y versionado

- Base URL local por defecto: `http://localhost:3000`
- Prefijo versionado: `/api/v1`
- OpenAPI JSON: `GET /api/v1/openapi.json`

## Variables de entorno del API

- `PORT` (default: `3000`)
- `JWT_SECRET` (default: `dev-only-change-me-in-prod`)
- `ACCESS_TTL_MINUTES` (default: `15`)
- `REFRESH_TTL_DAYS` (default: `30`)
- `BOOTSTRAP_WORKSPACE_NAME` (default: `Galynx`)
- `BOOTSTRAP_EMAIL` (default: `owner@galynx.local`)
- `BOOTSTRAP_PASSWORD` (default: `ChangeMe123!`)
- `PERSISTENCE_BACKEND` (`memory` o `mongo`, default: `memory`)
- `MONGO_URI` (requerido cuando `PERSISTENCE_BACKEND=mongo`)
- `REDIS_URL` (opcional, habilita pub/sub realtime entre réplicas)
- `METRICS_ENABLED` (default: `true`, expone `/api/v1/metrics`)
- `OTEL_EXPORTER_OTLP_ENDPOINT` (opcional, habilita trazas OTLP gRPC)
- `OTEL_SERVICE_NAME` (default: `galynx-api`)
- `OTEL_SAMPLE_RATIO` (default: `1.0`)
- `S3_BUCKET` (opcional, habilita presign real de adjuntos)
- `S3_REGION` (default: `us-east-1`)
- `S3_ENDPOINT` (opcional, para RustFS/S3 compatible)
- `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` (opcionales)
- `S3_FORCE_PATH_STYLE` (default: `true`, recomendado con RustFS)

Ejemplo para Mongo local:

```bash
export PERSISTENCE_BACKEND=mongo
export MONGO_URI='mongodb://root:password@localhost:27017/?authSource=admin'
export REDIS_URL='redis://localhost:6379'
export METRICS_ENABLED='true'
export OTEL_EXPORTER_OTLP_ENDPOINT='http://localhost:4317'
export OTEL_SERVICE_NAME='galynx-api'
export OTEL_SAMPLE_RATIO='1.0'
export S3_BUCKET='galynx-attachments'
export S3_REGION='us-east-1'
export S3_ENDPOINT='http://localhost:9000'
export S3_ACCESS_KEY_ID='rustfsadmin'
export S3_SECRET_ACCESS_KEY='rustfsadmin'
export S3_FORCE_PATH_STYLE='true'
cargo run
```

## 2) Autenticacion

### Esquema

- La API usa JWT bearer para endpoints protegidos.
- Header requerido:

```http
Authorization: Bearer <access_token>
```

### Flujo recomendado

1. `POST /api/v1/auth/login` con email/password.
2. Guardar `access_token` y `refresh_token`.
3. En `401` por token expirado, ejecutar `POST /api/v1/auth/refresh`.
4. Reintentar request original con nuevo `access_token`.
5. En logout, llamar `POST /api/v1/auth/logout` con refresh token.

### Usuario bootstrap (entorno local)

Si no se configuran variables de entorno:

- Email: `owner@galynx.local`
- Password: `ChangeMe123!`

## 3) Formato de errores

Todos los errores siguen este JSON:

```json
{
  "error": "bad_request",
  "message": "message body is required"
}
```

Codigos `error` usados actualmente:

- `unauthorized`
- `bad_request`
- `too_many_requests`
- `not_found`
- `internal_error`

## 4) Health y readiness

### `GET /api/v1/health`

Respuesta `200`:

```json
{ "status": "ok" }
```

### `GET /api/v1/ready`

Respuesta `200`:

```json
{ "status": "ready" }
```

### `GET /api/v1/metrics`

Expone métricas en formato Prometheus text/plain.

## 5) Auth endpoints

### `POST /api/v1/auth/login`

Body:

```json
{
  "email": "owner@galynx.local",
  "password": "ChangeMe123!",
  "workspace_id": "uuid-opcional"
}
```

Respuesta `200`:

```json
{
  "access_token": "...",
  "refresh_token": "...",
  "access_expires_at": 1739899200,
  "refresh_expires_at": 1742404800
}
```

### `POST /api/v1/auth/refresh`

Body:

```json
{
  "refresh_token": "..."
}
```

Respuesta `200`: mismo formato de login.

### `POST /api/v1/auth/logout`

Requiere bearer token.

Body:

```json
{
  "refresh_token": "..."
}
```

Respuesta `204` sin body.

### `GET /api/v1/me`

Requiere bearer token.

Respuesta `200`:

```json
{
  "id": "uuid",
  "email": "owner@galynx.local",
  "name": "Owner",
  "workspace_id": "uuid",
  "role": "owner"
}
```

## 6) Users

### `GET /api/v1/users`

Requiere rol `owner` o `admin`.

### `POST /api/v1/users`

Body:

```json
{
  "email": "member@galynx.local",
  "name": "Member User",
  "password": "ChangeMe123!",
  "role": "member"
}
```

Requiere rol `owner` o `admin`. Respuesta `201`.
`role` soporta `admin|member`.

## 6.1) Workspaces

### `GET /api/v1/workspaces`

Lista workspaces del usuario autenticado.

### `POST /api/v1/workspaces`

Crea workspace y agrega al usuario actual como `owner`.

### `GET /api/v1/workspaces/:id/members`

Lista miembros del workspace (requiere `owner/admin`).

### `POST /api/v1/workspaces/:id/members`

Onboarding de miembro al workspace (nuevo o existente). Requiere `owner/admin`.

## 7) Channels

## Roles

- `owner` y `admin`: pueden crear/eliminar canales.
- `member`: no puede administrar canales.

### `GET /api/v1/channels`

Lista canales del workspace del token.

Respuesta `200`:

```json
[
  {
    "id": "uuid",
    "workspace_id": "uuid",
    "name": "general",
    "is_private": false,
    "created_by": "uuid",
    "created_at": 1739800000000
  }
]
```

### `POST /api/v1/channels`

Body:

```json
{
  "name": "engineering",
  "is_private": false
}
```

Respuesta `201`: `ChannelResponse`.

Regla de acceso para canales privados:

- Si `is_private=true`, solo miembros explícitos del canal pueden leer/publicar.
- `owner` y `admin` tienen bypass de membresía.

### `DELETE /api/v1/channels/:id`

Respuesta `204`.

### `GET /api/v1/channels/:id/members`

Requiere rol `owner` o `admin`.

### `POST /api/v1/channels/:id/members`

Body:

```json
{ "user_id": "uuid" }
```

Requiere rol `owner` o `admin`. Respuesta `204`.

### `DELETE /api/v1/channels/:id/members/:user_id`

Requiere rol `owner` o `admin`. Respuesta `204`.

## 8) Messages

### `GET /api/v1/channels/:id/messages?limit=50&cursor=<cursor>`

- `limit`: opcional, rango efectivo `1..100` (default `50`).
- `cursor`: opcional.

Respuesta `200`:

```json
{
  "items": [
    {
      "id": "uuid",
      "workspace_id": "uuid",
      "channel_id": "uuid",
      "sender_id": "uuid",
      "body_md": "Hola equipo",
      "thread_root_id": null,
      "created_at": 1739801000000,
      "edited_at": null,
      "deleted_at": null
    }
  ],
  "next_cursor": "1739801000000:123456789"
}
```

### `POST /api/v1/channels/:id/messages`

Body:

```json
{
  "body_md": "Hola equipo"
}
```

Respuesta `201`: `MessageResponse`.

### `PATCH /api/v1/messages/:id`

Body:

```json
{
  "body_md": "Mensaje editado"
}
```

Respuesta `200`: `MessageResponse`.

Restriccion:

- Solo el autor del mensaje puede editar.

### `DELETE /api/v1/messages/:id`

Respuesta `204`.

Restriccion:

- Puede borrar: autor del mensaje, `owner` o `admin`.

## 9) Threads

### `GET /api/v1/threads/:root_id`

Resumen de hilo.

Respuesta `200`:

```json
{
  "root_message": { "id": "uuid", "workspace_id": "uuid", "channel_id": "uuid", "sender_id": "uuid", "body_md": "Root", "thread_root_id": null, "created_at": 1739801000000, "edited_at": null, "deleted_at": null },
  "reply_count": 2,
  "last_reply_at": 1739802000000,
  "participants": ["uuid", "uuid"]
}
```

### `GET /api/v1/threads/:root_id/replies?limit=50&cursor=<cursor>`

Respuesta `200`: `MessageListResponse`.

### `POST /api/v1/threads/:root_id/replies`

Body:

```json
{
  "body_md": "Respuesta al hilo"
}
```

Respuesta `201`: `MessageResponse` con `thread_root_id` apuntando al root.

## 10) Attachments

## Limites y TTL

- Tamano maximo: `100MB`.
- Presign expira en `900s` (15 min).
- Download URL expira en `600s` (10 min).

### Flujo recomendado

1. `POST /api/v1/attachments/presign`
2. Subir archivo binario al `upload_url` retornado.
3. `POST /api/v1/attachments/commit`
4. (Opcional) `GET /api/v1/attachments/:id` para URL de descarga temporal.

### `POST /api/v1/attachments/presign`

Body:

```json
{
  "channel_id": "uuid",
  "filename": "spec.pdf",
  "content_type": "application/pdf",
  "size_bytes": 245760
}
```

Respuesta `200`:

```json
{
  "upload_id": "uuid",
  "upload_url": "https://storage.galynx.local/upload/<upload_id>",
  "bucket": "galynx-attachments",
  "key": "workspace/<ws>/channel/<ch>/uploads/<id>-spec.pdf",
  "expires_at": 1739803000
}
```

### `POST /api/v1/attachments/commit`

Body:

```json
{
  "upload_id": "uuid",
  "message_id": "uuid"
}
```

`message_id` puede ser `null`.

Respuesta `200`:

```json
{
  "id": "uuid",
  "workspace_id": "uuid",
  "channel_id": "uuid",
  "message_id": "uuid",
  "uploader_id": "uuid",
  "filename": "spec.pdf",
  "content_type": "application/pdf",
  "size_bytes": 245760,
  "storage_bucket": "galynx-attachments",
  "storage_key": "workspace/...",
  "storage_region": "us-east-1",
  "created_at": 1739802100
}
```

### `GET /api/v1/attachments/:id`

Respuesta `200`:

```json
{
  "attachment": { "id": "uuid", "workspace_id": "uuid", "channel_id": "uuid", "message_id": "uuid", "uploader_id": "uuid", "filename": "spec.pdf", "content_type": "application/pdf", "size_bytes": 245760, "storage_bucket": "galynx-attachments", "storage_key": "workspace/...", "storage_region": "us-east-1", "created_at": 1739802100 },
  "download_url": "https://storage.galynx.local/download/galynx-attachments/<id>?exp=1739802700",
  "expires_at": 1739802700
}
```

## 11) Audit

### `GET /api/v1/audit?limit=50&cursor=<cursor>`

Restriccion:

- Solo `owner` y `admin`.

Respuesta `200`:

```json
{
  "items": [
    {
      "id": "uuid",
      "workspace_id": "uuid",
      "actor_id": "uuid",
      "action": "MESSAGE_CREATED",
      "target_type": "message",
      "target_id": "uuid",
      "metadata": { "channel_id": "uuid" },
      "created_at": 1739802200000
    }
  ],
  "next_cursor": "1739802200000:123456789"
}
```

## 12) WebSocket realtime

### Conexion

- Endpoint: `GET /api/v1/ws`
- Requiere `Authorization: Bearer <access_token>` en handshake.

Evento inicial al conectar:

```json
{
  "event_type": "WELCOME",
  "workspace_id": "uuid",
  "channel_id": null,
  "correlation_id": null,
  "server_ts": 1739800000000,
  "payload": {
    "user_id": "uuid",
    "role": "owner"
  }
}
```

### Envelope de evento

```json
{
  "event_type": "MESSAGE_CREATED",
  "workspace_id": "uuid",
  "channel_id": "uuid",
  "correlation_id": "client-123",
  "server_ts": 1739800000000,
  "payload": {}
}
```

### Envelope de comando cliente

```json
{
  "command": "SEND_MESSAGE",
  "client_msg_id": "client-123",
  "payload": {}
}
```

### Comandos soportados

- `SEND_MESSAGE` payload:

```json
{ "channel_id": "uuid", "body_md": "hola" }
```

- `EDIT_MESSAGE` payload:

```json
{ "message_id": "uuid", "body_md": "editado" }
```

- `DELETE_MESSAGE` payload:

```json
{ "message_id": "uuid" }
```

- `FETCH_MORE` payload:

```json
{ "channel_id": "uuid", "cursor": null, "limit": 50 }
```

- `FETCH_THREAD` payload:

```json
{ "root_id": "uuid", "cursor": null, "limit": 50 }
```

- `ADD_REACTION` payload:

```json
{ "message_id": "uuid", "emoji": ":thumbsup:" }
```

- `REMOVE_REACTION` payload:

```json
{ "message_id": "uuid", "emoji": ":thumbsup:" }
```

### ACK de comandos

Respuesta tipo `ACK`:

```json
{
  "event_type": "ACK",
  "workspace_id": null,
  "channel_id": null,
  "correlation_id": "client-123",
  "server_ts": 1739800000000,
  "payload": {
    "command": "SEND_MESSAGE",
    "result": { "message_id": "uuid" }
  }
}
```

Para `SEND_MESSAGE`, la API aplica idempotencia por `(workspace_id, user_id, channel_id, client_msg_id)`.
Si reenvias el mismo `client_msg_id`, el ACK devuelve el mismo `message_id` y puede incluir `"deduped": true`.
Tambien se aplica deduplicación con `client_msg_id` en `EDIT_MESSAGE`, `DELETE_MESSAGE`, `ADD_REACTION` y `REMOVE_REACTION`.

### Errores WS

La API envia evento `ERROR`:

```json
{
  "event_type": "ERROR",
  "server_ts": 1739800000000,
  "payload": {
    "status": 400,
    "error": "invalid SEND_MESSAGE payload"
  }
}
```

### Eventos de negocio que pueden llegar

- `CHANNEL_CREATED`
- `CHANNEL_DELETED`
- `MESSAGE_CREATED`
- `MESSAGE_UPDATED`
- `MESSAGE_DELETED`
- `THREAD_UPDATED`
- `REACTION_UPDATED`

## 13) Paginacion (messages, thread replies, audit)

Convencion de cursor:

- String con formato `<created_at>:<id_u128>`

Regla:

- Si `next_cursor` es `null`, no hay mas pagina.
- Para siguiente pagina, enviar `cursor=next_cursor`.

## 14) Rate limits actuales (en memoria)

- Auth (`/auth/login`, `/auth/refresh`, `/auth/logout`): `30 req/min` por combinacion IP/email.
- WebSocket connect (`/ws`): `12 conexiones/min` por IP+user.
- WebSocket commands: `600 comandos/min` por user.

Error cuando excede: HTTP `429` o evento WS `ERROR` con `status: 429`.

## 15) Mapeo sugerido para CLI

Comandos sugeridos sobre la API actual:

- `galynx auth login`
- `galynx auth login --workspace <workspace_id>`
- `galynx auth me`
- `galynx workspaces list`
- `galynx workspaces create --name <name>`
- `galynx workspaces members <workspace_id>`
- `galynx workspaces onboard <workspace_id> --email <email> --role <admin|member> [--name <name>] [--password <password>]`
- `galynx users list`
- `galynx users create --email <email> --name <name> --password <password> --role <admin|member>`
- `galynx channels list`
- `galynx channels create --name <name> [--private]`
- `galynx channels delete <channel_id>`
- `galynx messages list --channel <id> [--cursor ...] [--limit ...]`
- `galynx messages send --channel <id> --body "..."`
- `galynx messages edit <message_id> --body "..."`
- `galynx messages delete <message_id>`
- `galynx threads get <root_id>`
- `galynx threads replies <root_id> [--cursor ...] [--limit ...]`
- `galynx threads reply <root_id> --body "..."`
- `galynx attachments presign --channel <id> --file <path>`
- `galynx attachments commit --upload-id <id> [--message-id <id>]`
- `galynx attachments get <attachment_id>`
- `galynx audit list [--cursor ...] [--limit ...]`

## 16) Notas para frontend

- Implementar interceptor de `401` + refresh atomico (evitar multiples refresh paralelos).
- Tratar `429` con backoff exponencial corto.
- Para mensajes/threads/audit usar paginacion por cursor, no por offset.
- Preferir WS para sincronizacion en vivo y HTTP para carga inicial y fallback.
