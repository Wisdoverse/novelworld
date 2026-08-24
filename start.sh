#!/usr/bin/env bash
set -eo pipefail

# ═══════════════════════════════════════════════════════════════════════════
# NovelWorld — One-Click Start
# Just run: ./start.sh
# ═══════════════════════════════════════════════════════════════════════════

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${CYAN}"
echo "╔═══════════════════════════════════════════════════════════╗"
echo "║              📖 NovelWorld — One-Click Start             ║"
echo "║     Transform any novel into an interactive world        ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo -e "${NC}"

# ─── Check Docker ────────────────────────────────────────────────────────
if ! command -v docker &> /dev/null; then
    echo -e "${RED}❌ Docker is not installed.${NC}"
    echo "   Please install Docker: https://docs.docker.com/get-docker/"
    exit 1
fi

if ! docker compose version &> /dev/null; then
    echo -e "${RED}❌ Docker Compose is not available.${NC}"
    echo "   Please install Docker Compose v2."
    exit 1
fi

echo -e "${GREEN}✓ Docker detected${NC}"

# ─── Setup server-only secrets ───────────────────────────────────────────
random_hex() {
    openssl rand -hex "$1" 2>/dev/null || od -An -N"$1" -tx1 /dev/urandom | tr -d ' \n'
}

ensure_secret() {
    key="$1"
    placeholder="$2"
    bytes="$3"
    current=$(sed -n "s/^${key}=//p" .env | tail -n 1)
    if [ -z "$current" ] || [ "$current" = "$placeholder" ]; then
        value=$(random_hex "$bytes")
        if grep -q "^${key}=" .env; then
            sed -i "s|^${key}=.*$|${key}=${value}|" .env
        else
            printf '\n%s=%s\n' "$key" "$value" >> .env
        fi
    fi
}

if [ ! -f .env ]; then
    cp .env.example .env
fi

ensure_secret "JWT_SECRET" "change_me_to_a_random_32_char_string" 32
ensure_secret "POSTGRES_PASSWORD" "your_strong_password_here" 16
ensure_secret "REDIS_PASSWORD" "your_redis_password_here" 16
ensure_secret "RUNTIME_CONFIG_KEY" "change_me_to_a_random_64_char_hex_string" 32
ensure_secret "INTERNAL_SERVICE_TOKEN" "change_me_to_a_random_internal_service_token" 32

# Migrate untouched templates to the web setup path without changing real keys.
sed -i 's|^LLM_API_KEY=sk-your-api-key$|LLM_API_KEY=|' .env
sed -i 's|^IMAGE_GEN_API_KEY=sk-your-api-key$|IMAGE_GEN_API_KEY=|' .env
chmod 600 .env
echo -e "${GREEN}✓ Server secrets ready; AI and administrator setup will continue in the browser${NC}"

# ─── Start ───────────────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}Starting NovelWorld...${NC}"
echo ""

docker compose down
docker compose up -d --build 2>&1 | tail -5

echo ""
echo -e "${GREEN}╔═══════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║                    ✅ NovelWorld is running!              ║${NC}"
echo -e "${GREEN}╠═══════════════════════════════════════════════════════════╣${NC}"
echo -e "${GREEN}║                                                           ║${NC}"
echo -e "${GREEN}║   🌐 Open:  ${CYAN}http://localhost${GREEN}                              ║${NC}"
echo -e "${GREEN}║   📡 API:   ${CYAN}http://localhost/api${GREEN}                          ║${NC}"
echo -e "${GREEN}║                                                           ║${NC}"
echo -e "${GREEN}║   Stop:     docker compose down                           ║${NC}"
echo -e "${GREEN}║   Logs:     docker compose logs -f                        ║${NC}"
echo -e "${GREEN}║   Restart:  ./start.sh                                    ║${NC}"
echo -e "${GREEN}║                                                           ║${NC}"
echo -e "${GREEN}╚═══════════════════════════════════════════════════════════╝${NC}"

# Try to open browser
if command -v xdg-open &> /dev/null; then
    xdg-open http://localhost 2>/dev/null &
elif command -v open &> /dev/null; then
    open http://localhost 2>/dev/null &
fi
