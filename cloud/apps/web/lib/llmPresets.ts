export type LlmPreset = {
  id: string;
  label: string;
  baseUrl: string;
  suggestedModels: string[];
};

/** OpenAI-compatible provider presets for Cloud BYOK. */
export const LLM_PRESETS: LlmPreset[] = [
  {
    id: "deepseek",
    label: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    suggestedModels: ["deepseek-v4-flash", "deepseek-v4-pro"],
  },
  {
    id: "kimi",
    label: "Kimi",
    baseUrl: "https://api.moonshot.cn/v1",
    suggestedModels: ["moonshot-v1-32k", "moonshot-v1-128k", "moonshot-v1-8k"],
  },
  {
    id: "glm",
    label: "GLM",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    suggestedModels: ["glm-4.5", "glm-4-flash", "glm-4"],
  },
  {
    id: "qwen",
    label: "Qwen",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    suggestedModels: ["qwen-plus", "qwen-max", "qwen-turbo", "qwen2.5-coder"],
  },
  {
    id: "openai",
    label: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    suggestedModels: ["gpt-4.1", "gpt-4o", "gpt-4o-mini"],
  },
  {
    id: "custom",
    label: "Custom",
    baseUrl: "",
    suggestedModels: [],
  },
];

export function findPreset(id: string): LlmPreset {
  return LLM_PRESETS.find((p) => p.id === id) || LLM_PRESETS[LLM_PRESETS.length - 1];
}
