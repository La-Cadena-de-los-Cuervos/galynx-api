# Galynx – Plan completo de desarrollo

## 1. Visión general

**Galynx** es una aplicación de mensajería interna tipo Slack, orientada a equipos técnicos, con foco en:
- Tiempo real confiable
- Threads y archivos como funcionalidades de primera clase
- Seguridad (VPN, TLS, cifrado en reposo)
- Arquitectura moderna en Rust

**Alcance inicial**: uso interno, ~10 usuarios, con crecimiento planeado a 100+.

---

## 2. Stack tecnológico

### Cliente (Desktop)
- **Tauri**
- UI Web (Nuxt)
- Renderizado de Markdown (con sanitización)
- WebSocket para tiempo real
- Subida directa de archivos a S3 mediante URLs prefirmadas

### Backend
- **Rust + Axum** (HTTP + WebSocket)
- Autenticación email/password
- Autorización por roles
- Eventos en tiempo real (fan-out)

### Infraestructura
- **EKS (Kubernetes)**
- **ALB** como Ingress (compatible con WebSocket)
- **DocumentDB** (Mongo-compatible)
- **S3** para archivos
- **CloudWatch** para logs
- Redis (ElastiCache) para pub/sub cuando haya múltiples réplicas

### Identidad y seguridad
- Hash de passwords: **argon2id**
- Tokens: JWT access + refresh
- TLS en tránsito
- Cifrado en reposo (KMS)
- Acceso solo desde VPN

---

## 3. Roles y permisos

### Roles
- **Owner**
- **Admin**
- **Member**

### Permisos
| Acción | Owner | Admin | Member |
|------|------|------|--------|
| Crear canales | ✅ | ✅ | ❌ |
| Invitar usuarios | ✅ | ✅ | ❌ |
| Enviar mensajes | ✅ | ✅ | ✅ |
| Editar mensajes propios | ✅ | ✅ | ✅ |
| Eliminar mensajes | ✅ | ✅ | ❌ |
| Eliminar canales | ✅ | ✅ | ❌ |
| Deshabilitar usuarios | ✅ | ✅ | ❌ |

---

## 4. Modelo de datos (DocumentDB)

### users
- _id
- email (unique)
- name
- password_hash
- status (active / disabled)
- created_at

### workspaces
- _id
- name
- created_at

### workspace_members
- _id
- workspace_id
- user_id
- role (owner/admin/member)
- status
- created_at

### channels
- _id
- workspace_id
- name
- is_private
- created_by
- created_at

### channel_members (solo privados)
- _id
- channel_id
- user_id
- created_at

### messages
- _id (UUIDv7)
- workspace_id
- channel_id
- sender_id
- body_md
- mentions [user_id]
- thread_root_id (nullable)
- created_at
- edited_at (nullable)
- deleted_at (nullable)
- client_msg_id

### threads
- _id (root_message_id)
- channel_id
- workspace_id
- reply_count
- last_reply_at
- participants [user_id]

### attachments
- _id
- workspace_id
- channel_id
- message_id
- uploader_id
- filename
- content_type
- size_bytes
- storage { bucket, key, region }
- created_at

### audit_log
- _id
- workspace_id
- actor_id
- action
- target_type
- target_id
- metadata (json)
- created_at

---

## 5. Realtime (WebSocket)

### Eventos (server → client)
- WELCOME
- MESSAGE_CREATED
- MESSAGE_UPDATED
- MESSAGE_DELETED
- THREAD_UPDATED
- CHANNEL_CREATED
- CHANNEL_DELETED
- REACTION_UPDATED

### Comandos (client → server)
- SEND_MESSAGE
- EDIT_MESSAGE
- DELETE_MESSAGE
- FETCH_MORE
- FETCH_THREAD
- ADD_REACTION
- REMOVE_REACTION

### Reglas
- UI optimista con client_msg_id
- ACK del servidor con UUIDv7 definitivo
- Ping/keepalive cada ~30s para ALB

---

## 6. API HTTP (Axum)

### Auth
- POST /auth/login
- POST /auth/refresh
- POST /auth/logout

### Core
- GET /me
- GET /channels
- POST /channels
- DELETE /channels/{id}

### Mensajes
- GET /channels/{id}/messages
- PATCH /messages/{id}
- DELETE /messages/{id}

### Threads
- GET /threads/{root_id}
- GET /threads/{root_id}/replies
- POST /threads/{root_id}/replies

### Archivos
- POST /attachments/presign
- POST /attachments/commit
- GET /attachments/{id}

### Auditoría
- GET /audit

---

## 7. Archivos (hasta 100 MB)

### Flujo
1. Cliente solicita presign
2. Backend valida permisos y tamaño
3. Cliente sube directo a S3
4. Cliente confirma commit
5. Mensaje referencia attachment_id

### Notas
- Sin preview en MVP
- Descarga vía URL prefirmada
- Control de acceso por canal

---

## 8. Auditoría

Se registra:
- Login / logout
- Crear / borrar canales
- Envío / edición / borrado de mensajes
- Subida de archivos
- Deshabilitar usuarios

Auditoría se guarda siempre, incluso si la UI no la expone aún.

---

## 9. Roadmap por fases

### Fase 0 – Fundaciones
- Repo, CI, config, logging

### Fase 1 – Auth y roles
- Login, refresh, permisos

### Fase 2 – Canales + mensajes realtime
- WS, historial, markdown

### Fase 3 – Threads
- Panel derecho, replies

### Fase 4 – Archivos
- Presign, upload, commit

### Fase 5 – Auditoría
- Middleware + endpoints

### Fase 6 – Escalado
- Redis pub/sub
- HPA

---

## 10. Riesgos y mitigaciones

| Riesgo | Mitigación |
|------|-----------|
| Orden de mensajes | UUIDv7 + índices |
| WS en ALB | keepalive + reconnect |
| DocumentDB limits | queries simples, índices claros |
| Archivos grandes | upload directo S3 |
| Permisos | middleware centralizado |

---

## 11. Fuera de MVP (planeado)
- Presence / typing
- Autocomplete de menciones
- Preview de archivos
- Bots e integraciones
- Exportación de conversaciones
- Auto-update de desktop

---

**Este documento define el plan base de Galynx y debe evolucionar conforme el producto crezca.**

