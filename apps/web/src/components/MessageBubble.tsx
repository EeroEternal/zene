/**
 * MessageBubble - Renders a single message (user or assistant)
 */
'use client';

import ReactMarkdown from 'react-markdown';
import rehypeHighlight from 'rehype-highlight';
import { Message } from '@/stores/useStore';

interface Props {
  message: Message;
}

export function MessageBubble({ message }: Props) {
  const isUser = message.role === 'user';

  return (
    <div
      style={{
        ...styles.container,
        justifyContent: isUser ? 'flex-end' : 'flex-start',
      }}
    >
      <div
        style={{
          ...styles.bubble,
          background: isUser ? 'var(--user-msg-bg)' : 'var(--agent-msg-bg)',
          color: isUser ? 'var(--user-msg-text)' : 'var(--agent-msg-text)',
        }}
      >
        {message.isStreaming && !message.content ? (
          <StreamingDots />
        ) : (
          <div className="markdown-body">
            <ReactMarkdown rehypePlugins={[rehypeHighlight]}>
              {message.content}
            </ReactMarkdown>
          </div>
        )}
      </div>
    </div>
  );
}

function StreamingDots() {
  return (
    <span style={styles.dots}>
      <span style={styles.dot}>.</span>
      <span style={styles.dot}>.</span>
      <span style={styles.dot}>.</span>
    </span>
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
    borderTopRightRadius: undefined,
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
