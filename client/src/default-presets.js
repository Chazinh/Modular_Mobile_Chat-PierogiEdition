const HIDDEN_PRESETS = new Set(['gemma-4', 'minimax-m31', 'nemotron-35', 'muse-spark-13']);
export const narrativeRouteVisible = route => !/(?:gemma[-_ ]?4|minimax|nemotron|muse)/i.test(route || '');
export const narrativePresetVisible = preset => !HIDDEN_PRESETS.has(preset.id)
  && narrativeRouteVisible(preset.family) && narrativeRouteVisible(preset.groupRoute);

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
      id: "gemini-4-argon-medium",
      groupRoute: "Argon",
      thinkingCanDisable: true,
      selectable: true,
      label: "Argon",
      family: "gemini",
      preferredRoutes: [
        "Argon",
        "ag/gemini-4-argon-medium"
      ],
      routes: [
        { id: "Argon" },
        { id: "ag/gemini-4-argon-medium" }
      ],
      efforts: ["low"],
      defaultEffort: "low",
      thinking: "level",
      available: true
    },
    {
      id: "gemini-4-hydrogen",
      groupRoute: "Hydrogen",
      thinkingCanDisable: true,
      selectable: true,
      label: "Hydrogen",
      family: "gemini",
      preferredRoutes: [
        "Hydrogen",
        "ag/gemini-4-hydrogen-high",
        "ag/gemini-4-hydrogen-low"
      ],
      routes: [
        { id: "Hydrogen" },
        { id: "ag/gemini-4-hydrogen-high" },
        { id: "ag/gemini-4-hydrogen-low" }
      ],
      efforts: ["low", "high"],
      defaultEffort: "high",
      effortByRoute: {
        "ag/gemini-4-hydrogen-low": "low",
        "ag/gemini-4-hydrogen-high": "high"
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
      preferredRoutes: [
        "GLM",
        "openrouter/z-ai/glm-5.3-flash",
        "openrouter/z-ai/glm-5.2",
        "oc/glm-5.3-flash-free",
        "cbai/glm-5.2"
      ],
      routes: [
        { id: "GLM" },
        { id: "openrouter/z-ai/glm-5.3-flash" },
        { id: "openrouter/z-ai/glm-5.2" },
        { id: "oc/glm-5.3-flash-free" },
        { id: "cbai/glm-5.2" }
      ],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinkingCanDisable: true,
      thinkingDefault: false,
      thinking: "enabled",
      available: true
    },
    {
      id: "gpt-6-2-sol",
      groupRoute: "Gpt-sol-6.2",
      thinkingCanDisable: true,
      selectable: true,
      label: "Gpt-sol-6.2",
      family: "openai",
      preferredRoutes: [
        "Gpt-sol-6.2",
        "cx/gpt-6.2-sol",
        "cx/gpt-6.2-sol(high)",
        "cx/gpt-6.2-sol[1m]"
      ],
      routes: [
        { id: "Gpt-sol-6.2" },
        { id: "cx/gpt-6.2-sol" },
        { id: "cx/gpt-6.2-sol(high)" },
        { id: "cx/gpt-6.2-sol[1m]" }
      ],
      efforts: ["low", "medium", "high"],
      defaultEffort: "high",
      thinking: "enabled",
      available: true
    }
  ]
};
