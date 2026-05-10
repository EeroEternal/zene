/**
 * StreamingText - Shows typing indicator while streaming
 */
'use client';

import { useStore } from '@/stores/useStore';

export function StreamingText() {
  const streamedText = useStore((state) => state.streamedText);
  const isStreaming = useStore((state) => state.isStreaming);

  if (!isStreaming) return null;

  return (
    <div style={styles.container}>
      <div
        style={{
          ...styles.bubble,
          background: 'var(--agent-msg-bg)',
          color: 'var(--agent-msg-text)',
        }}
      >
        {streamedText ? (
          <div className="markdown-body">
            {streamedText}
          </div>
        ) : (
          <span style={styles.dots}>
            <span style={styles.dot}>.</span>
            <span style={styles.dot}>.</span>
            <span style={styles.dot}>.</span>
          </span>
        )}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: 'flex',
    marginBottom: '1rem',
    maxWidth: '80%',
  },
  bubble: {
    padding: '0.75rem 1rem',
    borderRadius: '1rem',
    lineHeight: 1.5,
    wordBreak: 'break-word',
  },
  dots: {
    display: 'inline-flex',
    gap: '0.2rem',
  },
  dot: {
    animation: 'blink 1.4s infinite both',
  },
};
