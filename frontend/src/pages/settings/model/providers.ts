type ProviderPreset = { label: string; endpoint: string; models: string[]; plan?: boolean; restricted?: boolean };

// These are suggestions; account-specific model IDs remain editable.
export const PROVIDERS = {
  deepseek: { label: 'DeepSeek', endpoint: 'https://api.deepseek.com', models: ['deepseek-flash'] },
  openai: { label: 'OpenAI', endpoint: 'https://api.openai.com', models: ['gpt-4o-mini'] },
  zhipu: { label: '智谱 GLM · 中国 API', endpoint: 'https://open.bigmodel.cn/api/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'] },
  zai: { label: 'Z.ai GLM · 国际 API', endpoint: 'https://api.z.ai/api/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'] },
  minimax_cn: { label: 'MiniMax · 中国 API', endpoint: 'https://api.minimax.cn/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'] },
  minimax_global: { label: 'MiniMax · 国际 API', endpoint: 'https://api.minimax.io/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'] },
  qwen_cn: { label: '千问 / 百炼 · 中国 API', endpoint: 'https://dashscope.aliyuncs.com/compatible-mode/v1', models: ['qwen-plus', 'qwen-flash', 'qwen3-coder-plus'] },
  qwen_global: { label: '千问 / 百炼 · 国际 API', endpoint: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1', models: ['qwen-plus', 'qwen-flash', 'qwen3-coder-plus'] },
  kimi_cn: { label: 'Kimi · 中国 API', endpoint: 'https://api.moonshot.cn/v1', models: ['kimi-k2.6'] },
  kimi_global: { label: 'Kimi · 国际 API', endpoint: 'https://api.moonshot.ai/v1', models: ['kimi-k2.6'] },
  stepfun: { label: '阶跃星辰 · 中国 API', endpoint: 'https://api.stepfun.com/v1', models: ['step-3.5-flash'] },
  stepfun_global: { label: '阶跃星辰 · 国际 API', endpoint: 'https://api.stepfun.ai/v1', models: ['step-3.5-flash'] },
  hunyuan: { label: '腾讯混元 · 中国 API', endpoint: 'https://api.hunyuan.cloud.tencent.com/v1', models: ['hunyuan-turbos-latest'] },
  doubao: { label: '豆包 / 火山方舟 · 中国 API', endpoint: 'https://ark.cn-beijing.volces.com/api/v3', models: ['doubao-seed-2-1-pro-260628'] },
  qianfan: { label: '文心 / 百度千帆 · 中国 API', endpoint: 'https://qianfan.baidubce.com/v2', models: ['ernie-4.5-turbo-128k'] },
  siliconflow: { label: '硅基流动 · 中国 API', endpoint: 'https://api.siliconflow.cn/v1', models: ['Qwen/Qwen3-235B-A22B-Instruct-2507'] },
  zhipu_coding: { label: '智谱 Coding Plan · 中国', endpoint: 'https://open.bigmodel.cn/api/coding/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'], plan: true, restricted: true },
  zai_coding: { label: 'Z.ai Coding Plan · 国际', endpoint: 'https://api.z.ai/api/coding/paas/v4', models: ['glm-5.3', 'glm-5.3-flash'], plan: true, restricted: true },
  minimax_coding_cn: { label: 'MiniMax Coding / Token Plan · 中国', endpoint: 'https://api.minimax.cn/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'], plan: true },
  minimax_coding_global: { label: 'MiniMax Coding / Token Plan · 国际', endpoint: 'https://api.minimax.io/v1', models: ['MiniMax-M3', 'MiniMax-M2.7', 'MiniMax-M2.7-highspeed'], plan: true },
  aliyun_coding_cn: { label: '百炼 Coding Plan · 中国', endpoint: 'https://coding.dashscope.aliyuncs.com/v1', models: ['qwen3.6-plus', 'qwen3-coder-plus', 'glm-5', 'kimi-k2.5', 'MiniMax-M2.5'], plan: true, restricted: true },
  aliyun_coding_global: { label: '百炼 Coding Plan · 国际', endpoint: 'https://coding-intl.dashscope.aliyuncs.com/v1', models: ['qwen3.6-plus', 'qwen3-coder-plus', 'glm-5', 'kimi-k2.5', 'MiniMax-M2.5'], plan: true, restricted: true },
  volcengine_coding: { label: '火山方舟 Coding Plan · 中国', endpoint: 'https://ark.cn-beijing.volces.com/api/coding/v3', models: ['ark-code-latest', 'doubao-seed-2.1-pro', 'glm-5.3', 'minimax-m3'], plan: true },
  qianfan_coding: { label: '百度千帆 Coding Plan · 中国', endpoint: 'https://qianfan.baidubce.com/v2/coding', models: [], plan: true },
  kimi_coding_cn: { label: 'Kimi Coding Plan · 中国', endpoint: 'https://api.kimi.com/coding/v1', models: ['k3', 'k3-256k', 'kimi-for-coding'], plan: true, restricted: true },
  kimi_coding_global: { label: 'Kimi Coding Plan · 国际', endpoint: 'https://api.kimi.ai/coding/v1', models: ['k3', 'k3-256k', 'kimi-for-coding'], plan: true, restricted: true },
  stepfun_coding: { label: '阶跃 Step Plan · 中国', endpoint: 'https://api.stepfun.com/step_plan/v1', models: ['step-3.5-flash', 'step-3.7-flash', 'step-5-preview'], plan: true },
  stepfun_coding_global: { label: '阶跃 Step Plan · 国际', endpoint: 'https://api.stepfun.ai/step_plan/v1', models: ['step-3.5-flash', 'step-3.7-flash', 'step-5-preview'], plan: true },
} satisfies Record<string, ProviderPreset>;

export type EditableProvider = keyof typeof PROVIDERS;
export const providerOptions = Object.entries(PROVIDERS) as [EditableProvider, ProviderPreset][];
