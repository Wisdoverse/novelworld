import { translate as t } from '@/shared/lib/i18n';
type ProviderPreset = { label: string; endpoint: string; models: string[]; plan?: boolean; restricted?: boolean };

// These are suggestions; account-specific model IDs remain editable.
export const PROVIDERS = {
  deepseek: { label: 'DeepSeek', endpoint: 'https://api.deepseek.com', models: ['deepseek-flash'] },
  openai: { label: 'OpenAI', endpoint: 'https://api.openai.com', models: ['gpt-4o-mini'] },
  zhipu: { get label() { return t("Zhipu GLM · China API"); }, endpoint: 'https://open.bigmodel.cn/api/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'] },
  zai: { get label() { return t("Z.ai GLM · Global API"); }, endpoint: 'https://api.z.ai/api/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'] },
  minimax_cn: { get label() { return t("MiniMax · China API"); }, endpoint: 'https://api.minimax.cn/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'] },
  minimax_global: { get label() { return t("MiniMax · Global API"); }, endpoint: 'https://api.minimax.io/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'] },
  qwen_cn: { get label() { return t("Qwen / Bailian · China API"); }, endpoint: 'https://dashscope.aliyuncs.com/compatible-mode/v1', models: ['qwen-plus', 'qwen-flash', 'qwen3-coder-plus'] },
  qwen_global: { get label() { return t("Qwen / Bailian · Global API"); }, endpoint: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1', models: ['qwen-plus', 'qwen-flash', 'qwen3-coder-plus'] },
  kimi_cn: { get label() { return t("Kimi · China API"); }, endpoint: 'https://api.moonshot.cn/v1', models: ['kimi-k2.6'] },
  kimi_global: { get label() { return t("Kimi · Global API"); }, endpoint: 'https://api.moonshot.ai/v1', models: ['kimi-k2.6'] },
  stepfun: { get label() { return t("StepFun · China API"); }, endpoint: 'https://api.stepfun.com/v1', models: ['step-3.5-flash'] },
  stepfun_global: { get label() { return t("StepFun · Global API"); }, endpoint: 'https://api.stepfun.ai/v1', models: ['step-3.5-flash'] },
  hunyuan: { get label() { return t("Tencent Hunyuan · China API"); }, endpoint: 'https://api.hunyuan.cloud.tencent.com/v1', models: ['hunyuan-turbos-latest'] },
  doubao: { get label() { return t("Doubao / Volcengine Ark · China API"); }, endpoint: 'https://ark.cn-beijing.volces.com/api/v3', models: ['doubao-seed-2-1-pro-260628'] },
  qianfan: { get label() { return t("ERNIE / Baidu Qianfan · China API"); }, endpoint: 'https://qianfan.baidubce.com/v2', models: ['ernie-4.5-turbo-128k'] },
  siliconflow: { get label() { return t("SiliconFlow · China API"); }, endpoint: 'https://api.siliconflow.cn/v1', models: ['Qwen/Qwen3-235B-A22B-Instruct-2507'] },
  zhipu_coding: { get label() { return t("Zhipu Coding Plan · China"); }, endpoint: 'https://open.bigmodel.cn/api/coding/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'], plan: true, restricted: true },
  zai_coding: { get label() { return t("Z.ai Coding Plan · Global"); }, endpoint: 'https://api.z.ai/api/coding/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'], plan: true, restricted: true },
  minimax_coding_cn: { get label() { return t("MiniMax Coding / Token Plan · China"); }, endpoint: 'https://api.minimax.cn/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'], plan: true },
  minimax_coding_global: { get label() { return t("MiniMax Coding / Token Plan · Global"); }, endpoint: 'https://api.minimax.io/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'], plan: true },
  aliyun_coding_cn: { get label() { return t("Bailian Coding Plan · China"); }, endpoint: 'https://coding.dashscope.aliyuncs.com/v1', models: ['qwen3.6-plus', 'qwen3-coder-plus', 'glm-5', 'kimi-k2.5', 'MiniMax-M2.5'], plan: true, restricted: true },
  aliyun_coding_global: { get label() { return t("Bailian Coding Plan · Global"); }, endpoint: 'https://coding-intl.dashscope.aliyuncs.com/v1', models: ['qwen3.6-plus', 'qwen3-coder-plus', 'glm-5', 'kimi-k2.5', 'MiniMax-M2.5'], plan: true, restricted: true },
  volcengine_coding: { get label() { return t("Volcengine Ark Coding Plan · China"); }, endpoint: 'https://ark.cn-beijing.volces.com/api/coding/v3', models: ['ark-code-latest', 'doubao-seed-2.1-pro', 'glm-5.3', 'minimax-m3'], plan: true },
  qianfan_coding: { get label() { return t("Baidu Qianfan Coding Plan · China"); }, endpoint: 'https://qianfan.baidubce.com/v2/coding', models: [], plan: true },
  kimi_coding_cn: { get label() { return t("Kimi Coding Plan · China"); }, endpoint: 'https://api.kimi.com/coding/v1', models: ['k3', 'k3-256k', 'kimi-for-coding'], plan: true, restricted: true },
  kimi_coding_global: { get label() { return t("Kimi Coding Plan · Global"); }, endpoint: 'https://api.kimi.ai/coding/v1', models: ['k3', 'k3-256k', 'kimi-for-coding'], plan: true, restricted: true },
  stepfun_coding: { get label() { return t("StepFun Step Plan · China"); }, endpoint: 'https://api.stepfun.com/step_plan/v1', models: ['step-3.5-flash', 'step-3.7-flash', 'step-5-preview'], plan: true },
  stepfun_coding_global: { get label() { return t("StepFun Step Plan · Global"); }, endpoint: 'https://api.stepfun.ai/step_plan/v1', models: ['step-3.5-flash', 'step-3.7-flash', 'step-5-preview'], plan: true },
} satisfies Record<string, ProviderPreset>;

export type EditableProvider = keyof typeof PROVIDERS;
export const providerOptions = Object.entries(PROVIDERS) as [EditableProvider, ProviderPreset][];
