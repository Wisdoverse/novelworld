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
