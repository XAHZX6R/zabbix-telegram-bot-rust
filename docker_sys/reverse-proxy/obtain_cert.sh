#!/usr/bin/env bash
# obtain_cert.sh - obtain Let's Encrypt certificate for a given domain using certbot docker image
# Usage: sudo ./obtain_cert.sh your_ip.sslip.io youremail@example.com
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "Usage: $0 DOMAIN EMAIL"
  echo "Example: $0 1.2.3.4.sslip.io you@example.com"
  exit 2
fi

DOMAIN=$1
EMAIL=$2

# Paths on the host where docker-compose mounts letsencrypt data
LE_DIR=/docker_sys/reverse-proxy/etc/letsencrypt
LIB_DIR=/docker_sys/reverse-proxy/var/lib/letsencrypt

echo "Stopping reverse-proxy container if running (so certbot can bind to :80)"
docker compose -f docker-compose.yml down || true

echo "Running certbot (standalone) in a temporary container to request cert for $DOMAIN"
docker run --rm -it \
  -p 80:80 \
  -v "$LE_DIR":/etc/letsencrypt \
  -v "$LIB_DIR":/var/lib/letsencrypt \
  certbot/certbot certonly --non-interactive --agree-tos --standalone \
  --preferred-challenges http --email "$EMAIL" -d "$DOMAIN"

echo "If successful, certs are saved under $LE_DIR/live/$DOMAIN/. Copy fullchain.pem and privkey.pem into /docker_sys/reverse-proxy/etc/nginx/ssl/"

mkdir -p /docker_sys/reverse-proxy/etc/nginx/ssl
cp -v "$LE_DIR/live/$DOMAIN/fullchain.pem" /docker_sys/reverse-proxy/etc/nginx/ssl/fullchain.pem
cp -v "$LE_DIR/live/$DOMAIN/privkey.pem" /docker_sys/reverse-proxy/etc/nginx/ssl/privkey.pem

echo "Restarting reverse-proxy"
docker compose -f docker-compose.yml up -d

echo "Done. Visit https://$DOMAIN/ to test."

