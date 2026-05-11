/**
 * Zene Agent Cloud - Cloudflare Worker
 *
 * API Routes:
 *  POST   /api/agent/:agentName/prompt  - Send prompt to agent (SSE)
 *  GET    /api/sessions              - List all sessions
 *  GET    /api/sessions/:id           - Get session details
 *  DELETE /api/sessions/:id           - Delete a session
 *  POST   /api/config/provider       - Set provider config
 *  GET    /api/config/provider       - Get provider config
 */
import { Hono } from 'hono';
import { cors } from 'hono/cors';
import { handleAgentPrompt } from './agent-handler';
import { handleGetSessions, handleGetSession, handleDeleteSession } from './session-handler';
import { handleGetConfig, handleSetConfig } from './config-handler';

interface Env {
  AI: any;
  DB: D1Database;
  AI_GATEWAY_URL?: string;
}

const app = new Hono<{ Bindings: Env }>();

// CORS for frontend access
app.use('/api/*', cors({
  origin: '*',
  allowMethods: ['GET', 'POST', 'DELETE', 'OPTIONS'],
  allowHeaders: ['Content-Type', 'Authorization'],
}));

// Health check
app.get('/', (c) => c.json({ status: 'ok', name: 'Zene Agent Cloud Worker' }));

// Agent prompts (SSE streaming)
app.post('/api/agent/:agentName/prompt', handleAgentPrompt);

// Session management
app.get('/api/sessions', handleGetSessions);
app.get('/api/sessions/:id', handleGetSession);
app.delete('/api/sessions/:id', handleDeleteSession);

// Configuration
app.post('/api/config/provider', handleSetConfig);
app.get('/api/config/provider', handleGetConfig);

export default {
  fetch: app.fetch.bind(app),
};
