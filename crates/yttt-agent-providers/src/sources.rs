pub const PI_EXTENSION_SOURCE: &str = r#"const PREFIX = "yttt-agent-v1:";
const MAX_TEXT = 2048;

function clip(value) {
  if (typeof value !== "string") return undefined;
  const normalized = value.trim().replace(/\s+/g, " ");
  if (!normalized) return undefined;
  return normalized.length <= MAX_TEXT ? normalized : `${normalized.slice(0, MAX_TEXT - 1)}…`;
}

function toolDetail(input) {
  if (!input || typeof input !== "object") return clip(input);
  for (const key of ["path", "command", "pattern", "query", "task", "prompt", "description", "url", "i"]) {
    const value = clip(input[key]);
    if (value) return value;
  }
  return undefined;
}

function send(name, payload, ctx) {
  const enriched = {
    ...payload,
    sessionId: ctx?.sessionManager?.getSessionId?.(),
    sessionFile: ctx?.sessionManager?.getSessionFile?.(),
    model: ctx?.model?.id,
  };
  const endpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
  const hookToken = process.env.YTTT_AGENT_HOOK_TOKEN;
  const scope = process.env.YTTT_AGENT_HOOK_SCOPE;
  if (endpoint && hookToken && scope) {
    void fetch(`${endpoint}/hook/pi`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-yttt-agent-hook-token": hookToken,
        "x-yttt-agent-hook-scope": scope,
      },
      body: JSON.stringify({ event: name, payload: enriched }),
      signal: AbortSignal.timeout(2000),
    }).catch(() => {});
    return;
  }
  const instanceId = process.env.YTTT_AGENT_INSTANCE_ID;
  const token = process.env.YTTT_AGENT_TOKEN;
  const generation = Number(process.env.YTTT_AGENT_GENERATION || "0");
  if (!instanceId || !token || !Number.isSafeInteger(generation) || generation <= 0) return;
  const frame = { protocol: 1, instanceId, token, generation, event: name, payload: enriched };
  const encoded = Buffer.from(JSON.stringify(frame), "utf8").toString("base64url");
  process.stdout.write(`\u001b]2;${PREFIX}${encoded}\u0007`);
}

export default function (pi) {
  const trackContext = (_event, ctx) => send("session_start", {}, ctx);
  pi.on("session_start", trackContext);
  pi.on("session_switch", trackContext);
  pi.on("before_agent_start", (event, ctx) =>
    send("before_agent_start", { prompt: clip(event?.prompt) }, ctx));
  pi.on("agent_start", (_event, ctx) => send("agent_start", {}, ctx));
  pi.on("agent_end", (event, ctx) =>
    send("agent_end", { willContinue: event?.willContinue === true }, ctx));
  pi.on("tool_call", (event, ctx) => send("tool_call", {
    toolCallId: event?.toolCallId,
    toolName: event?.toolName,
    detail: toolDetail(event?.input),
  }, ctx));
  pi.on("tool_approval_requested", (event, ctx) => send("tool_approval_requested", {
    toolCallId: event?.toolCallId,
    toolName: event?.toolName,
    reason: clip(event?.reason),
  }, ctx));
  pi.on("tool_approval_resolved", (event, ctx) => send("tool_approval_resolved", {
    toolCallId: event?.toolCallId,
    toolName: event?.toolName,
    approved: event?.approved,
    reason: clip(event?.reason),
  }, ctx));
  pi.on("tool_execution_start", (event, ctx) => send("tool_execution_start", {
    toolCallId: event?.toolCallId,
    toolName: event?.toolName,
    detail: clip(event?.intent) || toolDetail(event?.args),
  }, ctx));
  pi.on("tool_execution_end", (event, ctx) => send("tool_execution_end", {
    toolCallId: event?.toolCallId,
    toolName: event?.toolName,
    isError: event?.isError,
  }, ctx));
}
"#;
pub const OPENCODE_PLUGIN_SOURCE: &str = r#"const endpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
const token = process.env.YTTT_AGENT_HOOK_TOKEN;
const scope = process.env.YTTT_AGENT_HOOK_SCOPE;
const roles = new Map();
const toolStates = new Map();

function boundedSet(map, key, value, limit = 256) {
  if (!key) return;
  map.delete(key);
  map.set(key, value);
  while (map.size > limit) map.delete(map.keys().next().value);
}

function post(event, payload = {}) {
  if (!endpoint || !token || !scope) return;
  void fetch(`${endpoint}/hook/opencode`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-yttt-agent-hook-token": token,
      "x-yttt-agent-hook-scope": scope,
    },
    body: JSON.stringify({ event, payload }),
    signal: AbortSignal.timeout(2000),
  }).catch(() => {});
}

export const YtttAgentStatusPlugin = async (_ctx) => ({
  event: async ({ event }) => {
    if (!event?.type) return;
    const properties = event.properties || {};
    if (event.type === "session.created") {
      post("session_start", properties.info || properties);
      return;
    }
    if (event.type === "session.updated") {
      post("session_updated", properties.info || properties);
      return;
    }
    if (event.type === "session.status") {
      const status = properties.status?.type || properties.status;
      if (status === "busy" || status === "retry") post("session_busy", properties);
      if (status === "idle") post("session_idle", properties);
      return;
    }
    if (event.type === "session.idle") { post("session_idle", properties); return; }
    if (event.type === "session.error") { post("session_error", properties); return; }
    if (event.type === "permission.asked") { post("permission_request", properties); return; }
    if (event.type === "question.asked") { post("ask_user_question", properties); return; }
    if (event.type === "message.updated") {
      const info = properties.info || {};
      boundedSet(roles, info.id, info.role, 128);
      return;
    }
    if (event.type !== "message.part.updated") return;
    const part = properties.part;
    if (!part) return;
    if (part.type === "text" && roles.get(part.messageID) === "user" && part.text) {
      post("user_prompt", { prompt: part.text, sessionID: properties.sessionID });
      return;
    }
    if (part.type !== "tool") return;
    const key = part.callID || part.id;
    const status = part.state?.status;
    if (!key || toolStates.get(key) === status) return;
    boundedSet(toolStates, key, status);
    const payload = {
      toolName: part.tool,
      toolCallId: key,
      input: part.state?.input,
      failed: status === "error",
    };
    if (status === "pending" || status === "running") post("tool_execution_start", payload);
    if (status === "completed" || status === "error") post("tool_execution_end", payload);
  },
});
"#;
