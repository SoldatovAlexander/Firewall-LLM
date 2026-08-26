# Развёртывание

**Docker Compose:**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm:** `helm install fwllm ./deploy/helm/fwllm`

**Бенч:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`
