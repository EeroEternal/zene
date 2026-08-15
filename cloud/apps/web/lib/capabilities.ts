/** Named Console capabilities. Import from `@/cap/<id>`; do not copy into a new page. */
export const capabilities = {
  llm: {
    use: "BYOK LLM settings, readiness, model list",
    symbols: ["isLlmReady", "llmApi", "useLlmSettings", "ModelPicker"],
    files: [
      "cloud/apps/api/src/features/llm.rs",
      "cloud/apps/web/lib/cloud/llm.ts",
      "cloud/apps/web/lib/hooks/useLlmSettings.ts",
      "cloud/apps/web/components/pickers/ModelPicker.tsx",
    ],
  },
  repositories: {
    use: "org repos and branches",
    symbols: ["repositoriesApi", "useRepoBranches", "BranchPicker", "ProjectPicker"],
    files: [
      "cloud/apps/api/src/features/repositories.rs",
      "cloud/apps/web/lib/cloud/repositories.ts",
      "cloud/apps/web/lib/hooks/useRepoBranches.ts",
      "cloud/apps/web/components/pickers/ProjectPicker.tsx",
      "cloud/apps/web/components/pickers/BranchPicker.tsx",
    ],
  },
  github: {
    use: "GitHub connect, status, repo sync",
    symbols: ["githubApi"],
    files: [
      "cloud/apps/api/src/features/github.rs",
      "cloud/apps/web/lib/cloud/github.ts",
    ],
  },
  session: {
    use: "current user and email sign-in",
    symbols: ["authApi", "meApi"],
    files: ["cloud/apps/web/lib/cloud/session.ts"],
  },
  runs: {
    use: "create/list/follow/cancel a run, git publish",
    symbols: ["runsApi"],
    files: ["cloud/apps/web/lib/cloud/runs.ts"],
  },
  composer: {
    use: "task / follow-up prompt with / skills and @ files",
    symbols: ["Composer", "useComposerText"],
    files: [
      "cloud/apps/web/lib/hooks/useComposerText.ts",
      "cloud/apps/web/components/composer/Composer.tsx",
    ],
  },
  "project-picker": {
    use: "choose a connected GitHub repo",
    symbols: ["ProjectPicker"],
    files: ["cloud/apps/web/components/pickers/ProjectPicker.tsx"],
  },
  "branch-picker": {
    use: "choose a repo branch",
    symbols: ["BranchPicker"],
    files: ["cloud/apps/web/components/pickers/BranchPicker.tsx"],
  },
  "model-picker": {
    use: "choose the run model (needs useLlmSettings)",
    symbols: ["ModelPicker", "useLlmSettings"],
    files: ["cloud/apps/web/components/pickers/ModelPicker.tsx"],
  },
  "attach-menu": {
    use: "attach files, skills, MCP, permission, max turns",
    symbols: ["AttachMenu"],
    files: ["cloud/apps/web/components/pickers/AttachMenu.tsx"],
  },
  picker: {
    use: "any new searchable list or field select",
    symbols: ["FieldSelect", "SearchablePicker"],
    files: [
      "cloud/apps/web/components/ui/Picker.tsx",
      "cloud/apps/web/components/ui/FieldSelect.tsx",
    ],
  },
  menu: {
    use: "anchored action menu",
    symbols: ["Menu", "MenuItem", "MenuSep", "useDismiss"],
    files: [
      "cloud/apps/web/components/ui/Menu.tsx",
      "cloud/apps/web/components/ui/useDismiss.ts",
    ],
  },
  dialogs: {
    use: "confirm, prompt, toast (never window.alert/confirm/prompt)",
    symbols: ["ConfirmDialog", "PromptDialog", "useToast"],
    files: [
      "cloud/apps/web/components/ui/ConfirmDialog.tsx",
      "cloud/apps/web/components/ui/PromptDialog.tsx",
      "cloud/apps/web/components/Toast.tsx",
    ],
  },
  http: {
    use: "new typed Cloud client method",
    symbols: ["deleteJson", "getJson", "patchJson", "postJson", "putJson"],
    files: ["cloud/apps/web/lib/cloud/http.ts"],
  },
} as const;

export type CapabilityId = keyof typeof capabilities;

export function capability(id: CapabilityId) {
  return capabilities[id];
}

/** One import for a capability. Mix ids: `./cloud/scripts/use-capability.sh llm composer`. */
export function capImport(id: CapabilityId): string {
  const { symbols } = capabilities[id];
  return `import { ${symbols.join(", ")} } from "@/cap/${id}";`;
}
