/**
 * ThemeToggle - Dark/light theme toggle
 */
'use client';

import { useStore } from '@/stores/useStore';

export function ThemeToggle() {
  const { theme, toggleTheme } = useStore();

  return (
    <button onClick={toggleTheme} style={styles.button} title={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`}>
      {theme === 'dark' ? (
        <span style={styles.icon}>☀️</span>
      ) : (
        <span style={styles.icon}>🌙</span>
      )}
    </button>
  );
}

const styles: Record<string, React.CSSProperties> = {
  button: {
    background: 'transparent',
    border: '1px solid var(--border-color)',
    borderRadius: '0.5rem',
    cursor: 'pointer',
    padding: '0.5rem',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: '40px',
    height: '40px',
  },
  icon: {
    fontSize: '1.2rem',
  },
};
