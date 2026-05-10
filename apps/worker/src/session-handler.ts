/**
 * Session handlers - list, get, delete sessions
 *
 * In production, use D1 for persistent storage.
 * This uses in-memory storage for simplicity.
 */
import type { Context } from 'hono';

interface Env {
  DB: D1Database;
}

// In-memory storage (replace with D1 in production)
const sessions = new Map<string, any>();

/**
 * GET /api/sessions
 * List all sessions
 */
export async function handleGetSessions(c: Context<{ Bindings: Env }>) {
  const sessionList = Array.from(sessions.entries()).map(([id, data]) => ({
    id,
    createdAt: data.createdAt,
    updatedAt: data.updatedAt,
    messageCount: data.messages?.length ?? 0,
  }));

  return c.json({ sessions: sessionList });
}

/**
 * GET /api/sessions/:id
 * Get session details and message history
 */
export async function handleGetSession(c: Context<{ Bindings: Env }>) {
  const { id } = c.req.param();
  const session = sessions.get(id);

  if (!session) {
    return c.json({ error: 'Session not found' }, 404);
  }

  return c.json({ session });
}

/**
 * DELETE /api/sessions/:id
 * Delete a session
 */
export async function handleDeleteSession(c: Context<{ Bindings: Env }>) {
  const { id } = c.req.param();

  if (!sessions.has(id)) {
    return c.json({ error: 'Session not found' }, 404);
  }

  sessions.delete(id);
  return c.json({ success: true });
}

/**
 * Save session data (called from agent handler)
 */
export function saveSession(id: string, data: any): void {
  sessions.set(id, {
    ...data,
    updatedAt: Date.now(),
  });
}

/**
 * Load session data (called from agent handler)
 */
export function loadSession(id: string): any | null {
  return sessions.get(id) ?? null;
}
