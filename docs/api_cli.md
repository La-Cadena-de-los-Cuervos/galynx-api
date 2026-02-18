# Galynx API - Guia para CLI

Este documento define como consumir `galynx-api` desde un CLI.

## Base

- Base URL local: `http://localhost:3000`
- Prefijo: `/api/v1`

## Variables de entorno del API (backend)

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
- `S3_ENDPOINT` (opcional, para MinIO/S3 compatible)
- `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` (opcionales)
- `S3_FORCE_PATH_STYLE` (default: `true`, recomendado con MinIO)

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
export S3_ACCESS_KEY_ID='minioadmin'
export S3_SECRET_ACCESS_KEY='minioadmin'
export S3_FORCE_PATH_STYLE='true'
cargo run
```

## Ejecucion

Desde este repo:

```bash
cargo run --bin galynx -- --help
```

Ejemplos:

```bash
cargo run --bin galynx -- auth login --email owner@galynx.local --password 'ChangeMe123!'
cargo run --bin galynx -- auth login --email owner@galynx.local --password 'ChangeMe123!' --workspace <workspace_id>
cargo run --bin galynx -- auth me
cargo run --bin galynx -- channels list
cargo run --bin galynx -- messages send --channel <channel_id> --body "hola"
cargo run --bin galynx -- threads get <root_id>
cargo run --bin galynx -- audit list --limit 20
cargo run --bin bootstrap
```

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

### Users

- `GET /api/v1/users`
- `POST /api/v1/users`

### Workspaces

- `GET /api/v1/workspaces`
- `POST /api/v1/workspaces`
- `GET /api/v1/workspaces/:id/members`
- `POST /api/v1/workspaces/:id/members`

### Channels

- `GET /api/v1/channels`
- `POST /api/v1/channels`
- `DELETE /api/v1/channels/:id`
- `GET /api/v1/channels/:id/members`
- `POST /api/v1/channels/:id/members`
- `DELETE /api/v1/channels/:id/members/:user_id`

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
- `galynx auth login --workspace <workspace_id>`
- `galynx auth me`
- `galynx auth logout`
- `galynx workspaces list`
- `galynx workspaces create --name <name>`
- `galynx workspaces members <workspace_id>`
- `galynx workspaces onboard <workspace_id> --email <email> --role <admin|member> [--name <name>] [--password <password>]`
- `galynx users list`
- `galynx users create --email <email> --name <name> --password <password> --role <admin|member>`
- `galynx channels list`
- `galynx channels create --name <name> [--private]`
- `galynx channels delete <channel_id>`
- `galynx channels members <channel_id>`
- `galynx channels member-add <channel_id> --user <user_id>`
- `galynx channels member-remove <channel_id> --user <user_id>`
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

## Comandos ya implementados

- `auth login`
- `auth login --workspace`
- `auth me`
- `auth refresh`
- `auth logout`
- `workspaces list`
- `workspaces create`
- `workspaces members`
- `workspaces onboard`
- `users list`
- `users create`
- `channels list`
- `channels create`
- `channels delete`
- `channels members`
- `channels member-add`
- `channels member-remove`
- `messages list`
- `messages send`
- `messages edit`
- `messages delete`
- `threads get`
- `threads replies`
- `threads reply`
- `attachments presign`
- `attachments commit`
- `attachments get`
- `audit list`

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
