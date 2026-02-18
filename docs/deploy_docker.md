# Despliegue en Docker

## Opcion 1: Docker Compose (API + Mongo + Redis)

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
```

Parar servicios:

```bash
docker compose down
```

## Opcion 2: Solo API en contenedor

Build:

```bash
docker build -t galynx-api:local .
```

Run (con Mongo externo):

```bash
docker run --rm -p 3000:3000 \
  -e PERSISTENCE_BACKEND=mongo \
  -e MONGO_URI='mongodb://root:password@host.docker.internal:27017/?authSource=admin' \
  -e REDIS_URL='redis://host.docker.internal:6379' \
  -e JWT_SECRET='dev-only-change-me-in-prod' \
  galynx-api:local
```

## Variables de entorno principales

- `PORT` (default `3000`)
- `JWT_SECRET`
- `ACCESS_TTL_MINUTES` (default `15`)
- `REFRESH_TTL_DAYS` (default `30`)
- `BOOTSTRAP_EMAIL` (default `owner@galynx.local`)
- `BOOTSTRAP_PASSWORD` (default `ChangeMe123!`)
- `PERSISTENCE_BACKEND` (`memory` o `mongo`)
- `MONGO_URI` (requerida cuando `PERSISTENCE_BACKEND=mongo`)
- `REDIS_URL` (opcional, habilita pub/sub realtime entre réplicas)
