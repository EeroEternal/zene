/** Named Console capabilities. Import these modules; do not copy them into a new page. */
export const capabilities = {
  llm: {
    use: "BYOK LLM settings, readiness, model list",
    import: [
      'import { isLlmReady, llmApi } from "@/lib/cloud";',
      'import { useLlmSettings } from "@/lib/hooks";',
      'import { ModelPicker } from "@/components/pickers";',
    ],
    files: [
      "cloud/apps/api/src/features/llm.rs",
      "cloud/apps/web/lib/cloud/llm.ts",
      "cloud/apps/web/lib/hooks/useLlmSettings.ts",
      "cloud/apps/web/components/pickers/ModelPicker.tsx",
    ],
  },
  repositories: {
    use: "org repos and branches",
    import: [
      'import { repositoriesApi } from "@/lib/cloud";',
      'import { useRepoBranches } from "@/lib/hooks";',
      'import { BranchPicker, ProjectPicker } from "@/components/pickers";',
    ],
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
    import: ['import { githubApi } from "@/lib/cloud";'],
    files: [
      "cloud/apps/api/src/features/github.rs",
      "cloud/apps/web/lib/cloud/github.ts",
    ],
  },
  session: {
    use: "current user and email sign-in",
    import: ['import { authApi, meApi } from "@/lib/cloud";'],
    files: ["cloud/apps/web/lib/cloud/session.ts"],
  },
  runs: {
    use: "create/list/follow/cancel a run, git publish",
    import: ['import { runsApi } from "@/lib/cloud";'],
    files: ["cloud/apps/web/lib/cloud/runs.ts"],
  },
  composer: {
    use: "task / follow-up prompt with / skills and @ files",
    import: [
      'import { useComposerText } from "@/lib/hooks";',
      'import { Composer } from "@/components/composer";',
    ],
    files: [
      "cloud/apps/web/lib/hooks/useComposerText.ts",
      "cloud/apps/web/components/composer/Composer.tsx",
    ],
  },
  "project-picker": {
    use: "choose a connected GitHub repo",
    import: ['import { ProjectPicker } from "@/components/pickers";'],
    files: ["cloud/apps/web/components/pickers/ProjectPicker.tsx"],
  },
  "branch-picker": {
    use: "choose a repo branch",
    import: ['import { BranchPicker } from "@/components/pickers";'],
    files: ["cloud/apps/web/components/pickers/BranchPicker.tsx"],
  },
  "model-picker": {
    use: "choose the run model (needs useLlmSettings)",
    import: [
      'import { useLlmSettings } from "@/lib/hooks";',
      'import { ModelPicker } from "@/components/pickers";',
    ],
    files: ["cloud/apps/web/components/pickers/ModelPicker.tsx"],
  },
  "attach-menu": {
    use: "attach files, skills, MCP, permission, max turns",
    import: ['import { AttachMenu } from "@/components/pickers";'],
    files: ["cloud/apps/web/components/pickers/AttachMenu.tsx"],
  },
  picker: {
    use: "any new searchable list or field select",
    import: [
      'import { FieldSelect, SearchablePicker } from "@/components/ui";',
    ],
    files: [
      "cloud/apps/web/components/ui/Picker.tsx",
      "cloud/apps/web/components/ui/FieldSelect.tsx",
    ],
  },
  menu: {
    use: "anchored action menu",
    import: ['import { Menu, MenuItem, MenuSep, useDismiss } from "@/components/ui";'],
    files: [
      "cloud/apps/web/components/ui/Menu.tsx",
      "cloud/apps/web/components/ui/useDismiss.ts",
    ],
  },
  dialogs: {
    use: "confirm, prompt, toast (never window.alert/confirm/prompt)",
    import: [
      'import { ConfirmDialog, PromptDialog } from "@/components/ui";',
      'import { useToast } from "@/components/Toast";',
    ],
    files: [
      "cloud/apps/web/components/ui/ConfirmDialog.tsx",
      "cloud/apps/web/components/ui/PromptDialog.tsx",
      "cloud/apps/web/components/Toast.tsx",
    ],
  },
  http: {
    use: "new typed Cloud client method",
    import: ['import { getJson, postJson, putJson, patchJson, deleteJson } from "@/lib/cloud";'],
    files: ["cloud/apps/web/lib/cloud/http.ts"],
  },
} as const;

export type CapabilityId = keyof typeof capabilities;

export function capability(id: CapabilityId) {
  return capabilities[id];
}
