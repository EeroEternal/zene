/**
 * Zene Agent Cloud - Main Chat Page
 */
'use client';

import { useEffect } from 'react';
import { ChatWindow } from '@/components/ChatWindow';
import { SessionList } from '@/components/SessionList';
import { ConfigPanel } from '@/components/ConfigPanel';
import { ThemeToggle } from '@/components/ThemeToggle';
import { useStore } from '@/stores/useStore';

export default function Home() {
  const {
    theme,
    toggleTheme,
    loadSessions,
    loadProviderConfig,
  } = useStore();

  useEffect(() => {
    // Apply theme on mount
    document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

  useEffect(() => {
    // Load initial data
    loadSessions();
    loadProviderConfig();
  }, [loadSessions, loadProviderConfig]);

  return (
    <div style={styles.container}>
      {/* Sidebar */}
      <div style={styles.sidebar}>
        <div style={styles.sidebarHeader}>
          <h2 style={styles.logo}>Zene Agent Cloud</h2>
        </div>
        <SessionList />
        <ConfigPanel />
        <div style={styles.sidebarFooter}>
          <ThemeToggle />
        </div>
      </div>

      {/* Main Chat Area */}
      <div style={styles.main}>
        <ChatWindow />
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: 'flex',
    height: '100vh',
    overflow: 'hidden',
  },
  sidebar: {
    width: '300px',
    borderRight: '1px solid var(--border-color)',
    display: 'flex',
    flexDirection: 'column',
    background: 'var(--bg-secondary)',
  },
  sidebarHeader: {
    padding: '1rem',
    borderBottom: '1px solid var(--border-color)',
  },
  logo: {
    margin: 0,
    fontSize: '1.2rem',
    color: 'var(--text-primary)',
  },
  sidebarFooter: {
    padding: '1rem',
    borderTop: '1px solid var(--border-color)',
    display: 'flex',
    justifyContent: 'center',
  },
  main: {
    flex: 1,
    display: 'flex',
    flexDirection: 'column',
    overflow: 'hidden',
  },
};
