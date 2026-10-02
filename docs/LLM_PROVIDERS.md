# LLM provider configuration

In protected Settings, select the provider, region and API/plan variant, then enter
the matching account's Key. Switching variants requires a newly entered Key;
stored Keys and unsaved input are never forwarded to another variant. DeepSeek and
OpenAI keep their existing model choices. Other IDs are editable with suggestions;
the account catalog owns actual availability. Ark may require an inference endpoint ID.

## Fixed official API bases

| Settings provider ID | API base URL |
|---|---|
| `deepseek` | `https://api.deepseek.com` |
| `openai` | `https://api.openai.com` |
| `zhipu` | `https://open.bigmodel.cn/api/paas/v4` |
| `zai` | `https://api.z.ai/api/paas/v4` |
| `minimax_cn` / `minimax_coding_cn` | `https://api.minimax.cn/v1` |
| `minimax_global` / `minimax_coding_global` | `https://api.minimax.io/v1` |
| `qwen_cn` | `https://dashscope.aliyuncs.com/compatible-mode/v1` |
| `qwen_global` | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` |
| `kimi_cn` | `https://api.moonshot.cn/v1` |
| `kimi_global` | `https://api.moonshot.ai/v1` |
| `kimi_coding_cn` | `https://api.kimi.com/coding/v1` |
| `kimi_coding_global` | `https://api.kimi.ai/coding/v1` |
| `stepfun` | `https://api.stepfun.com/v1` |
| `stepfun_global` | `https://api.stepfun.ai/v1` |
| `stepfun_coding` | `https://api.stepfun.com/step_plan/v1` |
| `stepfun_coding_global` | `https://api.stepfun.ai/step_plan/v1` |
| `hunyuan` | `https://api.hunyuan.cloud.tencent.com/v1` |
| `doubao` | `https://ark.cn-beijing.volces.com/api/v3` |
| `qianfan` | `https://qianfan.baidubce.com/v2` |
| `siliconflow` | `https://api.siliconflow.cn/v1` |
| `zhipu_coding` | `https://open.bigmodel.cn/api/coding/paas/v4` |
| `zai_coding` | `https://api.z.ai/api/coding/paas/v4` |
| `aliyun_coding_cn` | `https://coding.dashscope.aliyuncs.com/v1` |
| `aliyun_coding_global` | `https://coding-intl.dashscope.aliyuncs.com/v1` |
| `volcengine_coding` | `https://ark.cn-beijing.volces.com/api/coding/v3` |
| `qianfan_coding` | `https://qianfan.baidubce.com/v2/coding` |

## Coding Plan and Token Plan eligibility

These entries establish endpoint compatibility, not authorization to use a personal
coding subscription for a multi-user application.
[GLM CN](https://docs.bigmodel.cn/cn/coding-plan/tool/others),
[Z.ai](https://docs.z.ai/devpack/usage-policy),
[Kimi Coding](https://www.kimi.com/code/docs/) and
[Alibaba Cloud](https://help.aliyun.com/en/model-studio/coding-plan)
restrict supported tools/use cases; Alibaba Cloud explicitly excludes application
backends. NovelWorld is not verified as an approved tool. Settings displays these
restrictions before saving. For these restricted plans, use ordinary API credentials
unless the provider has authorized this application; personal-only subscriptions
must not become shared platform Keys. Step Plan documents no platform restriction;
its subscription access remains separate from ordinary API billing.

MiniMax now calls its subscription **Token Plan**. Its subscription Key selects
the entitlement on the same API endpoint; use the corresponding regional plan Key,
not an ordinary API Key. Alibaba Cloud requires its dedicated `sk-sp-…` plan Key.
GLM, Ark and Qianfan likewise require the Key for the selected plan/account. Qianfan
plan models are entered from its console rather than an invented default.
Regions have separate credentials, subscriptions and catalogs. NovelWorld does not
switch to ordinary API endpoints on quota exhaustion; provider-side Extra Usage
may charge extra according to account settings, including Kimi and Step Plan.
Local token counters do not measure
remaining subscription quota; token-price estimates are not subscription bills.
See [MiniMax CN](https://platform.minimaxi.com/subscribe/token-plan),
[MiniMax International](https://platform.minimax.io/subscribe/token-plan),
[Alibaba regional setup](https://help.aliyun.com/en/model-studio/coding-plan-faq),
[Ark setup](https://docs.volcengine.com/docs/ark/coding-plan-personal-get-started?lang=zh)
and [Qianfan setup](https://cloud.baidu.com/doc/qianfan/s/ymmyn5kc2).

## Cost estimates and price snapshot

The administrator usage view estimates token costs from a bundled, offline
snapshot of official public prices. API prices are per million billable tokens;
estimates use the exact provider/model metric labels and show USD and CNY totals
separately. They apply the snapshot's current prices to the reported usage window,
not the prices or account terms that applied when each request ran. If a provider
price varies by request time, context length or another tier that usage metrics do
not retain, the view shows the corresponding range; it cannot reconstruct an
invoice. Missing cache-class usage, cache writes/storage, tools, taxes, discounts,
subscription quota, overage and other provider charges may also make the estimate
incomplete. Unknown or retired model prices remain unpriced, never zero.

Subscription rows are official monthly price references only. They do not infer
which plan was purchased, remaining quota or eligible use, and are never mapped to
ordinary API token prices. Plan restrictions above still apply.

Operators may set `LLM_PRICING_USD_PER_MILLION` or
`LLM_PRICING_CNY_PER_MILLION` to replace the snapshot price for an exact
`provider/model` key. An override takes precedence over the snapshot for that key;
configuring the same key in both currencies is rejected. An override is an
operator estimate, not an official account bill. `USD_CNY_RATE` is optional and
only enables conversion of fixed-price totals; without it, native USD and CNY
estimates remain separate. Price refreshes and overrides affect this usage view
only and never change a registered or Frozen Diagnostic's prices, budget or
evidence.

The [pricing guide](./LLM_PRICING.md) and
[official snapshot](../services/user-service/src/infrastructure/llm-prices.json)
own the verification date, source links, model prices, subscription quotes and
unavailable-price entries.

## Transport and evidence

The existing shared OpenAI-compatible adapter handles synchronous and streaming
Chat Completions. Root URLs add `/v1`; explicit API bases append resources directly,
preserving `/v1`, `/v2`, `/api/v3` and `/api/paas/v4`. This also applies to Responses
and embeddings, although a preset does not imply those APIs are available.
Operator-only `LLM_API_URL` supports other compatible services and workspace domains.
Configure the API base, not the full `/chat/completions` resource. Custom path prefixes
that previously relied on an appended `/v1` must explicitly include that version now.

MiniMax separates reasoning with `reasoning_split` and uses prompt-directed JSON
instead of an undocumented `response_format`; M3 disables default thinking, while
M2.x cannot disable it. Direct GLM 5.3 keeps mandatory thinking enabled with low
reasoning effort; other GLM models and Ark use their documented thinking control. Qwen uses
`enable_thinking` for supported Qwen models. Direct Kimi K2.6 disables thinking and
omits fixed-temperature overrides. Kimi Coding uses its own `k3`/`kimi-for-coding`
aliases with the account's default thinking policy and no temperature overrides;
NovelWorld does not impersonate a supported coding tool's User-Agent.
StepFun's per-frame cumulative usage is reported only on its terminal frame;
other providers retain the strict duplicate-usage guard, including Diagnostic runs.
Other custom models, including direct Moonshot K3/K2.7 Code,
are not qualified for NovelWorld's text-only history; editable IDs do not promise
multi-turn reasoning or tool support.

Saving retains the eight-output-token connection probe. Outside a Diagnostic, a
parsed HTTP-success completion ending in `finish_reason=length` proves connectivity
even if reasoning consumed that allowance. Malformed responses, rejected credentials,
transport errors and Diagnostic evidence failures remain errors. This is not a quality
test. Streams reject truncation, filtering, provider failure and unsupported terminal
reasons before emitting completion, so partial text is not accepted as a successful turn.
Deadlines, admission and bounded retries are unchanged. Additive internal config
`provider` metadata preserves region/plan metric labels; older responses infer
DeepSeek/OpenAI from URL. Diagnostic profiles, identity and ceilings are unchanged.

## Import provider budget

Version: **`import-provider-budget-v1`**. The import claim path enforces this
policy; changes to its thresholds and enforcement must land together.

This policy bounds application-port dispatches by the ingestion pipeline
across the crash window where a provider outcome is unknowable. It does not
promise exactly-once execution, cap exact spend, or count physical HTTP
requests. A fresh `LlmPort::chat_json` dispatch and an `ImagePort::generate`
dispatch are the units enforced here; transport behavior is described
separately below.

### Contract — `import-provider-budget-v1`

1. **Per-attempt application-dispatch ceiling.** The import reserves at most
   **640 application dispatches**: 610 fresh `LlmPort::chat_json` dispatches
   plus at most 30 `ImagePort::generate` dispatches. The atomic LLM token is
   consumed immediately before every port dispatch, including JSON/schema/
   evidence retries and chapter-boundary repair, so concurrency cannot exceed
   the 610 ceiling. The avatar cap reserves the other 30 slots.
   `ensure_import_budget` also rejects a source whose deterministic baseline
   scan plan cannot fit. For the gate's single-chapter 5 MiB boundary fixture,
   that forecast is **4 fixed + 221 character scans + 328 canon scans + 30
   images = 583**, which leaves 57 LLM tokens for validation or boundary-repair
   dispatches. Chapter count is also part of the scan plan, so a highly
   fragmented source can be rejected below the byte envelope. The four fixed
   slots are representative character extraction, narrative-node detection,
   and up to two whole-novel event-selection responses. Event selection is
   skipped without a dispatch when its complete candidate prompt exceeds 16
   KiB.
2. **Attempt ceiling.** A `novel_import_jobs` row MUST NOT be claimed more
   than **3** times. Attempt counting includes the acceptance claim and every
   recovery, lease-expiry, or user-retry claim.
3. **Cross-attempt application-dispatch ceiling.** Derived from (1) and (2):
   at most **3 × 640 = 1920 application-port dispatches** per import. This is
   not a physical HTTP-request or exact-spend ceiling.
4. **Terminal semantics.** A claim attempt for a job already at the ceiling
   MUST mark the job terminally `failed` with failure code
   `budget_exhausted`, set the Novel to `error` with the actionable public
   message "Import provider budget exhausted; re-upload the source", and the
   job MUST never be reclaimed by the recovery scan or resumed by the retry
   endpoint. A failure on the third claim keeps its actual failure code and
   shows re-upload guidance immediately, rather than offering an unusable
   retry. Re-uploading creates a new import with a fresh budget.
5. **Metering and retry proof boundary.** The enforcement evidence is the
   per-claim atomic 610-token LLM-dispatch budget plus the reserved 30 image
   slots. Cross-attempt evidence is the persisted `job.attempt`; structured
   logs and `llm-observability-v1` metrics remain operational evidence. Local
   validation loops are bounded to three fresh responses (initial + two) for
   representative, character-scan, narrative-node, and chapter-boundary JSON;
   three fresh responses for each canon chunk's JSON/evidence gate; and two
   fresh responses for event selection. All consume the same global 610-token
   budget. Separately, the current shared LLM client bounds one port dispatch
   to an initial transport invocation plus at most three retryable invocations
   and at most one JSON-mode fallback invocation. Those transport invocations
   are not additional application-budget tokens, and this policy does not
   claim a wire-level request count. No new high-cardinality metric labels are
   introduced.
6. **Completed work.** Replay of a completed import MUST make no application
   provider dispatch; the kill/restart drill already asserts stub counters
   stay 0→0 after a restart.
7. **Change rule.** Thresholds change only through a reviewed policy change
   approved before the implementation judged against it; a candidate change
   cannot weaken its own gate.

### Acceptance evidence that judges this policy

- The kill/restart drill (`tests/e2e/ingestion_recovery.sh`) forces two
  attempts per novel (one hard kill each at the `chapters` and `enriched`
  boundaries) and its verifier asserts the resulting `attempt <= 3`.
- An integration test seeds a job claimed three times, proves the fourth
  claim marks `budget_exhausted` and no application provider dispatch occurs,
  proves recovery never reclaims it, and proves the retry endpoint returns the
  re-upload guidance without an application provider dispatch.
- A unit test forces a schema retry after the runtime token is exhausted and
  proves the retry is rejected before the LLM port is dispatched.

### Non-goals

- Per-principal and time-window spend ceilings for public profiles (H2/H3).
- Provider-side idempotency keys (unavailable on the configured providers).
- Changing the per-attempt call budget, the avatar cap, or the golden loop's
  two-attempt retry expectations.

Official references checked on 2026-09-26:
[GLM API](https://docs.bigmodel.cn/cn/guide/develop/openai/introduction),
[GLM model/stream parameters](https://docs.z.ai/api-reference/llm/chat-completion),
[Z.ai API](https://docs.z.ai/api-reference/introduction),
[MiniMax CN format](https://platform.minimaxi.com/docs/api-reference/text-openai-api),
[MiniMax International format](https://platform.minimax.io/docs/api-reference/text-openai-api),
[Qwen API](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-chat-completions),
[Kimi CN](https://platform.kimi.com/docs/get-api-key),
[Kimi parameters](https://platform.kimi.ai/docs/api/models-overview),
[Kimi Coding models](https://www.kimi.com/code/docs/kimi-code/models.html),
[Step Plan CN](https://platform.stepfun.com/docs/zh/step-plan/overview),
[Step Plan International](https://platform.stepfun.ai/docs/en/step-plan/overview),
[StepFun API](https://platform.stepfun.com/docs/zh/api-reference/chat/chat-completion-create),
[Hunyuan API](https://cloud.tencent.com/document/product/1729/111007),
[Ark API](https://docs.volcengine.com/docs/ark/base-url-and-authentication?lang=zh),
[Qianfan IDs](https://cloud.baidu.com/doc/qianfan-api/s/Dmba8k71y),
[SiliconFlow API](https://docs.siliconflow.cn/docs/userguide/capabilities/stream-mode).
Offline tests establish catalog/wire/browser behavior, not paid provider execution,
qualification or deployment. H3/H4 qualification journeys continue to use DeepSeek.
