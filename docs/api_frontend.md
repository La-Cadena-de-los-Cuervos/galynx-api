# Galynx API - Documentacion Frontend

Este documento esta enfocado en integracion frontend (web/mobile) con `galynx-api`.

## Base URL y version

- Base URL local: `http://localhost:3000`
- Prefijo: `/api/v1`
- OpenAPI: `GET /api/v1/openapi.json`

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
- `S3_ENDPOINT` (opcional, endpoint interno S3/RustFS para el API)
- `S3_PUBLIC_ENDPOINT` (opcional, endpoint publico para URLs prefirmadas)
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
export S3_ENDPOINT='http://rustfs:9000'
export S3_PUBLIC_ENDPOINT='http://localhost:9000'
export S3_ACCESS_KEY_ID='rustfsadmin'
export S3_SECRET_ACCESS_KEY='rustfsadmin'
export S3_FORCE_PATH_STYLE='true'
cargo run
```

## Autenticacion

### Header

```http
Authorization: Bearer <access_token>
```

### Flujo sugerido

1. Login con `POST /api/v1/auth/login`.
2. Guardar `access_token` y `refresh_token`.
3. Si llega `401`, refrescar con `POST /api/v1/auth/refresh`.
4. Reintentar request original una vez.
5. Logout con `POST /api/v1/auth/logout`.

### Usuario bootstrap local (default)

- Email: `owner@galynx.local`
- Password: `ChangeMe123!`

## Formato de error

```json
{
  "error": "bad_request",
  "message": "message body is required"
}
```

Valores actuales de `error`:

- `unauthorized`
- `bad_request`
- `too_many_requests`
- `not_found`
- `internal_error`

## Endpoints

## Sistema

### `GET /api/v1/health`

```json
{ "status": "ok" }
```

### `GET /api/v1/ready`

```json
{ "status": "ready" }
```

### `GET /api/v1/metrics`

Formato Prometheus text/plain para scraping de métricas HTTP del API.

## Auth

### `POST /api/v1/auth/login`

Request:

```json
{
  "email": "owner@galynx.local",
  "password": "ChangeMe123!",
  "workspace_id": "uuid-opcional"
}
```

Response `200`:

```json
{
  "access_token": "...",
  "refresh_token": "...",
  "access_expires_at": 1739899200,
  "refresh_expires_at": 1742404800
}
```

### `POST /api/v1/auth/refresh`

Request:

```json
{ "refresh_token": "..." }
```

Response `200`: mismo esquema de login.

### `POST /api/v1/auth/logout`

Request:

```json
{ "refresh_token": "..." }
```

Response: `204`.

### `GET /api/v1/me`

Response `200`:

```json
{
  "id": "uuid",
  "email": "owner@galynx.local",
  "name": "Owner",
  "workspace_id": "uuid",
  "role": "owner"
}
```

## Workspaces

### `GET /api/v1/workspaces`

Lista los workspaces del usuario autenticado.

### `POST /api/v1/workspaces`

Crea un workspace y agrega al usuario actual como `owner`.

Request:

```json
{
  "name": "Mi Workspace"
}
```

### `GET /api/v1/workspaces/:id/members`

Requiere rol `owner` o `admin` del workspace del token.

### `POST /api/v1/workspaces/:id/members`

Onboarding de usuarios al workspace (nuevo o existente).
Requiere rol `owner` o `admin`.

Request:

```json
{
  "email": "nuevo@galynx.local",
  "name": "Nuevo Usuario",
  "password": "ChangeMe123!",
  "role": "member"
}
```

Notas:

- Si el email ya existe, `name/password` son opcionales y se agrega/actualiza membresía.
- `role` soporta `admin|member`.
- `owner` no se permite por API.

## Users

### `GET /api/v1/users`

Requiere rol `owner` o `admin`.

Response `200`:

```json
[
  {
    "id": "uuid",
    "email": "member@galynx.local",
    "name": "Member User",
    "workspace_id": "uuid",
    "role": "member"
  }
]
```

### `POST /api/v1/users`

Requiere rol `owner` o `admin`.

Request:

```json
{
  "email": "member@galynx.local",
  "name": "Member User",
  "password": "ChangeMe123!",
  "role": "member"
}
```

Response: `201`.

Notas:

- `role` soporta `admin` y `member`.
- Alta de `owner` por API no está permitida.

## Channels

### `GET /api/v1/channels`

Response `200`:

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

Requiere rol `owner` o `admin`.

Request:

```json
{
  "name": "engineering",
  "is_private": false
}
```

Response: `201`.

Nota de acceso:

- Si `is_private=true`, solo miembros explícitos del canal pueden leer/publicar.
- `owner` y `admin` pueden acceder aunque no estén en `channel_members`.

### `DELETE /api/v1/channels/:id`

Requiere rol `owner` o `admin`.

Response: `204`.

### `GET /api/v1/channels/:id/members`

Requiere rol `owner` o `admin`.

Response `200`:

```json
[
  { "user_id": "uuid" }
]
```

### `POST /api/v1/channels/:id/members`

Requiere rol `owner` o `admin`.

Request:

```json
{ "user_id": "uuid" }
```

Response: `204`.

### `DELETE /api/v1/channels/:id/members/:user_id`

Requiere rol `owner` o `admin`.

Response: `204`.

## Messages

### `GET /api/v1/channels/:id/messages?limit=50&cursor=<cursor>`

- `limit` efectivo: `1..100`, default `50`.
- `cursor`: opcional.

Response `200`:

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

Request:

```json
{ "body_md": "Hola equipo" }
```

Response: `201`.

### `PATCH /api/v1/messages/:id`

Solo autor del mensaje.

Request:

```json
{ "body_md": "Mensaje editado" }
```

Response: `200`.

### `DELETE /api/v1/messages/:id`

Puede borrar autor, `owner` o `admin`.

Response: `204`.

## Threads

### `GET /api/v1/threads/:root_id`

Response `200`:

```json
{
  "root_message": {
    "id": "uuid",
    "workspace_id": "uuid",
    "channel_id": "uuid",
    "sender_id": "uuid",
    "body_md": "Root",
    "thread_root_id": null,
    "created_at": 1739801000000,
    "edited_at": null,
    "deleted_at": null
  },
  "reply_count": 2,
  "last_reply_at": 1739802000000,
  "participants": ["uuid", "uuid"]
}
```

### `GET /api/v1/threads/:root_id/replies?limit=50&cursor=<cursor>`

Response: `200` (`MessageListResponse`).

### `POST /api/v1/threads/:root_id/replies`

Request:

```json
{ "body_md": "Respuesta al hilo" }
```

Response: `201`.

## Attachments

### Limites y TTL

- Max size: `100MB`
- Presign TTL: `900s`
- Download URL TTL: `600s`

### Flujo

1. `POST /api/v1/attachments/presign`
2. Subir binario a `upload_url`
3. `POST /api/v1/attachments/commit`
4. `GET /api/v1/attachments/:id` (opcional)

### `POST /api/v1/attachments/presign`

Request:

```json
{
  "channel_id": "uuid",
  "filename": "spec.pdf",
  "content_type": "application/pdf",
  "size_bytes": 245760
}
```

Response `200`:

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

Request:

```json
{
  "upload_id": "uuid",
  "message_id": "uuid"
}
```

`message_id` puede ser `null`.

Response: `200` (`AttachmentResponse`).

### `GET /api/v1/attachments/:id`

Response: `200` (`AttachmentGetResponse`) con `download_url` temporal.

## Audit

### `GET /api/v1/audit?limit=50&cursor=<cursor>`

Solo `owner` o `admin`.

Response `200`:

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

## WebSocket realtime

### Conexion

- Endpoint: `GET /api/v1/ws`
- Bearer token en handshake.

Evento inicial:

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

### Envelope estandar

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

### Comandos cliente soportados

- `SEND_MESSAGE`
- `EDIT_MESSAGE`
- `DELETE_MESSAGE`
- `FETCH_MORE`
- `FETCH_THREAD`
- `ADD_REACTION`
- `REMOVE_REACTION`

Ejemplo comando:

```json
{
  "command": "SEND_MESSAGE",
  "client_msg_id": "client-123",
  "payload": {
    "channel_id": "uuid",
    "body_md": "hola"
  }
}
```

Respuesta ACK:

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

Nota de idempotencia:

- En `SEND_MESSAGE`, si reutilizas el mismo `client_msg_id` para el mismo `channel_id` y usuario, la API responde el mismo `message_id` (sin crear duplicado).
- En ese caso el ACK puede incluir `"deduped": true` en `payload.result`.
- La misma estrategia de deduplicación por `client_msg_id` aplica también a `EDIT_MESSAGE`, `DELETE_MESSAGE`, `ADD_REACTION` y `REMOVE_REACTION`.

Error WS:

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

Eventos de negocio broadcast:

- `CHANNEL_CREATED`
- `CHANNEL_DELETED`
- `MESSAGE_CREATED`
- `MESSAGE_UPDATED`
- `MESSAGE_DELETED`
- `THREAD_UPDATED`
- `REACTION_UPDATED`

## Paginacion

Formato cursor:

- `<created_at>:<id_u128>`

Regla:

- Si `next_cursor` es `null`, no hay mas resultados.

## Rate limits actuales

- Auth: `30 req/min`
- WS connect: `12 req/min`
- WS command: `600 req/min`

## Recomendaciones frontend

- Interceptor de `401` con refresh atomico.
- Retry suave con backoff para `429`.
- Carga inicial por HTTP + sincronizacion en vivo por WS.
