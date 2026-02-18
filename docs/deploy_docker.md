# Despliegue en Docker

## Opcion 1: Docker Compose (API + Mongo + Redis + RustFS)

Desde la raiz del repo:

```bash
docker compose up --build -d
```

Ver logs del API:

```bash
docker compose logs -f galynx-api
```

Validar health:

```bash
curl -sS http://localhost:3000/api/v1/health
curl -sS http://localhost:3000/api/v1/metrics
```

Parar servicios:

```bash
docker compose down
```

Bootstrap operativo (desde el host, contra el mismo entorno de env vars):

```bash
cargo run --bin bootstrap
```

## Opcion 2: Solo API en contenedor

Build:

```bash
docker build -t galynx-api:local .
```

Run (con Mongo/Redis/RustFS externos):

```bash
docker run --rm -p 3000:3000 \
  -e PERSISTENCE_BACKEND=mongo \
  -e MONGO_URI='mongodb://root:password@host.docker.internal:27017/?authSource=admin' \
  -e REDIS_URL='redis://host.docker.internal:6379' \
  -e METRICS_ENABLED='true' \
  -e OTEL_EXPORTER_OTLP_ENDPOINT='http://host.docker.internal:4317' \
  -e OTEL_SERVICE_NAME='galynx-api' \
  -e OTEL_SAMPLE_RATIO='1.0' \
  -e S3_BUCKET='galynx-attachments' \
  -e S3_REGION='us-east-1' \
  -e S3_ENDPOINT='http://host.docker.internal:9000' \
  -e S3_ACCESS_KEY_ID='rustfsadmin' \
  -e S3_SECRET_ACCESS_KEY='rustfsadmin' \
  -e S3_FORCE_PATH_STYLE='true' \
  -e JWT_SECRET='dev-only-change-me-in-prod' \
  galynx-api:local
```

## Variables de entorno principales

- `PORT` (default `3000`)
- `JWT_SECRET`
- `ACCESS_TTL_MINUTES` (default `15`)
- `REFRESH_TTL_DAYS` (default `30`)
- `BOOTSTRAP_WORKSPACE_NAME` (default `Galynx`)
- `BOOTSTRAP_EMAIL` (default `owner@galynx.local`)
- `BOOTSTRAP_PASSWORD` (default `ChangeMe123!`)
- `PERSISTENCE_BACKEND` (`memory` o `mongo`)
- `MONGO_URI` (requerida cuando `PERSISTENCE_BACKEND=mongo`)
- `REDIS_URL` (opcional, habilita pub/sub realtime entre réplicas)
- `METRICS_ENABLED` (default `true`, expone `/api/v1/metrics`)
- `OTEL_EXPORTER_OTLP_ENDPOINT` (opcional, habilita trazas OTLP gRPC)
- `OTEL_SERVICE_NAME` (default `galynx-api`)
- `OTEL_SAMPLE_RATIO` (default `1.0`)
- `S3_BUCKET` (opcional, habilita presign real de adjuntos)
- `S3_REGION` (default `us-east-1`)
- `S3_ENDPOINT` (opcional, para RustFS/S3 compatible)
- `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` (opcionales)
- `S3_FORCE_PATH_STYLE` (default `true`, recomendado con RustFS)
