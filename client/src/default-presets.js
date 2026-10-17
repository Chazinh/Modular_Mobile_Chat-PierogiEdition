export const DEFAULT_PRESETS = {
  presets: [
    {
      id: "deepseek-v41-flash",
      thinkingCanDisable: true,
      selectable: true,
      label: "DeepSeek V4.1 Flash",
      family: "deepseek",
      preferredRoutes: [
        "cbai/deepseek-v4.1-flash",
        "nvidia/deepseek-ai/deepseek-v4.1-flash",
        "oc/deepseek-v4.1-flash-free"
      ],
      routes: [
        { id: "cbai/deepseek-v4.1-flash" },
        { id: "nvidia/deepseek-ai/deepseek-v4.1-flash" },
        { id: "oc/deepseek-v4.1-flash-free" }
      ],
      efforts: ["high"],
      defaultEffort: "high",
      thinking: "enabled",
      available: true
    },
    {
      id: "claude-opus-5-5",
      selectable: true,
      label: "Claude Opus 5.5",
      family: "claude",
      preferredRoutes: ["cc/claude-opus-5-5"],
      routes: [{ id: "cc/claude-opus-5-5" }],
      efforts: ["low"],
      defaultEffort: "low",
      thinking: "adaptive",
      available: true
    },
    {
      id: "claude-sonnet-5-5",
      thinkingCanDisable: true,
      thinkingOffType: "between_tools",
      selectable: true,
      label: "Claude Sonnet 5.5",
      family: "claude",
      preferredRoutes: ["cc/claude-sonnet-5-5"],
      routes: [{ id: "cc/claude-sonnet-5-5" }],
      efforts: ["low", "medium"],
      defaultEffort: "medium",
      thinking: "adaptive",
      available: true
    },
    {
      id: "gemini-38-flash",
      selectable: true,
      label: "Gemini 3.8 Flash",
      family: "gemini",
      preferredRoutes: [
        "ag/gemini-3.8-flash-low",
        "ag/gemini-3.8-flash-medium",
        "ag/gemini-3.8-flash-high",
        "gemini/gemini-3.8-flash"
      ],
      routes: [
        { id: "ag/gemini-3.8-flash-low" },
        { id: "ag/gemini-3.8-flash-medium" },
        { id: "ag/gemini-3.8-flash-high" },
        { id: "gemini/gemini-3.8-flash" }
      ],
      routeAliases: {
        "ag/gemini-3.8-flash": "ag/gemini-3.8-flash-low"
      },
      efforts: ["low", "medium", "high"],
      defaultEffort: "low",
      effortByRoute: {
        "ag/gemini-3.8-flash-low": "low",
        "ag/gemini-3.8-flash-medium": "medium",
        "ag/gemini-3.8-flash-high": "high"
      },
      thinking: "level",
      available: true
    },
    {
      id: "kimi-k3",
      label: "Kimi K3",
      selectable: true,
      family: "kimi",
      preferredRoutes: ["nvidia/moonshotai/kimi-k3"],
      routes: [{ id: "nvidia/moonshotai/kimi-k3" }],
      efforts: ["high"],
      defaultEffort: "high",
      thinking: "always",
      available: true
    },
    {
      id: "muse-spark-13",
      label: "Muse Spark 1.3",
      selectable: true,
      family: "muse",
      preferredRoutes: ["oc/muse-spark-1.3-contributor-free"],
      routes: [{ id: "oc/muse-spark-1.3-contributor-free" }],
      efforts: ["high"],
      defaultEffort: "high",
      thinking: "enabled",
      available: true
    },
    {
      id: "glm-52",
      label: "GLM",
      selectable: true,
      family: "glm",
      preferredRoutes: ["cbai/glm-5.2", "nvidia/z-ai/glm-5.2"],
      routes: [{ id: "cbai/glm-5.2" }, { id: "nvidia/z-ai/glm-5.2" }],
      efforts: ["high"],
      defaultEffort: "high",
      thinkingCanDisable: true,
      thinkingDefault: false,
      thinking: "enabled",
      available: true
    },
    {
      id: "step-5-preview",
      label: "Step 5 Preview",
      selectable: true,
      family: "step",
      preferredRoutes: ["oc/step-5-preview-free"],
      routes: [{ id: "oc/step-5-preview-free" }],
      efforts: ["high"],
      defaultEffort: "high",
      thinking: "enabled",
      available: true
    }
  ]
};
