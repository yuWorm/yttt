pub const PI_EXTENSION_SOURCE: &str = r#"const PREFIX = "yttt-agent-v1:";
const MAX_TEXT = 2048;
const DELIVERY_PROTOCOL = 1;
const DELIVERY_ATTEMPTS = 3;
const DELIVERY_TIMEOUT_MS = 2000;
const DELIVERY_RETRY_DELAY_MS = 1000;
const DELIVERY_STREAM_ID = globalThis.crypto?.randomUUID?.() ||
  `yttt-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
const OWNER_KEY = Symbol.for("yttt-agent-status.pi-session-owners");
const globalOwners = globalThis;
const sessionOwners = globalOwners[OWNER_KEY] ||= new Map();
const deliveryQueue = [];
let nextDeliverySequence = 1;
let deliveryWorker;
let deliveryStopped = false;

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

function reportingScope() {
  const endpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
  const hookToken = process.env.YTTT_AGENT_HOOK_TOKEN;
  const hookScope = process.env.YTTT_AGENT_HOOK_SCOPE;
  if (endpoint && hookToken && hookScope) return JSON.stringify(["hook", endpoint, hookToken, hookScope]);
  const instanceId = process.env.YTTT_AGENT_INSTANCE_ID;
  const token = process.env.YTTT_AGENT_TOKEN;
  const generation = process.env.YTTT_AGENT_GENERATION;
  if (instanceId && token && generation) return JSON.stringify(["osc", instanceId, token, generation]);
}

function sessionId(ctx) {
  return clip(ctx?.sessionManager?.getSessionId?.());
}

function claimSession(ctx, ownerId, switching = false) {
  const scope = reportingScope();
  const id = sessionId(ctx);
  if (!scope || !id) return true;
  const owner = sessionOwners.get(scope);
  if (!owner) {
    sessionOwners.set(scope, { ownerId, sessionId: id });
    return true;
  }
  if (owner.ownerId !== ownerId) return false;
  if (owner.sessionId === id) return true;
  if (!switching) return false;
  sessionOwners.set(scope, { ownerId, sessionId: id });
  return true;
}

function ownsSession(ctx, ownerId) {
  const scope = reportingScope();
  const id = sessionId(ctx);
  if (!scope || !id) return true;
  const owner = sessionOwners.get(scope);
  return owner?.ownerId === ownerId && owner.sessionId === id;
}

function releaseSession(scope, owner) {
  if (scope && sessionOwners.get(scope) === owner) sessionOwners.delete(scope);
}

function sleep(milliseconds) {
  return new Promise(resolve => setTimeout(resolve, milliseconds));
}

function startDeliveryWorker() {
  if (deliveryWorker) return;
  deliveryWorker = drainDeliveryQueue().finally(() => {
    deliveryWorker = undefined;
    if (deliveryQueue.length > 0) startDeliveryWorker();
  });
}

async function deliverHook(delivery) {
  let status;
  for (let attempt = 0; attempt < DELIVERY_ATTEMPTS; attempt += 1) {
    let retry = true;
    try {
      const response = await fetch(`${delivery.endpoint}/hook/pi`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-yttt-agent-hook-token": delivery.token,
          "x-yttt-agent-hook-scope": delivery.scope,
        },
        body: JSON.stringify({
          protocol: DELIVERY_PROTOCOL,
          streamId: DELIVERY_STREAM_ID,
          sequence: delivery.sequence,
          event: delivery.event,
          payload: delivery.payload,
        }),
        signal: AbortSignal.timeout(DELIVERY_TIMEOUT_MS),
      });
      status = response.status;
      if (response.ok) {
        const acknowledgement = await response.json().catch(() => undefined);
        if (
          typeof acknowledgement?.acceptedSequence === "number" &&
          Number.isSafeInteger(acknowledgement.acceptedSequence) &&
          acknowledgement.acceptedSequence >= delivery.sequence
        ) return "accepted";
      } else {
        retry = response.status === 408 || response.status === 429 || response.status >= 500;
      }
    } catch {}
    if (!retry) {
      console.error(`yttt agent status delivery stopped (HTTP ${status})`);
      return "permanent";
    }
    if (attempt + 1 < DELIVERY_ATTEMPTS) await sleep(DELIVERY_RETRY_DELAY_MS);
  }
  return "retry";
}

async function drainDeliveryQueue() {
  while (deliveryQueue.length > 0) {
    const delivery = deliveryQueue[0];
    const result = await deliverHook(delivery);
    if (result === "retry") {
      await sleep(DELIVERY_RETRY_DELAY_MS);
      continue;
    }
    if (result === "permanent") {
      deliveryStopped = true;
      while (deliveryQueue.length > 0) deliveryQueue.shift().settled();
      return;
    }
    deliveryQueue.shift()?.settled();
  }
}

function enqueueDelivery(endpoint, token, scope, event, payload) {
  if (deliveryStopped) return Promise.resolve();
  const completion = Promise.withResolvers();
  deliveryQueue.push({
    endpoint,
    token,
    scope,
    sequence: nextDeliverySequence++,
    event,
    payload,
    settled: completion.resolve,
  });
  startDeliveryWorker();
  return completion.promise;
}

function send(name, payload, ctx) {
  const enriched = {
    ...payload,
    sessionId: sessionId(ctx),
    sessionFile: ctx?.sessionManager?.getSessionFile?.(),
    model: ctx?.model?.id,
  };
  const endpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
  const hookToken = process.env.YTTT_AGENT_HOOK_TOKEN;
  const scope = process.env.YTTT_AGENT_HOOK_SCOPE;
  if (endpoint && hookToken && scope) {
    return enqueueDelivery(endpoint, hookToken, scope, name, enriched);
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
  const ownerId = `${DELIVERY_STREAM_ID}-${Math.random().toString(36).slice(2)}`;
  const trackContext = (event, ctx) => {
    const switching = event?.type === "session_switch" ||
      ["new", "resume", "fork"].includes(event?.reason);
    if (!claimSession(ctx, ownerId, switching)) return;
    void send(event?.type === "session_switch" ? "session_switch" : "session_start", {}, ctx);
  };
  pi.on("session_start", trackContext);
  // Pi's current lifecycle uses session_start with reason "reload", "new", or
  // "resume"; retain the old event only for compatible older runtimes.
  pi.on("session_switch", trackContext);
  pi.on("session_shutdown", async (_event, ctx) => {
    if (!ownsSession(ctx, ownerId)) return;
    const scope = reportingScope();
    const owner = scope && sessionOwners.get(scope);
    let timer;
    const timeout = new Promise(resolve => { timer = setTimeout(resolve, 2000); });
    try {
      await Promise.race([send("session_shutdown", {}, ctx), timeout]);
    } finally {
      clearTimeout(timer);
      releaseSession(scope, owner);
    }
  });
  pi.on("before_agent_start", (event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("before_agent_start", { prompt: clip(event?.prompt) }, ctx);
  });
  pi.on("agent_start", (_event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("agent_start", {}, ctx);
  });
  pi.on("agent_end", (event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("agent_end", { willContinue: event?.willContinue === true }, ctx);
  });
  pi.on("tool_call", (event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("tool_call", {
      toolCallId: event?.toolCallId,
      toolName: event?.toolName,
      detail: toolDetail(event?.input),
    }, ctx);
  });
  pi.on("tool_approval_requested", (event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("tool_approval_requested", {
      toolCallId: event?.toolCallId,
      toolName: event?.toolName,
      reason: clip(event?.reason),
    }, ctx);
  });
  pi.on("tool_approval_resolved", (event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("tool_approval_resolved", {
      toolCallId: event?.toolCallId,
      toolName: event?.toolName,
      approved: event?.approved,
      reason: clip(event?.reason),
    }, ctx);
  });
  pi.on("tool_execution_start", (event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("tool_execution_start", {
      toolCallId: event?.toolCallId,
      toolName: event?.toolName,
      detail: clip(event?.intent) || toolDetail(event?.args),
    }, ctx);
  });
  pi.on("tool_execution_end", (event, ctx) => {
    if (ownsSession(ctx, ownerId)) void send("tool_execution_end", {
      toolCallId: event?.toolCallId,
      toolName: event?.toolName,
      isError: event?.isError,
    }, ctx);
  });
}
"#;
pub const OPENCODE_PLUGIN_SOURCE: &str = r#"const endpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
const token = process.env.YTTT_AGENT_HOOK_TOKEN;
const scope = process.env.YTTT_AGENT_HOOK_SCOPE;
const DELIVERY_PROTOCOL = 1;
const DELIVERY_ATTEMPTS = 3;
const DELIVERY_TIMEOUT_MS = 2000;
const DELIVERY_RETRY_DELAY_MS = 1000;
const DELIVERY_STREAM_ID = globalThis.crypto?.randomUUID?.() ||
  `yttt-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
const deliveryQueue = [];
let nextDeliverySequence = 1;
let deliveryWorker;
let deliveryStopped = false;

function boundedSet(map, key, value, limit = 256) {
  if (!key) return;
  map.delete(key);
  map.set(key, value);
  while (map.size > limit) map.delete(map.keys().next().value);
}

function sleep(milliseconds) {
  return new Promise(resolve => setTimeout(resolve, milliseconds));
}

function startDeliveryWorker() {
  if (deliveryWorker) return;
  deliveryWorker = drainDeliveryQueue().finally(() => {
    deliveryWorker = undefined;
    if (deliveryQueue.length > 0) startDeliveryWorker();
  });
}

async function deliverHook(delivery) {
  let status;
  for (let attempt = 0; attempt < DELIVERY_ATTEMPTS; attempt += 1) {
    let retry = true;
    try {
      const response = await fetch(`${delivery.endpoint}/hook/opencode`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-yttt-agent-hook-token": delivery.token,
          "x-yttt-agent-hook-scope": delivery.scope,
        },
        body: JSON.stringify({
          protocol: DELIVERY_PROTOCOL,
          streamId: DELIVERY_STREAM_ID,
          sequence: delivery.sequence,
          event: delivery.event,
          payload: delivery.payload,
        }),
        signal: AbortSignal.timeout(DELIVERY_TIMEOUT_MS),
      });
      status = response.status;
      if (response.ok) {
        const acknowledgement = await response.json().catch(() => undefined);
        if (
          typeof acknowledgement?.acceptedSequence === "number" &&
          Number.isSafeInteger(acknowledgement.acceptedSequence) &&
          acknowledgement.acceptedSequence >= delivery.sequence
        ) return "accepted";
      } else {
        retry = response.status === 408 || response.status === 429 || response.status >= 500;
      }
    } catch {}
    if (!retry) {
      console.error(`yttt agent status delivery stopped (HTTP ${status})`);
      return "permanent";
    }
    if (attempt + 1 < DELIVERY_ATTEMPTS) await sleep(DELIVERY_RETRY_DELAY_MS);
  }
  return "retry";
}

async function drainDeliveryQueue() {
  while (deliveryQueue.length > 0) {
    const delivery = deliveryQueue[0];
    const result = await deliverHook(delivery);
    if (result === "retry") {
      await sleep(DELIVERY_RETRY_DELAY_MS);
      continue;
    }
    if (result === "permanent") {
      deliveryStopped = true;
      while (deliveryQueue.length > 0) deliveryQueue.shift().settled();
      return;
    }
    deliveryQueue.shift()?.settled();
  }
}

function post(event, payload = {}) {
  if (!endpoint || !token || !scope || deliveryStopped) return Promise.resolve();
  const completion = Promise.withResolvers();
  deliveryQueue.push({
    endpoint,
    token,
    scope,
    sequence: nextDeliverySequence++,
    event,
    payload,
    settled: completion.resolve,
  });
  startDeliveryWorker();
  return completion.promise;
}

function text(value) {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function responseData(response) {
  return response && typeof response === "object" && "data" in response ? response.data : response;
}

function sessionId(properties, record) {
  return text(properties?.sessionID) || text(properties?.sessionId) ||
    text(record?.sessionID) || text(record?.sessionId) ||
    text(properties?.info?.sessionID) || text(properties?.info?.sessionId) ||
    text(properties?.info?.id);
}

function sessionParent(info) {
  return text(info?.parentID) || text(info?.parentId) ||
    text(info?.parentSessionId) || text(info?.parentSessionID) ||
    text(info?.parent_session_id);
}

function isBusy(status) {
  const type = typeof status === "object" ? status?.type : status;
  return type === "busy" || type === "retry";
}

export const YtttAgentStatusPlugin = async ({ client }) => {
  const roles = new Map();
  const toolStates = new Map();
  const sessions = new Map();
  let rootSessionId;

  function remember(info) {
    const id = text(info?.id) || text(info?.sessionID) || text(info?.sessionId);
    if (!id) return;
    const current = sessions.get(id);
    const next = { ...(current || {}), ...info, id };
    boundedSet(sessions, id, next);
    return next;
  }

  async function loadSession(id) {
    const cached = sessions.get(id);
    if (cached) return cached;
    try {
      return remember(responseData(await client.session.get({ path: { id } })));
    } catch {
      return undefined;
    }
  }

  function rootPayload(info, id) {
    return { ...(info || {}), sessionID: id };
  }

  function selectRoot(info) {
    const id = text(info?.id);
    if (!id || sessionParent(info)) return false;
    if (rootSessionId === id) return true;
    rootSessionId = id;
    void post("session_start", rootPayload(info, id));
    return true;
  }

  async function isCurrentRoot(id) {
    if (!id || rootSessionId !== id) return false;
    const info = sessions.get(id) || await loadSession(id);
    return rootSessionId === id && (!info || !sessionParent(info));
  }

  async function attachExistingRoot() {
    try {
      const [listed, statuses] = await Promise.all([
        client.session.list().then(responseData),
        client.session.status().then(responseData),
      ]);
      for (const info of Array.isArray(listed) ? listed : []) remember(info);
      const candidates = [];
      for (const [id, status] of Object.entries(statuses || {})) {
        const info = sessions.get(id) || await loadSession(id);
        if (info && !sessionParent(info) && isBusy(status)) candidates.push([info, status]);
      }
      if (rootSessionId || candidates.length !== 1) return;
      const [info, status] = candidates[0];
      if (!selectRoot(info)) return;
      void post("session_busy", { sessionID: info.id, status });
    } catch {}
  }

  void attachExistingRoot();

  return {
    event: async ({ event }) => {
      if (!event?.type) return;
      const properties = event.properties || {};
      if (event.type === "session.created") {
        const info = remember(properties.info || properties);
        if (info) selectRoot(info);
        return;
      }
      if (event.type === "session.updated") {
        const info = remember(properties.info || properties);
        if (info?.id && rootSessionId === info.id && !sessionParent(info)) {
          void post("session_updated", rootPayload(info, info.id));
        }
        return;
      }
      if (event.type === "session.status" || event.type === "session.idle") {
        const id = sessionId(properties);
        if (sessionParent(properties) || !await isCurrentRoot(id)) return;
        const status = event.type === "session.idle" ? "idle" : properties.status;
        if (isBusy(status)) void post("session_busy", { ...properties, sessionID: id });
        if ((typeof status === "object" ? status?.type : status) === "idle") {
          void post("session_idle", { ...properties, sessionID: id });
        }
        return;
      }
      if (event.type === "session.error") return;
      if (event.type === "permission.asked" || event.type === "question.asked") {
        const id = sessionId(properties);
        if (sessionParent(properties) || !await isCurrentRoot(id)) return;
        void post(event.type === "permission.asked" ? "permission_request" : "ask_user_question", {
          ...properties,
          sessionID: id,
        });
        return;
      }
      if (event.type === "message.updated") {
        const info = properties.info || {};
        const id = sessionId(properties, info);
        const messageId = text(info.id);
        if (id && messageId) boundedSet(roles, `${id}\u0000${messageId}`, info.role, 128);
        return;
      }
      if (event.type !== "message.part.updated") return;
      const part = properties.part;
      if (!part) return;
      const id = sessionId(properties, part);
      if (!id) return;
      if (part.type === "text" && roles.get(`${id}\u0000${part.messageID}`) === "user" && part.text) {
        const info = sessions.get(id) || await loadSession(id);
        if (sessionParent(properties) || !info || sessionParent(info) || !selectRoot(info)) return;
        void post("user_prompt", { prompt: part.text, sessionID: id });
        return;
      }
      if (part.type !== "tool" || sessionParent(properties) || !await isCurrentRoot(id)) return;
      const key = `${id}\u0000${part.callID || part.id}`;
      const status = part.state?.status;
      if (!part.callID && !part.id || toolStates.get(key) === status) return;
      boundedSet(toolStates, key, status);
      const payload = {
        sessionID: id,
        toolName: part.tool,
        toolCallId: part.callID || part.id,
        input: part.state?.input,
        failed: status === "error",
      };
      if (status === "pending" || status === "running") void post("tool_execution_start", payload);
      if (status === "completed" || status === "error") void post("tool_execution_end", payload);
    },
  };
};
"#;
