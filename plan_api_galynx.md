# Plan de Desarrollo del API de Galynx (Rust + Axum)

## Resumen
Construir `galynx-api` como un monolito modular en Rust que cubra auth, roles, canales, mensajes realtime, threads, archivos, auditoría y reacciones, usando contrato OpenAPI-first, Mongo-compatible (DocumentDB en prod), WebSocket at-least-once con idempotencia y stack local con Docker Compose.

## Objetivo y criterios de éxito
- Entregar un API funcional para el cliente Galynx con versionado `/api/v1`.
- Implementar todos los endpoints HTTP y comandos/eventos WS definidos.
- Autenticación JWT access + refresh stateful con rotación y revocación.
- Realtime estable con deduplicación por `client_msg_id`.
- Flujo de archivos presign/commit y control de acceso por canal/workspace.
- Auditoría persistente en acciones críticas.
- CI bloqueante con unit + integración + smoke e2e.

## Alcance funcional
- Auth: `login`, `refresh`, `logout`, `GET /me`.
- RBAC por workspace: owner/admin/member.
- Canales: listar, crear, borrar; privados con membresía explícita.
- Mensajes: crear, editar, borrar, listar con cursor.
- Threads: leer thread, listar replies, crear reply.
- Reacciones: agregar/quitar y propagar eventos.
- Archivos: presign, commit, metadata y descarga prefirmada.
- Auditoría: registro obligatorio y consulta por API.
- Realtime WS: comandos y eventos definidos en el plan base.

## Arquitectura técnica
- Servicio: monolito modular (`auth`, `channels`, `messages`, `threads`, `reactions`, `attachments`, `audit`, `realtime`, `platform`).
- Contrato: OpenAPI-first como fuente de verdad.
- HTTP: Axum con middlewares de auth, RBAC, trace-id, rate limiting y auditoría.
- WS: endpoint autenticado, heartbeat cada ~30s, fan-out por canal/thread.
- Persistencia: Mongo driver (compatible con DocumentDB).
- Escalado: Redis pub/sub para múltiples réplicas.
- Observabilidad: logs estructurados + métricas + trazas OTel.

## Interfaces públicas (v1)
- `POST /api/v1/auth/login`
- `POST /api/v1/auth/refresh`
- `POST /api/v1/auth/logout`
- `GET /api/v1/me`
- `GET /api/v1/channels`
- `POST /api/v1/channels`
- `DELETE /api/v1/channels/{id}`
- `GET /api/v1/channels/{id}/messages?cursor=...&limit=...`
- `PATCH /api/v1/messages/{id}`
- `DELETE /api/v1/messages/{id}`
- `GET /api/v1/threads/{root_id}`
- `GET /api/v1/threads/{root_id}/replies?cursor=...&limit=...`
- `POST /api/v1/threads/{root_id}/replies`
- `POST /api/v1/attachments/presign`
- `POST /api/v1/attachments/commit`
- `GET /api/v1/attachments/{id}`
- `GET /api/v1/audit?cursor=...&limit=...`
- `GET /api/v1/ws` (upgrade WebSocket)

## WebSocket
### Comandos cliente -> servidor
- `SEND_MESSAGE`
- `EDIT_MESSAGE`
- `DELETE_MESSAGE`
- `FETCH_MORE`
- `FETCH_THREAD`
- `ADD_REACTION`
- `REMOVE_REACTION`

### Eventos servidor -> cliente
- `WELCOME`
- `MESSAGE_CREATED`
- `MESSAGE_UPDATED`
- `MESSAGE_DELETED`
- `THREAD_UPDATED`
- `CHANNEL_CREATED`
- `CHANNEL_DELETED`
- `REACTION_UPDATED`

### Garantía y deduplicación
- Entrega at-least-once.
- Idempotencia por `(sender_id, channel_id, client_msg_id)`.
- ACK con `message_id` definitivo (UUIDv7).

## Modelo de datos
- Colecciones: `users`, `workspaces`, `workspace_members`, `channels`, `channel_members`, `messages`, `threads`, `attachments`, `audit_log`, `refresh_sessions`, `reactions`.

### Adiciones importantes
- `refresh_sessions`: sesión refresh stateful (hash, expiración, rotación, revocación).
- `reactions`: índice único por `(message_id, emoji, user_id)`.

### Índices clave
- `users.email` unique.
- `workspace_members(workspace_id,user_id)` unique.
- `channels(workspace_id,name)` unique.
- `messages(channel_id,created_at,_id)`.
- `audit_log(workspace_id,created_at,_id)`.
- `refresh_sessions(user_id,expires_at)` + TTL.
- `reactions(message_id,emoji,user_id)` unique.

## Seguridad
- Password hashing con Argon2id.
- Access token corto (ej. 15 min).
- Refresh token stateful (ej. 30 días), rotatorio, hash en DB y revocación por logout.
- Detección de reuse de refresh para invalidar cadena comprometida.
- Rate limiting básico por IP+usuario en auth y WS.
- TLS en tránsito y acceso interno por VPN en producción.

## Flujo de archivos
1. Cliente solicita `presign`.
2. API valida permisos, MIME y tamaño (hasta 100 MB).
3. Cliente sube directo a S3/MinIO.
4. Cliente llama `commit`.
5. API registra metadata en `attachments`.
6. Descarga mediante URL prefirmada con control de acceso.

## Roadmap de implementación
1. Fundaciones: estructura modular, config, logging/tracing, healthchecks, OpenAPI base.
2. Auth/RBAC: JWT, refresh stateful, middleware de autorización.
3. Canales y mensajes HTTP: CRUD + paginación cursor.
4. Realtime WS: conexión autenticada, fan-out, ACK idempotente.
5. Threads y reacciones: replies y actualización en tiempo real.
6. Archivos: presign/commit/get con MinIO local y S3 prod.
7. Auditoría: captura sistemática y endpoint de consulta.
8. Escalado: Redis pub/sub y hardening operativo.

## Entorno local y operación
- Docker Compose para Mongo, Redis y MinIO.
- Script/CLI de bootstrap para crear primer workspace + owner.
- CI con lint/build/tests y smoke e2e.

## Pruebas requeridas
- Unit: RBAC, validaciones, reglas de edición/borrado, refresh rotation.
- Integración: auth completo, canales/mensajes, archivos, auditoría.
- WS integración: conexión, comandos, eventos, reconexión, deduplicación.
- Smoke e2e: flujo completo desde login hasta auditoría.
- Seguridad: expiración/revocación/reuse y denegación cross-workspace.

## Supuestos y defaults
- Base de datos: DocumentDB/Mongo-compatible.
- Contrato: OpenAPI-first.
- Arquitectura: monolito modular.
- Versionado: `/api/v1`.
- Realtime: at-least-once + idempotencia.
- Refresh: stateful con rotación y revocación.
- Paginación: cursor por `created_at` + `_id`.
- Observabilidad: logs + métricas + trazas OTel.

## Estado actual (checkpoint 2026-02-18)

### Funcional hoy
- API HTTP v1 operativa en Axum.
- Auth: `login`, `refresh`, `logout`, `me`.
- Canales y mensajes: listar, crear, editar, borrar, paginación por cursor.
- Threads: resumen, replies, crear reply.
- Attachments: `presign`, `commit`, `get` (flujo lógico de metadata).
- Auditoría: escritura y consulta.
- WebSocket: conexión autenticada, comandos y eventos principales.
- WebSocket con bridge Redis opcional (`REDIS_URL`) para fan-out entre réplicas.
- Idempotencia WS implementada para comandos mutantes (`SEND_MESSAGE`, `EDIT_MESSAGE`, `DELETE_MESSAGE`, `ADD_REACTION`, `REMOVE_REACTION`) con `client_msg_id`.
- Índices Mongo base creados al arranque (unique + consulta) y TTL para expiración de `refresh_sessions`.
- Attachments con presign/download reales S3 compatibles (MinIO mediante `S3_*` env vars).
- Canales privados con control de acceso por membresía explícita (`channel_members`) y bypass owner/admin.
- API para gestionar membresía de canal (`GET/POST/DELETE /channels/{id}/members`).
- CI base agregada con checks bloqueantes (`fmt`, `clippy`, `test`, `build`).
- Persistencia Mongo operativa (con configuración por env).
- CLI funcional para operaciones principales.
- Empaquetado Docker (`Dockerfile` + `docker-compose.yml`).

### Pendiente para siguiente fase
- Completar modelo de datos objetivo (`workspaces` y onboarding de usuarios para administración de `channel_members`).
- Agregar observabilidad avanzada (métricas y trazas OTel).
- Extender CI con pruebas de integración/WS/e2e smoke sobre servicios reales.
- Añadir bootstrap operativo formal (script/flujo de inicialización).
