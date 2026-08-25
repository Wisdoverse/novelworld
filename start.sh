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

CHECK_MODE=false
RESUME_AFTER_L0=false
case "${1:-}" in
  '') ;;
  --check) CHECK_MODE=true ;;
  --l0-resume) RESUME_AFTER_L0=true ;;
  *) printf 'Usage: %s [--check]\n' "$0" >&2; exit 2 ;;
esac
[[ "$#" -le 1 ]] || { printf 'Usage: %s [--check]\n' "$0" >&2; exit 2; }

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

valid_postgres_identifier() {
  [[ "$1" =~ ^[a-z_][a-z0-9_]{0,62}$ ]]
}

valid_postgres_password() {
  local value=$1 lowered distinct
  lowered=$(printf '%s' "$value" | tr '[:upper:]' '[:lower:]')
  distinct=$(printf '%s' "$value" | fold -w1 | sort -u | wc -l | tr -d ' ')
  [[ "$value" =~ ^[A-Za-z0-9._~-]{16,}$ ]] \
    && [[ "$distinct" -ge 8 ]] \
    && [[ "$lowered" != *placeholder* ]] \
    && [[ "$lowered" != *change_me* ]] \
    && [[ "$lowered" != your_strong_password_here ]]
}

assert_l0_configuration() {
  local file=$1 user database password
  user=$(env_value "$file" POSTGRES_USER)
  database=$(env_value "$file" POSTGRES_DB)
  password=$(env_value "$file" POSTGRES_PASSWORD)
  valid_postgres_identifier "$user" || {
    printf 'POSTGRES_USER must be a lowercase PostgreSQL identifier (1-63 characters).\n' >&2
    return 1
  }
  valid_postgres_identifier "$database" || {
    printf 'POSTGRES_DB must be a lowercase PostgreSQL identifier (1-63 characters).\n' >&2
    return 1
  }
  valid_postgres_password "$password" || {
    printf 'POSTGRES_PASSWORD must be URL-safe, non-placeholder, at least 16 characters, and contain at least 8 distinct characters.\n' >&2
    return 1
  }
}

initialize_l0_configuration() {
  local file=$1 allow_prompt=$2 marker user database password default_user default_database
  L0_CONFIGURED_NOW=0
  marker=$(env_value "$file" BOOTSTRAP_L0_COMPLETE)
  case "$marker" in
    true) assert_l0_configuration "$file"; return ;;
    ''|false) ;;
    *) printf 'BOOTSTRAP_L0_COMPLETE must be exactly true or false.\n' >&2; return 1 ;;
  esac

  user=$(env_value "$file" POSTGRES_USER)
  database=$(env_value "$file" POSTGRES_DB)
  password=$(env_value "$file" POSTGRES_PASSWORD)
  if valid_postgres_identifier "$user" \
    && valid_postgres_identifier "$database" \
    && valid_postgres_password "$password"; then
    # Existing and automation-preseeded installations migrate without a prompt.
    set_env_value "$file" BOOTSTRAP_L0_COMPLETE true
    return
  fi

  if [[ "$allow_prompt" != true || ! -t 0 ]]; then
    printf 'L0 database setup is incomplete. Run ./start.sh interactively, or preseed POSTGRES_USER, POSTGRES_DB, and a strong POSTGRES_PASSWORD in .env.\n' >&2
    return 1
  fi

  printf '\n%b\n' "${CYAN}首次启动：配置必需的本地 PostgreSQL（L0）${NC}"
  printf '%s\n' '数据库密码将自动生成；模型、Redis 和对象存储可稍后配置。'
  if valid_postgres_identifier "$user"; then default_user=$user; else default_user=novel; fi
  if valid_postgres_identifier "$database"; then default_database=$database; else default_database=novel_world; fi
  read -r -p "PostgreSQL 用户名 [${default_user}]: " user
  read -r -p "PostgreSQL 数据库名 [${default_database}]: " database
  user=${user:-$default_user}
  database=${database:-$default_database}
  valid_postgres_identifier "$user" || {
    printf 'PostgreSQL 用户名必须是 1-63 位小写字母、数字或下划线，且不能以数字开头。\n' >&2
    return 1
  }
  valid_postgres_identifier "$database" || {
    printf 'PostgreSQL 数据库名必须是 1-63 位小写字母、数字或下划线，且不能以数字开头。\n' >&2
    return 1
  }

  set_env_value "$file" POSTGRES_USER "$user"
  set_env_value "$file" POSTGRES_DB "$database"
  set_env_value "$file" POSTGRES_PASSWORD "$(random_hex 16)"
  # Commit point: never mark L0 complete until every hard database value is persisted.
  set_env_value "$file" BOOTSTRAP_L0_COMPLETE true
  assert_l0_configuration "$file"
  L0_CONFIGURED_NOW=1
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

if [[ "$CHECK_MODE" == true ]]; then
  temporary_directory=$(mktemp -d)
  trap 'rm -rf -- "$temporary_directory"' EXIT
  fresh_env="$temporary_directory/fresh.env"
  cp .env.example "$fresh_env"
  if initialize_l0_configuration "$fresh_env" false 2>/dev/null; then
    printf 'An unconfigured non-interactive L0 launch was accepted.\n' >&2
    exit 1
  fi
  seeded_password=0123456789abcdef0123456789abcdef
  set_env_value "$fresh_env" POSTGRES_USER novel
  set_env_value "$fresh_env" POSTGRES_DB novel_world
  set_env_value "$fresh_env" POSTGRES_PASSWORD "$seeded_password"
  initialize_l0_configuration "$fresh_env" false
  [[ "$(env_value "$fresh_env" BOOTSTRAP_L0_COMPLETE)" == true ]]
  [[ "$(env_value "$fresh_env" POSTGRES_PASSWORD)" == "$seeded_password" ]]

  invalid_env="$temporary_directory/invalid.env"
  cp .env.example "$invalid_env"
  set_env_value "$invalid_env" BOOTSTRAP_L0_COMPLETE true
  if initialize_l0_configuration "$invalid_env" false 2>/dev/null; then
    printf 'A committed L0 marker bypassed database validation.\n' >&2
    exit 1
  fi
  grep -Fq 'exec bash "$0" --l0-resume' start.sh || {
    printf 'Unix launcher must commit L0 and automatically restart itself.\n' >&2
    exit 1
  }
  printf '%b\n' "${GREEN}Unix launcher self-check passed.${NC}"
  exit 0
fi

[[ -f .env ]] || cp .env.example .env
chmod 600 .env
initialize_l0_configuration .env true
if [[ "$L0_CONFIGURED_NOW" -eq 1 ]]; then
  [[ "$RESUME_AFTER_L0" == false ]] || {
    printf 'L0 setup restart loop detected.\n' >&2
    exit 1
  }
  printf '%b\n' "${GREEN}L0 数据库设置已保存，正在自动重启启动器…${NC}"
  exec bash "$0" --l0-resume
fi

if ! command -v docker >/dev/null 2>&1; then
  printf '%b\n' "${RED}Docker is not installed. See https://docs.docker.com/get-docker/${NC}" >&2
  exit 1
fi
if ! docker compose version >/dev/null 2>&1; then
  printf '%b\n' "${RED}Docker Compose v2 is not available.${NC}" >&2
  exit 1
fi
printf '%b\n' "${GREEN}Docker detected${NC}"

resolve_cache_mode

# Generate L1/root values only. Redis credentials are never invented.
ensure_secret JWT_SECRET change_me_to_a_random_32_char_string 32
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
