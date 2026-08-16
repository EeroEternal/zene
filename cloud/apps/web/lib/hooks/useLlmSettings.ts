"use client";

import { useCallback, useEffect, useState } from "react";
import { isLlmReady, llmApi } from "@/lib/cloud";
import { DEFAULT_MODEL_ID, loadSelectedModel, modelsForPicker, saveSelectedModel } from "@/lib/models";
import type { LlmSettingsView } from "@/lib/types";

export function useLlmSettings() {
  const [view, setView] = useState<LlmSettingsView | null>(null);
  const [selectedModel, setSelectedModel] = useState(DEFAULT_MODEL_ID);

  useEffect(() => {
    setSelectedModel(loadSelectedModel());
    let cancelled = false;
    llmApi
      .get()
      .then((next) => {
        if (cancelled) return;
        setView(next);
        const models = modelsForPicker(next);
        const current = loadSelectedModel();
        if (current === DEFAULT_MODEL_ID && next.defaultModel) {
          setSelectedModel(next.defaultModel);
          saveSelectedModel(next.defaultModel);
        } else if (current !== DEFAULT_MODEL_ID && models.length && !models.includes(current)) {
          const fallback = next.defaultModel || models[0] || DEFAULT_MODEL_ID;
          setSelectedModel(fallback);
          saveSelectedModel(fallback);
        }
      })
      .catch(() => {
        if (!cancelled) setView(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const selectModel = useCallback((id: string) => {
    setSelectedModel(id);
    saveSelectedModel(id);
  }, []);

  return {
    view,
    ready: isLlmReady(view),
    selectedModel,
    selectModel,
  };
}
