pub const INDEX_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Zene Web Agent</title>
  <style>
    :root {
      --bg: #f4efe6;
      --ink: #1f1a14;
      --muted: #6d6256;
      --panel: rgba(255, 252, 246, 0.92);
      --line: #d7ccbc;
      --accent: #0f6a5b;
      --accent-ink: #f7fffc;
      --warn: #8a4b12;
      --danger: #8b2430;
      --ok: #1f6b3a;
      --mono: "IBM Plex Mono", "SFMono-Regular", Menlo, Consolas, monospace;
      --sans: "Iowan Old Style", "Palatino Linotype", Palatino, Georgia, serif;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100vh;
      color: var(--ink);
      font-family: var(--sans);
      background:
        radial-gradient(circle at top left, rgba(15, 106, 91, 0.12), transparent 40%),
        linear-gradient(160deg, #f7f1e6 0%, #ebe2d2 48%, #f3efe7 100%);
    }
    main {
      max-width: 960px;
      margin: 0 auto;
      padding: 28px 18px 48px;
      display: grid;
      gap: 14px;
    }
    h1 {
      margin: 0;
      font-size: 1.7rem;
      letter-spacing: 0.01em;
    }
    .sub {
      color: var(--muted);
      margin: 4px 0 0;
      font-size: 0.95rem;
    }
    .card {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 10px;
      padding: 14px;
      backdrop-filter: blur(6px);
    }
    .row {
      display: flex;
      gap: 8px;
      flex-wrap: wrap;
      align-items: center;
    }
    label { font-size: 0.85rem; color: var(--muted); }
    input, textarea, button {
      font: inherit;
    }
    input, textarea {
      width: 100%;
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 10px 12px;
      background: #fffdf8;
      color: var(--ink);
    }
    textarea { min-height: 88px; resize: vertical; }
    button {
      border: 0;
      border-radius: 8px;
      padding: 9px 14px;
      background: var(--accent);
      color: var(--accent-ink);
      cursor: pointer;
    }
    button.secondary {
      background: transparent;
      color: var(--ink);
      border: 1px solid var(--line);
    }
    button.danger { background: var(--danger); color: #fff; }
    button:disabled { opacity: 0.55; cursor: not-allowed; }
    #log {
      min-height: 320px;
      max-height: 52vh;
      overflow: auto;
      font-family: var(--mono);
      font-size: 0.84rem;
      line-height: 1.45;
      white-space: pre-wrap;
      word-break: break-word;
    }
    .msg { margin: 0 0 10px; }
    .msg.user { color: var(--accent); }
    .msg.assistant { color: var(--ink); }
    .msg.thought { color: var(--muted); font-style: italic; }
    .msg.system { color: var(--warn); }
    .msg.tool { color: #355f7a; }
    .pending {
      border-left: 3px solid var(--warn);
      padding-left: 10px;
      margin: 10px 0;
    }
    .status {
      font-family: var(--mono);
      font-size: 0.78rem;
      color: var(--muted);
    }
    .ok { color: var(--ok); }
    .err { color: var(--danger); }
  </style>
</head>
<body>
  <main>
    <header>
      <h1>Zene</h1>
      <p class="sub">Headless agent over HTTP long-polling ACP gateway</p>
    </header>

    <section class="card">
      <div class="row" style="margin-bottom:10px">
        <div style="flex:1; min-width:220px">
          <label for="workspace">Workspace</label>
          <input id="workspace" placeholder="/absolute/path/to/project" />
        </div>
        <div style="width:220px">
          <label for="token">Token</label>
          <input id="token" placeholder="from launch URL hash" />
        </div>
      </div>
      <div class="row">
        <button id="btnStart">Start agent</button>
        <button id="btnCancel" class="secondary" disabled>Cancel turn</button>
        <span id="status" class="status">idle</span>
      </div>
    </section>

    <section class="card">
      <div id="log"></div>
    </section>

    <section class="card">
      <label for="prompt">Prompt</label>
      <textarea id="prompt" placeholder="Ask Zene to inspect or change code…"></textarea>
      <div class="row" style="margin-top:10px">
        <button id="btnSend" disabled>Send</button>
        <button id="btnClear" class="secondary">Clear log</button>
      </div>
    </section>
  </main>
  <script>
    const state = {
      token: "",
      agentId: null,
      sessionId: null,
      cursor: 0,
      nextRpcId: 1,
      polling: false,
      assistantBuf: "",
      thoughtBuf: "",
    };

    const el = (id) => document.getElementById(id);
    const logEl = el("log");
    const statusEl = el("status");

    function setStatus(text, cls) {
      statusEl.textContent = text;
      statusEl.className = "status " + (cls || "");
    }

    function appendLog(kind, text) {
      const div = document.createElement("div");
      div.className = "msg " + kind;
      div.textContent = text;
      logEl.appendChild(div);
      logEl.scrollTop = logEl.scrollHeight;
      return div;
    }

    function uuid() {
      if (crypto.randomUUID) return crypto.randomUUID();
      return "req-" + Math.random().toString(16).slice(2) + Date.now();
    }

    function tokenFromHash() {
      const hash = new URLSearchParams(location.hash.replace(/^#/, ""));
      return hash.get("token") || "";
    }

    async function api(path, options = {}) {
      const headers = Object.assign({ "content-type": "application/json" }, options.headers || {});
      if (state.token) headers["X-Zene-Token"] = state.token;
      const res = await fetch(path, Object.assign({}, options, { headers }));
      const text = await res.text();
      let body = null;
      try { body = text ? JSON.parse(text) : null; } catch (_) { body = { raw: text }; }
      if (!res.ok) {
        const msg = body && (body.message || body.error) || res.statusText;
        throw new Error(msg);
      }
      return body;
    }

    async function postMessages(messages) {
      return api(`/api/v1/agents/${state.agentId}/messages`, {
        method: "POST",
        body: JSON.stringify({ requestId: uuid(), messages }),
      });
    }

    function rpcRequest(method, params) {
      const id = state.nextRpcId++;
      return { jsonrpc: "2.0", id, method, params };
    }

    function renderChunk(kind, text) {
      if (!text) return;
      if (kind === "assistant") {
        state.assistantBuf += text;
        let node = logEl.querySelector(".live-assistant");
        if (!node) {
          node = appendLog("assistant", "");
          node.classList.add("live-assistant");
        }
        node.textContent = "assistant: " + state.assistantBuf;
      } else if (kind === "thought") {
        state.thoughtBuf += text;
        let node = logEl.querySelector(".live-thought");
        if (!node) {
          node = appendLog("thought", "");
          node.classList.add("live-thought");
        }
        node.textContent = "thought: " + state.thoughtBuf;
      }
    }

    function finalizeStreams() {
      const a = logEl.querySelector(".live-assistant");
      if (a) a.classList.remove("live-assistant");
      const t = logEl.querySelector(".live-thought");
      if (t) t.classList.remove("live-thought");
      state.assistantBuf = "";
      state.thoughtBuf = "";
    }

    function handlePayload(payload) {
      if (!payload || typeof payload !== "object") return;

      if (payload.type === "gateway.system") {
        appendLog("system", `[gateway:${payload.kind}] ${payload.message || ""}`);
        return;
      }

      if (payload.method === "session/update") {
        const update = payload.params && payload.params.update || {};
        const sessionId = payload.params && payload.params.sessionId;
        if (sessionId) state.sessionId = sessionId;
        if (update.sessionUpdate === "agent_message_chunk") {
          renderChunk("assistant", (update.content && update.content.text) || "");
        } else if (update.sessionUpdate === "agent_thought_chunk") {
          renderChunk("thought", (update.content && update.content.text) || "");
        } else if (update.sessionUpdate === "tool_call" || update.sessionUpdate === "tool_call_update") {
          finalizeStreams();
          appendLog("tool", `${update.sessionUpdate}: ${update.title || update.toolCallId || "tool"}`);
        } else {
          appendLog("system", JSON.stringify(update));
        }
        return;
      }

      if (payload.method === "session/request_permission") {
        finalizeStreams();
        const params = payload.params || {};
        const box = document.createElement("div");
        box.className = "pending";
        const title = document.createElement("div");
        title.textContent = `Permission required: ${(params.toolCall && (params.toolCall.title || params.toolCall.toolCallId)) || "tool"}`;
        box.appendChild(title);
        const options = (params.options || []);
        const row = document.createElement("div");
        row.className = "row";
        row.style.marginTop = "8px";
        for (const opt of options) {
          const btn = document.createElement("button");
          btn.textContent = opt.name || opt.optionId;
          if ((opt.kind || "").includes("reject") || /deny|reject|no/i.test(opt.optionId || "")) {
            btn.className = "danger";
          }
          btn.onclick = async () => {
            row.querySelectorAll("button").forEach((b) => b.disabled = true);
            try {
              await postMessages([{
                jsonrpc: "2.0",
                id: payload.id,
                result: { outcome: { outcome: "selected", optionId: opt.optionId } },
              }]);
              appendLog("system", `permission -> ${opt.optionId}`);
            } catch (err) {
              appendLog("system", "permission reply failed: " + err.message);
            }
          };
          row.appendChild(btn);
        }
        if (!options.length) {
          const allow = document.createElement("button");
          allow.textContent = "allow-once";
          allow.onclick = async () => {
            await postMessages([{
              jsonrpc: "2.0",
              id: payload.id,
              result: { outcome: { outcome: "selected", optionId: "allow-once" } },
            }]);
          };
          const deny = document.createElement("button");
          deny.className = "danger";
          deny.textContent = "reject";
          deny.onclick = async () => {
            await postMessages([{
              jsonrpc: "2.0",
              id: payload.id,
              result: { outcome: { outcome: "selected", optionId: "reject" } },
            }]);
          };
          row.appendChild(allow);
          row.appendChild(deny);
        }
        box.appendChild(row);
        logEl.appendChild(box);
        logEl.scrollTop = logEl.scrollHeight;
        return;
      }

      if (Object.prototype.hasOwnProperty.call(payload, "result") || Object.prototype.hasOwnProperty.call(payload, "error")) {
        if (payload.result && payload.result.sessionId) {
          state.sessionId = payload.result.sessionId;
          appendLog("system", `session ready: ${state.sessionId}`);
        } else if (payload.error) {
          appendLog("system", `rpc error: ${JSON.stringify(payload.error)}`);
        }
        return;
      }

      appendLog("system", JSON.stringify(payload));
    }

    async function pollLoop() {
      if (state.polling || !state.agentId) return;
      state.polling = true;
      while (state.agentId) {
        try {
          const qs = new URLSearchParams({
            cursor: String(state.cursor),
            waitMs: "25000",
            limit: "100",
          });
          const body = await api(`/api/v1/agents/${state.agentId}/events?${qs}`);
          for (const event of body.events || []) {
            state.cursor = event.cursor;
            handlePayload(event.payload);
          }
          if (body.hasMore) continue;
        } catch (err) {
          setStatus("poll error: " + err.message, "err");
          await new Promise((r) => setTimeout(r, 1500));
        }
      }
      state.polling = false;
    }

    async function startAgent() {
      state.token = el("token").value.trim() || tokenFromHash();
      el("token").value = state.token;
      const workspace = el("workspace").value.trim();
      if (!state.token) {
        setStatus("token required", "err");
        return;
      }
      if (!workspace) {
        setStatus("workspace required", "err");
        return;
      }
      setStatus("starting…");
      const created = await api("/api/v1/agents", {
        method: "POST",
        body: JSON.stringify({ requestId: uuid(), workspace }),
      });
      state.agentId = created.agent.agentId;
      state.cursor = 0;
      appendLog("system", `agent ${state.agentId} @ ${created.agent.workspace}`);
      pollLoop();

      await postMessages([
        rpcRequest("initialize", {
          protocolVersion: 1,
          clientCapabilities: {
            fs: { readTextFile: true, writeTextFile: true },
            terminal: true,
          },
          clientInfo: { name: "zene-web", version: "0.1.0" },
        }),
      ]);
      await postMessages([
        rpcRequest("session/new", {
          cwd: workspace,
          mcpServers: [],
        }),
      ]);
      el("btnSend").disabled = false;
      el("btnCancel").disabled = false;
      setStatus("ready", "ok");
    }

    async function sendPrompt() {
      const text = el("prompt").value.trim();
      if (!text || !state.sessionId) return;
      finalizeStreams();
      appendLog("user", "user: " + text);
      el("prompt").value = "";
      await postMessages([
        rpcRequest("session/prompt", {
          sessionId: state.sessionId,
          prompt: [{ type: "text", text }],
        }),
      ]);
    }

    async function cancelTurn() {
      if (!state.sessionId) return;
      await postMessages([
        { jsonrpc: "2.0", method: "session/cancel", params: { sessionId: state.sessionId } },
      ]);
      appendLog("system", "cancel sent");
    }

    el("btnStart").onclick = () => startAgent().catch((err) => setStatus(err.message, "err"));
    el("btnSend").onclick = () => sendPrompt().catch((err) => setStatus(err.message, "err"));
    el("btnCancel").onclick = () => cancelTurn().catch((err) => setStatus(err.message, "err"));
    el("btnClear").onclick = () => { logEl.innerHTML = ""; };
    el("token").value = tokenFromHash();
    setStatus(el("token").value ? "token loaded" : "waiting for token");
  </script>
</body>
</html>
"#;
