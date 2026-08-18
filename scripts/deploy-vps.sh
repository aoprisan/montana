#!/usr/bin/env bash
# Build the Linux scraper in Docker, upload the PWA, and configure a daily timer.
# Usage: ./scripts/deploy-vps.sh [root@SERVER_IP] [domain]
set -euo pipefail

cd "$(dirname "$0")/.."

DEFAULT_HOST=root@93.115.53.191
DEFAULT_DOMAIN=calendartrail.ro
HOST="${1:-${MONTANA_DEPLOY_HOST:-$DEFAULT_HOST}}"
DOMAIN="${2:-${MONTANA_DEPLOY_DOMAIN:-$DEFAULT_DOMAIN}}"

if [[ ! "$DOMAIN" =~ ^[a-z0-9.-]+$ ]] || [[ "$DOMAIN" != *.* ]]; then
  printf 'error: invalid domain: %s\n' "$DOMAIN" >&2
  exit 2
fi

for command_name in docker file node rsync ssh; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'error: required command not found: %s\n' "$command_name" >&2
    exit 2
  }
done

node scripts/validate.mjs

release_id="$(date -u +%Y%m%d%H%M%S)-$(git rev-parse --short HEAD 2>/dev/null || printf local)"
release_dir="/opt/montana/releases/$release_id"
build_dir="$(mktemp -d)"
trap 'rm -rf "$build_dir"' EXIT

docker buildx build \
  --platform linux/amd64 \
  --target artifact \
  --output "type=local,dest=$build_dir" \
  --file deploy/Dockerfile.scraper .
file "$build_dir/montana-scraper" | grep -q 'ELF 64-bit.*x86-64' || {
  printf 'error: Docker did not produce a Linux x86-64 scraper binary\n' >&2
  exit 2
}

ssh "$HOST" "set -eu
  getent passwd montana >/dev/null 2>&1 || useradd --system --home-dir /nonexistent --shell /usr/sbin/nologin montana
  install -d -o root -g root -m 0755 '$release_dir' /opt/montana/releases /opt/montana/bin /opt/montana/deploy
  install -d -o montana -g montana -m 0750 /opt/montana/backups"
rsync -az --delete app/ "$HOST:$release_dir/"
rsync -az deploy/Caddyfile "$HOST:/opt/montana/Caddyfile"
rsync -az deploy/montana-scraper.service deploy/montana-scraper.timer "$HOST:/opt/montana/deploy/"
rsync -az "$build_dir/montana-scraper" "$HOST:/opt/montana/bin/montana-scraper.$release_id"

ssh "$HOST" "set -eu
  install -o root -g root -m 0755 '/opt/montana/bin/montana-scraper.$release_id' /opt/montana/bin/montana-scraper.next
  mv /opt/montana/bin/montana-scraper.next /opt/montana/bin/montana-scraper
  rm -f '/opt/montana/bin/montana-scraper.$release_id'
  chown -R montana:montana '$release_dir'
  install -o root -g root -m 0644 /opt/montana/deploy/montana-scraper.service /etc/systemd/system/montana-scraper.service
  install -o root -g root -m 0644 /opt/montana/deploy/montana-scraper.timer /etc/systemd/system/montana-scraper.timer
  ln -sfn '$release_dir' /opt/montana/current
  sed 's/__DOMAIN__/$DOMAIN/g' /opt/montana/Caddyfile > /etc/caddy/sites-enabled/montana.caddy
  caddy validate --config /etc/caddy/Caddyfile
  systemctl daemon-reload
  systemctl reload caddy
  systemctl enable --now montana-scraper.timer
  systemctl start montana-scraper.service
  test \"\$(systemctl is-active caddy)\" = active
  test \"\$(systemctl is-active montana-scraper.timer)\" = active
  test -f /opt/montana/current/index.html"

printf '\nDeployment %s is installed for https://%s/; the scraper runs daily.\n' "$release_id" "$DOMAIN"
