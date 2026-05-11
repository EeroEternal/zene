/**
 * ChatWindow - Main chat area with message list and input
 */
'use client';

import { useState, useRef, useEffect } from 'react';
import { useStore } from '@/stores/useStore';
import { MessageBubble } from './MessageBubble';
import { StreamingText } from './StreamingText';

export function ChatWindow() {
  const {
    currentSessionId,
    sessions,
    messages,
    isStreaming,
    startStreaming,
  } = useStore();

  const [input, setInput] = useState('');
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-scroll to bottom
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, currentSessionId, isStreaming]);

  // Auto-focus input when session changes (e.g., after creating new session)
  useEffect(() => {
    if (currentSessionId) {
      inputRef.current?.focus();
    }
  }, [currentSessionId]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!input.trim() || !currentSessionId || isStreaming) return;

    const prompt = input.trim();
    setInput('');

    try {
      await startStreaming(currentSessionId, prompt);
    } catch (error) {
      console.error('[zene] Failed to send message:', error);
    }
  };

  const currentSession = sessions.find((s) => s.id === currentSessionId);
  const sessionMessages = currentSessionId ? messages[currentSessionId] || [] : [];

  if (!currentSessionId) {
    return (
      <div style={styles.empty}>
        <h2>Welcome to Zene Agent Cloud</h2>
        <p>Select a session or create a new one to start chatting.</p>
      </div>
    );
  }

  return (
    <div style={styles.container}>
      {/* Header */}
      <div style={styles.header}>
        <h3 style={styles.headerTitle}>
          {currentSession?.title || 'New Session'}
        </h3>
      </div>

      {/* Messages */}
      <div style={styles.messagesContainer}>
        {sessionMessages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} />
        ))}
        {isStreaming && <StreamingText />}
        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      <form onSubmit={handleSubmit} style={styles.inputForm}>
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Type your message..."
          style={styles.input}
          disabled={isStreaming}
        />
        <button
          type="submit"
          style={styles.sendButton}
          disabled={isStreaming || !input.trim()}
        >
          {isStreaming ? '...' : 'Send'}
        </button>
      </form>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
    overflow: 'hidden',
  },
  empty: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    height: '100%',
    color: 'var(--text-secondary)',
    textAlign: 'center',
    padding: '2rem',
  },
  header: {
    padding: '1rem',
    borderBottom: '1px solid var(--border-color)',
  },
  headerTitle: {
    margin: 0,
    fontSize: '1.1rem',
    color: 'var(--text-primary)',
  },
  messagesContainer: {
    flex: 1,
    overflowY: 'auto',
    padding: '1rem',
  },
  inputForm: {
    display: 'flex',
    padding: '1rem',
    borderTop: '1px solid var(--border-color)',
    gap: '0.5rem',
  },
  input: {
    flex: 1,
    padding: '0.75rem 1rem',
    border: '1px solid var(--border-color)',
    borderRadius: '0.5rem',
    background: 'var(--bg-secondary)',
    color: 'var(--text-primary)',
    fontSize: '1rem',
    outline: 'none',
  },
  sendButton: {
    padding: '0.75rem 1.5rem',
    background: 'var(--accent)',
    color: '#fff',
    border: 'none',
    borderRadius: '0.5rem',
    cursor: 'pointer',
    fontSize: '1rem',
    fontWeight: 600,
  },
};
