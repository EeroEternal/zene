/**
 * SessionList - Sidebar with session list
 */
'use client';

import { useState } from 'react';
import { useStore } from '@/stores/useStore';

export function SessionList() {
  const {
    sessions,
    currentSessionId,
    createSession,
    switchSession,
    deleteSession,
    loadSessions,
  } = useStore();

  const [isLoading, setIsLoading] = useState(false);

  const handleNewSession = () => {
    createSession();
  };

  const handleSwitch = (id: string) => {
    if (id === currentSessionId) return;
    switchSession(id);
  };

  const handleDelete = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    await deleteSession(id);
  };

  return (
    <div style={styles.container}>
      <button onClick={handleNewSession} style={styles.newButton}>
        + New Session
      </button>

      <div style={styles.list}>
        {sessions.map((session) => (
          <div
            key={session.id}
            onClick={() => handleSwitch(session.id)}
            style={{
              ...styles.sessionItem,
              background: session.id === currentSessionId ? 'var(--bg-tertiary)' : 'transparent',
            }}
          >
            <span style={styles.sessionTitle}>
              {session.title || 'New Session'}
            </span>
            <button
              onClick={(e) => handleDelete(e, session.id)}
              style={styles.deleteButton}
              title="Delete session"
            >
              ×
            </button>
          </div>
        ))}
      </div>

      <button onClick={() => loadSessions()} style={styles.refreshButton}>
        Refresh
      </button>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
  },
  newButton: {
    padding: '0.75rem 1rem',
    background: 'var(--accent)',
    color: '#fff',
    border: 'none',
    borderRadius: '0.5rem',
    cursor: 'pointer',
    fontSize: '0.9rem',
    fontWeight: 600,
    margin: '0.5rem',
  },
  list: {
    flex: 1,
    overflowY: 'auto',
    padding: '0.5rem',
  },
  sessionItem: {
    padding: '0.75rem',
    borderRadius: '0.5rem',
    cursor: 'pointer',
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: '0.25rem',
    transition: 'background 0.2s',
  },
  sessionTitle: {
    fontSize: '0.9rem',
    color: 'var(--text-primary)',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    flex: 1,
  },
  deleteButton: {
    background: 'none',
    border: 'none',
    color: 'var(--text-secondary)',
    cursor: 'pointer',
    fontSize: '1.2rem',
    padding: '0 0.25rem',
    lineHeight: 1,
  },
  refreshButton: {
    padding: '0.5rem',
    background: 'transparent',
    border: '1px solid var(--border-color)',
    borderRadius: '0.5rem',
    cursor: 'pointer',
    color: 'var(--text-secondary)',
    fontSize: '0.8rem',
    margin: '0.5rem',
  },
};
