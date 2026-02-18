# Galynx API

API backend del proyecto Galynx, implementada en Rust con Axum.

Estado actual:
- API HTTP v1 funcional.
- Realtime por WebSocket funcional.
- Persistencia en memoria y Mongo.
- CLI (`galynx`) para operaciones principales.
- Entorno Docker listo para desarrollo local.

## Stack

- Rust (edition 2024)
- Axum
- MongoDB (backend persistente)
- Redis (preparado para fase de escalado)
- Docker / Docker Compose

## Estructura principal

- `src/main.rs`: entrada del servidor.
- `src/app.rs`: estado global y rutas.
- `src/auth.rs`: autenticación JWT + refresh.
- `src/channels.rs`: canales y mensajes.
- `src/threads.rs`: hilos de mensajes.
- `src/attachments.rs`: presign/commit/get.
- `src/audit.rs`: auditoría.
- `src/realtime.rs`: WebSocket.
- `src/storage.rs`: backend memory/mongo.
- `src/observability.rs`: métricas HTTP y middleware de observabilidad.
- `src/bin/galynx.rs`: CLI.
- `src/bin/bootstrap.rs`: bootstrap operativo idempotente.

## Requisitos

- Rust toolchain estable
- Docker (opcional, recomendado para entorno local)
- MongoDB 8+ (si usas persistencia `mongo`)

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
- `METRICS_ENABLED` (default: `true`, expone `GET /api/v1/metrics`)
- `OTEL_EXPORTER_OTLP_ENDPOINT` (opcional, habilita export de trazas OTLP gRPC)
- `OTEL_SERVICE_NAME` (default: `galynx-api`)
- `OTEL_SAMPLE_RATIO` (default: `1.0`)
- `S3_BUCKET` (opcional, habilita presign real de adjuntos)
- `S3_REGION` (default: `us-east-1`)
- `S3_ENDPOINT` (opcional, para MinIO u otro endpoint S3 compatible)
- `S3_ACCESS_KEY_ID` (opcional)
- `S3_SECRET_ACCESS_KEY` (opcional)
- `S3_FORCE_PATH_STYLE` (default: `true`, recomendado para MinIO)

## Ejecutar en local (sin Docker)

### 1) Levantar API en memoria (rápido)

```bash
cargo run
```

### 2) Levantar API con Mongo

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

Health check:

```bash
curl -sS http://localhost:3000/api/v1/health
curl -sS http://localhost:3000/api/v1/metrics
```

## Ejecutar con Docker Compose

```bash
docker compose up --build -d
```

Ver logs del API:

```bash
docker compose logs -f galynx-api
```

Detener:

```bash
docker compose down
```

## CLI

Comandos de ayuda:

```bash
cargo run --bin galynx -- --help
cargo run --bin galynx -- auth --help
```

Flujo mínimo:

```bash
cargo run --bin galynx -- auth login --email owner@galynx.local --password 'ChangeMe123!'
cargo run --bin galynx -- auth me
cargo run --bin galynx -- workspaces list
cargo run --bin galynx -- channels list
```

Bootstrap operativo (idempotente):

```bash
cargo run --bin bootstrap
```

## Documentación

- API para frontend: `docs/api_frontend.md`
- API para CLI: `docs/api_cli.md`
- Documento combinado: `docs/api_frontend_cli.md`
- Despliegue Docker: `docs/deploy_docker.md`
- Plan de implementación: `plan_api_galynx.md`

## OpenAPI

Con el servidor levantado:

- JSON spec: `GET /api/v1/openapi.json`

Ejemplo:

```bash
curl -sS http://localhost:3000/api/v1/openapi.json | jq '.'
```

## Notas

- `cargo run` ejecuta por defecto el binario del API (`galynx-api`).
- Para ejecutar el CLI usa `cargo run --bin galynx -- ...`.
- Si el CLI devuelve datos inesperados, valida que el API esté en `PERSISTENCE_BACKEND=mongo` y vuelve a hacer login para regenerar sesión.
