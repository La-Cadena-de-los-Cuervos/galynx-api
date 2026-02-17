# Galynx API - Documentacion para Frontend y CLI

Este documento describe la API actual de `galynx-api` para integracion de frontend y para base del CLI.

## 1) Base URL y versionado

- Base URL local por defecto: `http://localhost:3000`
- Prefijo versionado: `/api/v1`
- OpenAPI JSON: `GET /api/v1/openapi.json`

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

## 5) Auth endpoints

### `POST /api/v1/auth/login`

Body:

```json
{
  "email": "owner@galynx.local",
  "password": "ChangeMe123!"
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

## 6) Channels

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

### `DELETE /api/v1/channels/:id`

Respuesta `204`.

## 7) Messages

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

## 8) Threads

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

## 9) Attachments

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

## 10) Audit

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

## 11) WebSocket realtime

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

## 12) Paginacion (messages, thread replies, audit)

Convencion de cursor:

- String con formato `<created_at>:<id_u128>`

Regla:

- Si `next_cursor` es `null`, no hay mas pagina.
- Para siguiente pagina, enviar `cursor=next_cursor`.

## 13) Rate limits actuales (en memoria)

- Auth (`/auth/login`, `/auth/refresh`, `/auth/logout`): `30 req/min` por combinacion IP/email.
- WebSocket connect (`/ws`): `12 conexiones/min` por IP+user.
- WebSocket commands: `600 comandos/min` por user.

Error cuando excede: HTTP `429` o evento WS `ERROR` con `status: 429`.

## 14) Mapeo sugerido para CLI

Comandos sugeridos sobre la API actual:

- `galynx auth login`
- `galynx auth me`
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

## 15) Notas para frontend

- Implementar interceptor de `401` + refresh atomico (evitar multiples refresh paralelos).
- Tratar `429` con backoff exponencial corto.
- Para mensajes/threads/audit usar paginacion por cursor, no por offset.
- Preferir WS para sincronizacion en vivo y HTTP para carga inicial y fallback.
