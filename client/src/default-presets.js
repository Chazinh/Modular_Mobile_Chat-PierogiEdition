export const DEFAULT_PRESETS = {
  presets: [
    {
      id: "deepseek-v41-flash",
      groupRoute: "Deepseek",
      thinkingCanDisable: true,
      selectable: true,
      label: "DeepSeek V4.1 Flash",
      family: "deepseek",
      preferredRoutes: [
        "Deepseek",
        "oc/deepseek-v4.1-flash-free",
        "cbai/deepseek-v4.1-flash",
        "nvidia/deepseek-ai/deepseek-v4.1-flash"
      ],
      routes: [
        { id: "Deepseek" },
        { id: "oc/deepseek-v4.1-flash-free" },
        { id: "cbai/deepseek-v4.1-flash" },
        { id: "nvidia/deepseek-ai/deepseek-v4.1-flash" }
      ],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinking: "enabled",
      available: true
    },
    {
      id: "claude-opus-5-5",
      groupRoute: "Opus",
      selectable: true,
      label: "Claude Opus 5.5",
      family: "claude",
      preferredRoutes: ["Opus", "cc/claude-opus-5-5"],
      routes: [{ id: "Opus" }, { id: "cc/claude-opus-5-5" }],
      efforts: ["low"],
      defaultEffort: "low",
      thinking: "adaptive",
      available: true
    },
    {
      id: "claude-sonnet-5-5",
      groupRoute: "Sonnet",
      thinkingCanDisable: true,
      thinkingOffType: "between_tools",
      selectable: true,
      label: "Claude Sonnet 5.5",
      family: "claude",
      preferredRoutes: ["Sonnet", "cc/claude-sonnet-5-5"],
      routes: [{ id: "Sonnet" }, { id: "cc/claude-sonnet-5-5" }],
      efforts: ["low", "medium"],
      defaultEffort: "medium",
      thinking: "adaptive",
      available: true
    },
    {
      id: "gemini-38-flash",
      groupRoute: "Gemini",
      selectable: true,
      label: "Gemini 3.8 Flash",
      family: "gemini",
      preferredRoutes: [
        "Gemini",
        "ag/gemini-3.8-flash-low",
        "ag/gemini-3.8-flash-medium",
        "ag/gemini-3.8-flash-high",
        "gemini/gemini-3.8-flash"
      ],
      routes: [
        { id: "Gemini" },
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
      id: "glm-52",
      groupRoute: "GLM",
      label: "GLM",
      selectable: true,
      family: "glm",
      preferredRoutes: ["GLM", "cbai/glm-5.2"],
      routes: [{ id: "GLM" }, { id: "cbai/glm-5.2" }],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinkingCanDisable: true,
      thinkingDefault: false,
      thinking: "enabled",
      available: true
    },
    {
      id: "muse-spark-13",
      groupRoute: "Muse",
      selectable: true,
      label: "Muse Spark",
      family: "muse",
      preferredRoutes: ["Muse", "oc-router/meta/muse-spark-open", "oc/muse-spark-1.3-contributor-free"],
      routes: [{ id: "Muse" }, { id: "oc-router/meta/muse-spark-open" }, { id: "oc/muse-spark-1.3-contributor-free" }],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinkingCanDisable: true,
      thinking: "enabled",
      available: true
    },
    {
      id: "nemotron-35",
      groupRoute: "Nemotron",
      selectable: true,
      label: "Nemotron 3.5 Super",
      family: "nemotron",
      preferredRoutes: ["Nemotron", "oc-router/nvidia/nemotron-3.5-super-v1:free", "nvidia/nemotron-3.5-super-v1:free"],
      routes: [{ id: "Nemotron" }, { id: "oc-router/nvidia/nemotron-3.5-super-v1:free" }, { id: "nvidia/nemotron-3.5-super-v1:free" }],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinkingCanDisable: true,
      thinking: "enabled",
      available: true
    },
    {
      id: "minimax-m31",
      groupRoute: "MiniMaxM3.1",
      selectable: true,
      label: "MiniMax M3.1",
      family: "minimax",
      preferredRoutes: [
        "MiniMaxM3.1",
        "openrouter/minimax/minimax-m3.1-flash:free",
        "oc/minimax-m3.1-flash-free",
        "minimax/minimax-m3.1-flash:free"
      ],
      routes: [
        { id: "MiniMaxM3.1" },
        { id: "openrouter/minimax/minimax-m3.1-flash:free" },
        { id: "oc/minimax-m3.1-flash-free" },
        { id: "minimax/minimax-m3.1-flash:free" }
      ],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinkingCanDisable: true,
      thinking: "enabled",
      available: true
    },
    {
      id: "gemma-4",
      groupRoute: "Gemma4",
      selectable: true,
      label: "Gemma 4",
      family: "gemma",
      preferredRoutes: [
        "Gemma4",
        "openrouter/google/gemma-4-31b-it:free",
        "google/gemma-4-31b-it:free",
        "google/gemma-4-31b-it",
        "openrouter/google/gemma-4-31b-it",
        "oc/gemma-4-31b-it-free"
      ],
      routes: [
        { id: "Gemma4" },
        { id: "openrouter/google/gemma-4-31b-it:free" },
        { id: "google/gemma-4-31b-it:free" },
        { id: "google/gemma-4-31b-it" }
      ],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinkingCanDisable: true,
      thinking: "enabled",
      available: true
    }
  ]
};
