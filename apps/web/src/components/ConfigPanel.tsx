/**
 * ConfigPanel - LLM provider configuration
 */
'use client';

import { useState } from 'react';
import { useStore } from '@/stores/useStore';

export function ConfigPanel() {
  const { providerConfig, setProviderConfig } = useStore();
  const [isOpen, setIsOpen] = useState(false);
  const [form, setForm] = useState({
    provider: providerConfig.provider || 'anthropic',
    apiKey: '',
    baseUrl: providerConfig.baseUrl || '',
    gatewayUrl: providerConfig.gatewayUrl || '',
    defaultModel: providerConfig.defaultModel || '',
  });

  const handleSave = async () => {
    await setProviderConfig({
      ...providerConfig,
      provider: form.provider,
      ...(form.apiKey && { apiKey: form.apiKey }),
      ...(form.baseUrl && { baseUrl: form.baseUrl }),
      ...(form.gatewayUrl && { gatewayUrl: form.gatewayUrl }),
      ...(form.defaultModel && { defaultModel: form.defaultModel }),
    });
    setIsOpen(false);
  };

  return (
    <div style={styles.container}>
      <button onClick={() => setIsOpen(!isOpen)} style={styles.toggleButton}>
        ⚙️ Settings {isOpen ? '▲' : '▼'}
      </button>

      {isOpen && (
        <div style={styles.panel}>
          <h4 style={styles.title}>LLM Provider Settings</h4>

          <label style={styles.label}>
            Provider:
            <select
              value={form.provider}
              onChange={(e) => setForm({ ...form, provider: e.target.value })}
              style={styles.input}
            >
              <option value="anthropic">Anthropic</option>
              <option value="openai">OpenAI</option>
              <option value="deepseek">DeepSeek</option>
              <option value="custom">Custom</option>
            </select>
          </label>

          <label style={styles.label}>
            API Key:
            <input
              type="password"
              value={form.apiKey}
              onChange={(e) => setForm({ ...form, apiKey: e.target.value })}
              placeholder="Enter API key"
              style={styles.input}
            />
          </label>

          <label style={styles.label}>
            Base URL (optional):
            <input
              type="text"
              value={form.baseUrl}
              onChange={(e) => setForm({ ...form, baseUrl: e.target.value })}
              placeholder="https://api.example.com"
              style={styles.input}
            />
          </label>

          <label style={styles.label}>
            Default Model:
            <input
              type="text"
              value={form.defaultModel}
              onChange={(e) => setForm({ ...form, defaultModel: e.target.value })}
              placeholder="e.g., claude-sonnet-4-6"
              style={styles.input}
            />
          </label>

          <label style={styles.label}>
            AI Gateway URL (optional):
            <input
              type="text"
              value={form.gatewayUrl}
              onChange={(e) => setForm({ ...form, gatewayUrl: e.target.value })}
              placeholder="https://gateway.ai.cloudflare.com/..."
              style={styles.input}
            />
          </label>

          <button onClick={handleSave} style={styles.saveButton}>
            Save Configuration
          </button>
        </div>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    borderTop: '1px solid var(--border-color)',
  },
  toggleButton: {
    width: '100%',
    padding: '0.75rem',
    background: 'transparent',
    border: 'none',
    cursor: 'pointer',
    color: 'var(--text-secondary)',
    fontSize: '0.9rem',
    textAlign: 'left',
  },
  panel: {
    padding: '0.5rem 1rem 1rem',
  },
  title: {
    margin: '0 0 0.5rem 0',
    fontSize: '0.9rem',
    color: 'var(--text-primary)',
  },
  label: {
    display: 'block',
    marginBottom: '0.5rem',
    fontSize: '0.8rem',
    color: 'var(--text-secondary)',
  },
  input: {
    display: 'block',
    width: '100%',
    padding: '0.5rem',
    marginTop: '0.25rem',
    border: '1px solid var(--border-color)',
    borderRadius: '0.25rem',
    background: 'var(--bg-primary)',
    color: 'var(--text-primary)',
    fontSize: '0.9rem',
  },
  saveButton: {
    padding: '0.5rem 1rem',
    background: 'var(--accent)',
    color: '#fff',
    border: 'none',
    borderRadius: '0.25rem',
    cursor: 'pointer',
    fontSize: '0.9rem',
    width: '100%',
    marginTop: '0.5rem',
  },
};
