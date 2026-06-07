#!/usr/bin/env bash
set -e

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

# ─── Setup .env ──────────────────────────────────────────────────────────
if [ ! -f .env ]; then
    echo ""
    echo -e "${YELLOW}First time setup — let's configure your environment.${NC}"
    echo ""
    cp .env.example .env

    # Generate random JWT secret
    JWT_SECRET=$(openssl rand -hex 32 2>/dev/null || head -c 64 /dev/urandom | base64 | tr -dc 'a-zA-Z0-9' | head -c 64)
    sed -i "s|change_me_to_a_random_32_char_string|${JWT_SECRET}|g" .env

    # Generate random passwords
    PG_PASS=$(openssl rand -hex 16 2>/dev/null || echo "novel_pg_$(date +%s)")
    REDIS_PASS=$(openssl rand -hex 16 2>/dev/null || echo "novel_redis_$(date +%s)")
    sed -i "s|your_strong_password_here|${PG_PASS}|g" .env
    sed -i "s|your_redis_password_here|${REDIS_PASS}|g" .env

    # Ask for LLM API key
    echo -e "${CYAN}Which LLM provider do you want to use?${NC}"
    echo ""
    echo "  1) OpenAI          (needs OPENAI_API_KEY)"
    echo "  2) DeepSeek         (needs DEEPSEEK_API_KEY)"
    echo "  3) 通义千问 Qwen    (needs QWEN_API_KEY)"
    echo "  4) GLM 智谱AI       (needs GLM_API_KEY)"
    echo "  5) Anthropic Claude (needs ANTHROPIC_API_KEY)"
    echo "  6) Moonshot / Kimi  (needs MOONSHOT_API_KEY)"
    echo "  7) 豆包 Doubao      (needs DOUBAO_API_KEY)"
    echo "  8) Other / Custom   (OpenAI-compatible URL)"
    echo ""
    read -p "Choose [1-8, default 1]: " PROVIDER_CHOICE
    PROVIDER_CHOICE=${PROVIDER_CHOICE:-1}

    echo ""
    case $PROVIDER_CHOICE in
        1)
            read -p "Enter your OpenAI API Key: " API_KEY
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            sed -i "s|IMAGE_GEN_API_KEY=sk-your-api-key|IMAGE_GEN_API_KEY=${API_KEY}|g" .env
            echo "OPENAI_API_KEY=${API_KEY}" >> .env
            ;;
        2)
            read -p "Enter your DeepSeek API Key: " API_KEY
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            sed -i "s|LLM_API_URL=https://api.openai.com|LLM_API_URL=https://api.deepseek.com|g" .env
            sed -i "s|LLM_MODEL=gpt-4o-mini|LLM_MODEL=deepseek-chat|g" .env
            echo "DEEPSEEK_API_KEY=${API_KEY}" >> .env
            ;;
        3)
            read -p "Enter your Qwen/DashScope API Key: " API_KEY
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            sed -i "s|LLM_API_URL=https://api.openai.com|LLM_API_URL=https://dashscope.aliyuncs.com/compatible-mode|g" .env
            sed -i "s|LLM_MODEL=gpt-4o-mini|LLM_MODEL=qwen-max|g" .env
            echo "QWEN_API_KEY=${API_KEY}" >> .env
            ;;
        4)
            read -p "Enter your GLM/ZhipuAI API Key: " API_KEY
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            sed -i "s|LLM_API_URL=https://api.openai.com|LLM_API_URL=https://open.bigmodel.cn/api/paas|g" .env
            sed -i "s|LLM_MODEL=gpt-4o-mini|LLM_MODEL=glm-4-flash|g" .env
            echo "GLM_API_KEY=${API_KEY}" >> .env
            ;;
        5)
            read -p "Enter your Anthropic API Key: " API_KEY
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            echo "ANTHROPIC_API_KEY=${API_KEY}" >> .env
            ;;
        6)
            read -p "Enter your Moonshot API Key: " API_KEY
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            sed -i "s|LLM_API_URL=https://api.openai.com|LLM_API_URL=https://api.moonshot.cn|g" .env
            sed -i "s|LLM_MODEL=gpt-4o-mini|LLM_MODEL=moonshot-v1-8k|g" .env
            echo "MOONSHOT_API_KEY=${API_KEY}" >> .env
            ;;
        7)
            read -p "Enter your Doubao API Key: " API_KEY
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            sed -i "s|LLM_API_URL=https://api.openai.com|LLM_API_URL=https://ark.cn-beijing.volces.com/api/v3|g" .env
            sed -i "s|LLM_MODEL=gpt-4o-mini|LLM_MODEL=doubao-1.5-pro-32k|g" .env
            echo "DOUBAO_API_KEY=${API_KEY}" >> .env
            ;;
        8)
            read -p "Enter API URL (e.g. https://api.example.com): " API_URL
            read -p "Enter API Key: " API_KEY
            read -p "Enter Model name: " MODEL_NAME
            sed -i "s|LLM_API_KEY=sk-your-api-key|LLM_API_KEY=${API_KEY}|g" .env
            sed -i "s|LLM_API_URL=https://api.openai.com|LLM_API_URL=${API_URL}|g" .env
            sed -i "s|LLM_MODEL=gpt-4o-mini|LLM_MODEL=${MODEL_NAME}|g" .env
            ;;
    esac

    echo ""
    echo -e "${GREEN}✓ Configuration saved to .env${NC}"
else
    echo -e "${GREEN}✓ .env already exists${NC}"
fi

# ─── Start ───────────────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}Starting NovelWorld...${NC}"
echo ""

docker compose up -d --build 2>&1 | tail -5

echo ""
echo -e "${GREEN}╔═══════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║                    ✅ NovelWorld is running!              ║${NC}"
echo -e "${GREEN}╠═══════════════════════════════════════════════════════════╣${NC}"
echo -e "${GREEN}║                                                           ║${NC}"
echo -e "${GREEN}║   🌐 Open:  ${CYAN}http://localhost${GREEN}                              ║${NC}"
echo -e "${GREEN}║   📡 API:   ${CYAN}http://localhost:8080${GREEN}                         ║${NC}"
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
