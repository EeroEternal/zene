/**
 * Zustand store for Zene Agent Cloud
 *
 * Manages:
 * - Sessions (create, switch, delete)
 * - Messages (per session)
 * - Streaming state
 * - Provider configuration
 * - Theme
 */
import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import { v4 as uuidv4 } from 'uuid';

// ─── Types ───────────────────────────────────────────────────────────────────

export interface Message {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

export interface Session {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
}

export interface ProviderConfig {
  provider: string;
  apiKey?: string;
  baseUrl?: string;
  gatewayUrl?: string;
  defaultModel?: string;
}

// ─── Store ───────────────────────────────────────────────────────────────────

interface Store {
  // Sessions
  sessions: Session[];
  currentSessionId: string | null;
  createSession: () => void;
  switchSession: (id: string) => void;
  deleteSession: (id: string) => Promise<void>;
  loadSessions: () => Promise<void>;

  // Messages
  messages: Record<string, Message[]>; // sessionId → messages
  addMessage: (sessionId: string, message: Message) => void;
  updateMessage: (sessionId: string, messageId: string, updater: (msg: Message) => Message) => void;
  loadMessages: (sessionId: string) => Promise<void>;

  // Streaming
  isStreaming: boolean;
  streamedText: string;
  startStreaming: (sessionId: string, prompt: string) => Promise<void>;
  stopStreaming: () => void;

  // Config
  providerConfig: ProviderConfig;
  setProviderConfig: (config: ProviderConfig) => Promise<void>;
  loadProviderConfig: () => Promise<void>;

  // Theme
  theme: 'dark' | 'light';
  toggleTheme: () => void;
}

// ─── API Base URL ───────────────────────────────────────────────────────────

const API_BASE = process.env.NEXT_PUBLIC_API_URL || '';

// ─── Store Implementation ─────────────────────────────────────────────────

export const useStore = create<Store>()(
  persist(
    (set, get) => ({
      // ── Sessions ───────────────────────────────────

      sessions: [],
      currentSessionId: null,

  createSession: () => {
    const id = uuidv4();
    const session: Session = {
      id,
      title: 'New Session',
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    set((state) => ({
      sessions: [session, ...state.sessions],
      currentSessionId: id,
      messages: { ...state.messages, [id]: [] },
    }));
  },

  switchSession: (id: string) => {
    set({ currentSessionId: id });
    // Load messages if not already loaded
    const { messages } = get();
    if (!messages[id]) {
      get().loadMessages(id);
    }
  },

  deleteSession: async (id: string) => {
    try {
      const res = await fetch(`${API_BASE}/api/sessions/${id}`, {
        method: 'DELETE',
      });
      if (!res.ok) throw new Error('Failed to delete session');
    } catch (error) {
      console.error('[zene] Failed to delete session:', error);
    }

    set((state) => {
      const newMessages = { ...state.messages };
      delete newMessages[id];
      return {
        sessions: state.sessions.filter((s) => s.id !== id),
        currentSessionId: state.currentSessionId === id ? null : state.currentSessionId,
        messages: newMessages,
      };
    });
  },

  loadSessions: async () => {
    try {
      const res = await fetch(`${API_BASE}/api/sessions`);
      if (!res.ok) throw new Error('Failed to load sessions');
      const data = await res.json();
      set({ sessions: data.sessions || [] });
    } catch (error) {
      console.error('[zene] Failed to load sessions:', error);
    }
  },

  // ── Messages ───────────────────────────────────

  messages: {},

  addMessage: (sessionId: string, message: Message) => {
    set((state) => ({
      messages: {
        ...state.messages,
        [sessionId]: [...(state.messages[sessionId] || []), message],
      },
    }));
  },

  updateMessage: (sessionId: string, messageId: string, updater: (msg: Message) => Message) => {
    set((state) => {
      const sessionMessages = state.messages[sessionId] || [];
      return {
        messages: {
          ...state.messages,
          [sessionId]: sessionMessages.map((msg) =>
            msg.id === messageId ? updater(msg) : msg
          ),
        },
      };
    });
  },

  loadMessages: async (sessionId: string) => {
    try {
      const res = await fetch(`${API_BASE}/api/sessions/${sessionId}`);
      if (!res.ok) throw new Error('Failed to load session');
      const data = await res.json();
      set((state) => ({
        messages: {
          ...state.messages,
          [sessionId]: data.messages || [],
        },
      }));
    } catch (error) {
      console.error('[zene] Failed to load messages:', error);
    }
  },

  // ── Streaming ──────────────────────────────────

  isStreaming: false,
  streamedText: '',

  startStreaming: async (sessionId: string, prompt: string) => {
    const { addMessage, updateMessage } = get();
    const { providerConfig } = get();

    // Add user message
    const userMsg: Message = {
      id: uuidv4(),
      role: 'user',
      content: prompt,
      timestamp: Date.now(),
    };
    addMessage(sessionId, userMsg);

    // Create assistant message (will be updated as we stream)
    const assistantMsg: Message = {
      id: uuidv4(),
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
      isStreaming: true,
    };
    addMessage(sessionId, assistantMsg);
    set({ isStreaming: true, streamedText: '' });

    try {
      const res = await fetch(`${API_BASE}/api/agent/chat/prompt`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          sessionId,
          prompt,
          model: providerConfig.defaultModel,
        }),
      });

      if (!res.ok) {
        let message = 'Failed to start streaming';
        try {
          const data = await res.json();
          message = data.error || message;
        } catch {
          message = await res.text().catch(() => message);
        }
        throw new Error(message);
      }

      const reader = res.body?.getReader();
      if (!reader) throw new Error('No reader available');

      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          if (!line.startsWith('data: ')) continue;
          const data = line.slice(6);
          if (data === '[DONE]') continue;

          let event: any;
          try {
            event = JSON.parse(data);
          } catch {
            throw new Error('Failed to parse streaming response');
          }

          if (event.type === 'text_delta') {
            const { streamedText } = get();
            const newText = streamedText + event.text;
            set({ streamedText: newText });
            updateMessage(sessionId, assistantMsg.id, (msg) => ({
              ...msg,
              content: newText,
            }));
          } else if (event.type === 'error') {
            throw new Error(event.error || 'Streaming error');
          }
        }
      }

      // Streaming complete
      set((state) => ({
        isStreaming: false,
        streamedText: '',
        messages: {
          ...state.messages,
          [sessionId]: (state.messages[sessionId] || []).map((msg) =>
            msg.id === assistantMsg.id ? { ...msg, isStreaming: false } : msg
          ),
        },
      }));
    } catch (error) {
      console.error('[zene] Streaming error:', error);
      const message = error instanceof Error ? error.message : String(error);
      set((state) => ({
        isStreaming: false,
        streamedText: '',
        messages: {
          ...state.messages,
          [sessionId]: (state.messages[sessionId] || []).map((msg) =>
            msg.id === assistantMsg.id
              ? { ...msg, content: message, isStreaming: false }
              : msg
          ),
        },
      }));
    }
  },

  stopStreaming: () => {
    set({ isStreaming: false });
  },

  // ── Config ─────────────────────────────────────

  providerConfig: {
    provider: 'deepseek',
    defaultModel: 'deepseek/deepseek-chat',
  },

  setProviderConfig: async (config: ProviderConfig) => {
    set({ providerConfig: config });
    try {
      await fetch(`${API_BASE}/api/config/provider`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config),
      });
    } catch (error) {
      console.error('[zene] Failed to save config:', error);
    }
  },

  loadProviderConfig: async () => {
    try {
      const res = await fetch(`${API_BASE}/api/config/provider`);
      if (!res.ok) throw new Error('Failed to load config');
      const data = await res.json();
      const config = data.config || {};
      // 如果后端返回空对象（worker 重启后），保留当前值（localStorage 恢复的或默认值）
      if (Object.keys(config).length === 0) return;
      set({ providerConfig: config });
    } catch (error) {
      console.error('[zene] Failed to load config:', error);
    }
  },

      // ── Theme ──────────────────────────────────────

      theme: 'light' as 'dark' | 'light',

      toggleTheme: () => {
        set((state) => {
          const newTheme = state.theme === 'dark' ? 'light' : 'dark';
          if (typeof window !== 'undefined') {
            document.documentElement.setAttribute('data-theme', newTheme);
          }
          return { theme: newTheme };
        });
      },
    }),
    {
      name: 'zene-store',
      partialize: (state) => ({ theme: state.theme, providerConfig: state.providerConfig }),
    }
  )
);
