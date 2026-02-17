# Galynx API - Documentacion Frontend

Este documento esta enfocado en integracion frontend (web/mobile) con `galynx-api`.

## Base URL y version

- Base URL local: `http://localhost:3000`
- Prefijo: `/api/v1`
- OpenAPI: `GET /api/v1/openapi.json`

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

## Auth

### `POST /api/v1/auth/login`

Request:

```json
{
  "email": "owner@galynx.local",
  "password": "ChangeMe123!"
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

### `DELETE /api/v1/channels/:id`

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
