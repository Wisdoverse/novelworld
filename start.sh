#!/usr/bin/env bash
set -euo pipefail

# NovelWorld one-click start. The default profile needs PostgreSQL and the
# bootstrap roots only; Redis is selected explicitly through CACHE_MODE.
cd "$(dirname "$0")"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

printf '%b\n' "${CYAN}NovelWorld — One-Click Start${NC}"

if ! command -v docker >/dev/null 2>&1; then
  printf '%b\n' "${RED}Docker is not installed. See https://docs.docker.com/get-docker/${NC}" >&2
  exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
  printf '%b\n' "${RED}Docker Compose v2 is not available.${NC}" >&2
  exit 1
fi
printf '%b\n' "${GREEN}Docker detected${NC}"

random_hex() {
  openssl rand -hex "$1" 2>/dev/null \
    || od -An -N"$1" -tx1 /dev/urandom | tr -d ' \n'
}

env_value() {
  local file=$1 key=$2 count
  count=$(grep -c "^${key}=" "$file" || true)
  [[ "$count" -le 1 ]] || { printf 'Duplicate %s entries in %s\n' "$key" "$file" >&2; return 1; }
  sed -n "s/^${key}=//p" "$file"
}

set_env_value() {
  local file=$1 key=$2 value=$3 count
  count=$(grep -c "^${key}=" "$file" || true)
  [[ "$count" -le 1 ]] || { printf 'Duplicate %s entries in %s\n' "$key" "$file" >&2; return 1; }
  if [[ "$count" -eq 1 ]]; then
    sed -i "s|^${key}=.*$|${key}=${value}|" "$file"
  else
    printf '\n%s=%s\n' "$key" "$value" >>"$file"
  fi
}

valid_redis_password() {
  local value=$1 lowered distinct
  lowered=$(printf '%s' "$value" | tr '[:upper:]' '[:lower:]')
  distinct=$(printf '%s' "$value" | fold -w1 | sort -u | wc -l | tr -d ' ')
  [[ "$value" =~ ^[A-Za-z0-9._~-]{16,}$ ]] \
    && [[ "$distinct" -ge 8 ]] \
    && [[ "$lowered" != *placeholder* ]] \
    && [[ "$lowered" != *change_me* ]] \
    && [[ "$lowered" != your_redis_password_here ]] \
    && [[ "$lowered" != runtime-redis-only ]]
}

resolve_cache_mode() {
  local mode_count redis_password
  mode_count=$(grep -c '^CACHE_MODE=' .env || true)
  [[ "$mode_count" -le 1 ]] || { printf 'Duplicate CACHE_MODE entries in .env\n' >&2; return 1; }
  redis_password=$(env_value .env REDIS_PASSWORD)

  if [[ "$mode_count" -eq 0 ]]; then
    # Pre-decision launchers generated Redis credentials. Migrate those installs
    # once; a copied current template already contains CACHE_MODE=postgres.
    if [[ -n "$redis_password" && "$redis_password" != your_redis_password_here ]]; then
      CACHE_MODE_VALUE=redis
    else
      CACHE_MODE_VALUE=postgres
      [[ "$redis_password" != your_redis_password_here ]] || set_env_value .env REDIS_PASSWORD ''
    fi
    set_env_value .env CACHE_MODE "$CACHE_MODE_VALUE"
  else
    CACHE_MODE_VALUE=$(env_value .env CACHE_MODE)
  fi

  case "$CACHE_MODE_VALUE" in
    postgres) ;;
    redis)
      valid_redis_password "$redis_password" || {
        printf 'CACHE_MODE=redis requires a URL-safe, non-placeholder REDIS_PASSWORD of at least 16 characters with 8 distinct characters.\n' >&2
        return 1
      }
      ;;
    *) printf 'CACHE_MODE must be exactly postgres or redis.\n' >&2; return 1 ;;
  esac
  readonly CACHE_MODE_VALUE
}

ensure_secret() {
  local key=$1 placeholder=$2 bytes=$3 current value
  current=$(env_value .env "$key")
  if [[ -z "$current" || "$current" == "$placeholder" ]]; then
    value=$(random_hex "$bytes")
    set_env_value .env "$key" "$value"
  fi
}

[[ -f .env ]] || cp .env.example .env
resolve_cache_mode

# Generate L1/root values only. Redis credentials are never invented.
ensure_secret JWT_SECRET change_me_to_a_random_32_char_string 32
ensure_secret POSTGRES_PASSWORD your_strong_password_here 16
ensure_secret RUNTIME_CONFIG_KEY change_me_to_a_random_64_char_hex_string 32
ensure_secret INTERNAL_SERVICE_TOKEN change_me_to_a_random_internal_service_token 32
sed -i 's|^LLM_API_KEY=sk-your-api-key$|LLM_API_KEY=|' .env
sed -i 's|^IMAGE_GEN_API_KEY=sk-your-api-key$|IMAGE_GEN_API_KEY=|' .env
chmod 600 .env
printf '%b\n' "${GREEN}Bootstrap roots ready; AI and administrator setup continue in the browser.${NC}"

export CACHE_MODE=$CACHE_MODE_VALUE
export COMPOSE_PROFILES=
compose_args=(docker compose)
if [[ "$CACHE_MODE_VALUE" == redis ]]; then
  redis_password=$(env_value .env REDIS_PASSWORD)
  export REDIS_PASSWORD=$redis_password
  export REDIS_URL="redis://:${redis_password}@redis:6379"
  compose_args+=(--profile redis)
else
  export REDIS_PASSWORD=
  export REDIS_URL=memory://
fi

printf '%b\n' "${CYAN}Stopping old writers before migrations...${NC}"
docker compose --profile redis down
printf '%b\n' "${CYAN}Starting NovelWorld (cache: ${CACHE_MODE_VALUE})...${NC}"
"${compose_args[@]}" up -d --build --wait --wait-timeout 180

printf '%b\n' "${GREEN}NovelWorld is ready at http://localhost${NC}"
printf '%s\n' 'Stop: docker compose --profile redis down' 'Logs: docker compose logs -f'

if command -v xdg-open >/dev/null 2>&1; then
  xdg-open http://localhost >/dev/null 2>&1 &
elif command -v open >/dev/null 2>&1; then
  open http://localhost >/dev/null 2>&1 &
fi
