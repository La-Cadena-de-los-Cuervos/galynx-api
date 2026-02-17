# Galynx API - Guia para CLI

Este documento define como consumir `galynx-api` desde un CLI.

## Base

- Base URL local: `http://localhost:3000`
- Prefijo: `/api/v1`

## Autenticacion en CLI

## Flujo recomendado

1. `galynx auth login --email ... --password ...`
2. Guardar tokens en archivo local seguro (ej. `~/.config/galynx/credentials.json`).
3. Antes de cada comando, validar expiracion de access token.
4. Si expiro, intentar `auth refresh` automaticamente.
5. Si refresh falla (`401`), pedir login nuevamente.

## Endpoints usados por el CLI

### Auth

- `POST /api/v1/auth/login`
- `POST /api/v1/auth/refresh`
- `POST /api/v1/auth/logout`
- `GET /api/v1/me`

### Channels

- `GET /api/v1/channels`
- `POST /api/v1/channels`
- `DELETE /api/v1/channels/:id`

### Messages

- `GET /api/v1/channels/:id/messages`
- `POST /api/v1/channels/:id/messages`
- `PATCH /api/v1/messages/:id`
- `DELETE /api/v1/messages/:id`

### Threads

- `GET /api/v1/threads/:root_id`
- `GET /api/v1/threads/:root_id/replies`
- `POST /api/v1/threads/:root_id/replies`

### Attachments

- `POST /api/v1/attachments/presign`
- `POST /api/v1/attachments/commit`
- `GET /api/v1/attachments/:id`

### Audit

- `GET /api/v1/audit`

## Mapeo de comandos CLI sugerido

- `galynx auth login`
- `galynx auth me`
- `galynx auth logout`
- `galynx channels list`
- `galynx channels create --name <name> [--private]`
- `galynx channels delete <channel_id>`
- `galynx messages list --channel <id> [--cursor <cursor>] [--limit <n>]`
- `galynx messages send --channel <id> --body "..."`
- `galynx messages edit <message_id> --body "..."`
- `galynx messages delete <message_id>`
- `galynx threads get <root_id>`
- `galynx threads replies <root_id> [--cursor <cursor>] [--limit <n>]`
- `galynx threads reply <root_id> --body "..."`
- `galynx attachments presign --channel <id> --file <path> --content-type <type>`
- `galynx attachments commit --upload-id <id> [--message-id <id>]`
- `galynx attachments get <attachment_id>`
- `galynx audit list [--cursor <cursor>] [--limit <n>]`

## Contratos clave para CLI

## Errores

Formato:

```json
{
  "error": "bad_request",
  "message": "message body is required"
}
```

Codigos:

- `unauthorized`
- `bad_request`
- `too_many_requests`
- `not_found`
- `internal_error`

## Paginacion por cursor

Afecta:

- mensajes de canal
- replies de threads
- auditoria

Respuesta incluye:

```json
{
  "items": [],
  "next_cursor": "1739802200000:123456789"
}
```

Si `next_cursor` viene `null`, no hay mas resultados.

## Roles y permisos

- `owner/admin`: gestion de canales + lectura de audit.
- `member`: no gestiona canales, no lee audit.
- Edicion de mensaje: solo autor.
- Borrado de mensaje: autor o `owner/admin`.

## Adjuntos desde CLI

## Flujo operativo

1. `attachments presign` con metadata de archivo.
2. Subir binario a `upload_url` (HTTP PUT/POST segun backend de storage).
3. `attachments commit` con `upload_id`.
4. `attachments get` para generar URL de descarga temporal.

## Limites actuales

- Max archivo: `100MB`
- Presign TTL: `900s`
- Download TTL: `600s`

## Rate limits actuales

- Auth: `30 req/min`
- WS connect: `12 req/min`
- WS command: `600 req/min`

Para CLI HTTP, manejar `429` con backoff exponencial corto y retry acotado.

## Variables de entorno sugeridas para CLI

- `GALYNX_API_BASE_URL`
- `GALYNX_ACCESS_TOKEN`
- `GALYNX_REFRESH_TOKEN`

## Ejemplos de requests (curl)

### Login

```bash
curl -sS -X POST "$GALYNX_API_BASE_URL/api/v1/auth/login" \
  -H 'content-type: application/json' \
  -d '{"email":"owner@galynx.local","password":"ChangeMe123!"}'
```

### List channels

```bash
curl -sS "$GALYNX_API_BASE_URL/api/v1/channels" \
  -H "authorization: Bearer $GALYNX_ACCESS_TOKEN"
```

### Send message

```bash
curl -sS -X POST "$GALYNX_API_BASE_URL/api/v1/channels/$CHANNEL_ID/messages" \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $GALYNX_ACCESS_TOKEN" \
  -d '{"body_md":"hola desde cli"}'
```

### List audit

```bash
curl -sS "$GALYNX_API_BASE_URL/api/v1/audit?limit=20" \
  -H "authorization: Bearer $GALYNX_ACCESS_TOKEN"
```
