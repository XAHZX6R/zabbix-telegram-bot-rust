#!/usr/bin/env bash
# Performs a docker system prune and brings up the Zabbix stack and reverse-proxy
# Usage: sudo ./docker_prune_and_up.sh
set -euo pipefail

echo "=== Pruning unused Docker data (images, containers, networks) ==="
docker system prune -af

echo "=== Bringing down any running compose stacks (root and reverse-proxy) ==="
if docker compose -f docker-compose.yml ps >/dev/null 2>&1; then
  docker compose -f docker-compose.yml down --remove-orphans || true
fi
if docker compose -f docker_sys/reverse-proxy/docker-compose.yml ps >/dev/null 2>&1; then
  docker compose -f docker_sys/reverse-proxy/docker-compose.yml down --remove-orphans || true
fi

echo "=== Starting Zabbix stack ==="
docker compose -f docker-compose.yml up -d --remove-orphans

echo "=== Starting reverse-proxy ==="
docker compose -f docker_sys/reverse-proxy/docker-compose.yml up -d --remove-orphans

echo "Done."

echo "Zabbix UI should be reachable at http://<host-ip>:8081 or http://localhost:8081 if you are on the host machine."

echo "If you configured a domain (e.g. <IP>.sslip.io) and obtained certificates, visit https://<IP>.sslip.io/ to test TLS."

