# llm-client

NovelWorld's internal OpenAI-compatible LLM transport.

Production services construct `RuntimeLlmClient::from_env()`. It retrieves the
active, user-scoped provider configuration from user-service using
`USER_SERVICE_URL` and `INTERNAL_SERVICE_TOKEN`; provider credentials are not
discovered from a registry of environment variables. `static_config` exists for
explicit static and test configurations.

Agent-service configures its embedding transport directly:

```rust
let client = LlmClient::new().with_openai_compatible(
    "provider-name",
    api_key,
    api_url,
);
```

The supported transport surface is deliberately small:

- `chat` and `chat_stream` for OpenAI-compatible chat completions
- `embed` for OpenAI-compatible embeddings
- `longform_chat_for_user` and `json_chat_for_user` for the two runtime prompt
  policies used by NovelWorld

Every chat request declares an `LlmOperation` and an output-token limit. The
non-streaming JSON chat path has at most five explicit HTTP attempts (the
initial request, up to three transport retries, and one JSON fallback); this
is an attempt bound, not a monetary spending cap. Chat, stream setup, and
embedding calls share bounded admission, a total deadline, three
exponential-backoff retries, `Retry-After` handling, and Prometheus
attempt/request metrics. Automatic redirects and reqwest transport retries are
disabled, so a custom provider that redirects must be configured with its
final URL. Embedding transport metrics use the separate
`novelworld_embedding_*` namespace; they do not enter the closed
`llm-observability-v1` chat budget until embedding usage semantics have their
own versioned policy.
