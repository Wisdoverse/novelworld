# llm-client

Unified Rust LLM client supporting 20 providers with a single API.

## Quick Start

```rust
use llm_client::{LlmClient, ChatRequest, EmbeddingRequest};

let client = LlmClient::new()
    .with_openai("sk-...")
    .with_anthropic("sk-ant-...")
    .with_deepseek("sk-...");

// Simple chat
let answer = client.simple_chat("openai/gpt-4o", "You are helpful.", "Hello!").await?;

// JSON mode
let json = client.json_chat("deepseek/deepseek-chat", "Return {name, age}").await?;

// Streaming
let stream = client.chat_stream(
    ChatRequest::new("anthropic/claude-sonnet-4-20250514")
        .message("user", "Write a poem")
).await?;

// Embeddings
let embed = client.embed(EmbeddingRequest {
    model: "openai/text-embedding-3-small".into(),
    input: "Hello world".into(),
}).await?;
```

## Providers & Models

### OpenAI

```rust
client.with_openai(key)
```

| Model | ID |
|-------|-----|
| GPT-4o | `openai/gpt-4o` |
| GPT-4o mini | `openai/gpt-4o-mini` |
| GPT-4.1 | `openai/gpt-4.1` |
| GPT-4.1 mini | `openai/gpt-4.1-mini` |
| GPT-4.1 nano | `openai/gpt-4.1-nano` |
| o3 | `openai/o3` |
| o3-mini | `openai/o3-mini` |
| o4-mini | `openai/o4-mini` |
| text-embedding-3-small | `openai/text-embedding-3-small` |
| text-embedding-3-large | `openai/text-embedding-3-large` |

### Anthropic

```rust
client.with_anthropic(key)
```

| Model | ID |
|-------|-----|
| Claude Opus 4 | `anthropic/claude-opus-4-20250514` |
| Claude Sonnet 4 | `anthropic/claude-sonnet-4-20250514` |
| Claude Haiku 3.5 | `anthropic/claude-3-5-haiku-20241022` |

### Google Gemini

```rust
client.with_gemini(key)
```

| Model | ID |
|-------|-----|
| Gemini 2.5 Pro | `gemini/gemini-2.5-pro` |
| Gemini 2.5 Flash | `gemini/gemini-2.5-flash` |
| Gemini 2.0 Flash | `gemini/gemini-2.0-flash` |
| text-embedding-004 | `gemini/text-embedding-004` |

### DeepSeek

```rust
client.with_deepseek(key)
```

| Model | ID |
|-------|-----|
| DeepSeek-V3 | `deepseek/deepseek-chat` |
| DeepSeek-R1 | `deepseek/deepseek-reasoner` |
| DeepSeek-Coder | `deepseek/deepseek-coder` |

### 豆包 Doubao (ByteDance)

```rust
client.with_doubao(key)       // CN (default)
client.with_doubao_intl(key)  // International
```

| Model | ID |
|-------|-----|
| Doubao-1.5-pro | `doubao/doubao-1.5-pro-32k` |
| Doubao-1.5-lite | `doubao/doubao-1.5-lite-32k` |
| Doubao-vision-pro | `doubao/doubao-1.5-vision-pro-32k` |

### 通义千问 Qwen (Alibaba)

```rust
client.with_qwen(key)       // CN (default)
client.with_qwen_intl(key)  // International
```

| Model | ID |
|-------|-----|
| Qwen3-235B | `qwen/qwen3-235b-a22b` |
| Qwen3-32B | `qwen/qwen3-32b` |
| Qwen3-Coder | `qwen/qwen3-coder-plus` |
| Qwen-Max | `qwen/qwen-max` |
| Qwen-Turbo | `qwen/qwen-turbo` |
| text-embedding-v3 | `qwen/text-embedding-v3` |

### GLM 智谱AI (ZhipuAI)

```rust
client.with_glm(key)        // CN (default)
client.with_glm_cn(key)     // CN
client.with_glm_intl(key)   // International
```

| Model | ID |
|-------|-----|
| GLM-4-Plus | `glm/glm-4-plus` |
| GLM-4-Air | `glm/glm-4-air` |
| GLM-4-Flash | `glm/glm-4-flash` |
| GLM-4-Long | `glm/glm-4-long` |
| CodeGeeX-4 | `glm/codegeex-4` |
| Embedding-3 | `glm/embedding-3` |

### MiniMax

```rust
client.with_minimax(key)
```

| Model | ID |
|-------|-----|
| MiniMax-Text-01 | `minimax/MiniMax-Text-01` |
| abab6.5s | `minimax/abab6.5s-chat` |

### Moonshot 月之暗面 (Kimi)

```rust
client.with_moonshot(key)
```

| Model | ID |
|-------|-----|
| moonshot-v1-128k | `moonshot/moonshot-v1-128k` |
| moonshot-v1-32k | `moonshot/moonshot-v1-32k` |
| moonshot-v1-8k | `moonshot/moonshot-v1-8k` |

### 百川 Baichuan

```rust
client.with_baichuan(key)
```

| Model | ID |
|-------|-----|
| Baichuan4-Turbo | `baichuan/Baichuan4-Turbo` |
| Baichuan4-Air | `baichuan/Baichuan4-Air` |

### 阶跃星辰 Stepfun

```rust
client.with_stepfun(key)
```

| Model | ID |
|-------|-----|
| step-2-16k | `stepfun/step-2-16k` |
| step-1-128k | `stepfun/step-1-128k` |

### 零一万物 Yi

```rust
client.with_yi(key)
```

| Model | ID |
|-------|-----|
| yi-lightning | `yi/yi-lightning` |
| yi-large | `yi/yi-large` |

### 讯飞星火 iFlytek Spark

```rust
client.with_spark(key)
```

| Model | ID |
|-------|-----|
| spark-max | `spark/spark-max` |
| spark-pro | `spark/spark-pro` |
| spark-lite | `spark/spark-lite` |

### 小米 Xiaomi

```rust
client.with_xiaomi(key)
```

| Model | ID |
|-------|-----|
| MiMo-7B | `xiaomi/MiMo-7B` |

### Mistral

```rust
client.with_mistral(key)
```

| Model | ID |
|-------|-----|
| Mistral Large | `mistral/mistral-large-latest` |
| Mistral Small | `mistral/mistral-small-latest` |
| Codestral | `mistral/codestral-latest` |

### Groq

```rust
client.with_groq(key)
```

| Model | ID |
|-------|-----|
| Llama 3.3 70B | `groq/llama-3.3-70b-versatile` |
| Llama 4 Scout | `groq/meta-llama/llama-4-scout-17b-16e-instruct` |
| Gemma 2 9B | `groq/gemma2-9b-it` |

### Together AI

```rust
client.with_together(key)
```

| Model | ID |
|-------|-----|
| Llama 3.3 70B | `together/meta-llama/Llama-3.3-70B-Instruct-Turbo` |
| Qwen 2.5 72B | `together/Qwen/Qwen2.5-72B-Instruct-Turbo` |
| DeepSeek V3 | `together/deepseek-ai/DeepSeek-V3` |

### Local / Self-hosted

```rust
client.with_ollama()                      // Ollama at localhost:11434
client.with_vllm("http://gpu-server:8000") // vLLM at custom URL
```

## Custom Provider

Any OpenAI-compatible API:

```rust
client.with_openai_compatible("my-provider", api_key, "https://my-api.com")
// then use: "my-provider/model-name"
```

## Features

- **Model routing**: `"provider/model"` format auto-routes to correct API
- **3 native formats**: OpenAI, Anthropic Messages API, Gemini generateContent
- **Retry**: 3x exponential backoff (1s, 2s, 4s) with Retry-After support
- **Streaming**: SSE stream parsing for all providers
- **Embeddings**: OpenAI + Gemini + compatible providers
- **JSON mode**: Force JSON output where supported
