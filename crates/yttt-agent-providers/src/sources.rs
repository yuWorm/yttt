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
const MAX_CACHE = 256;
const MAX_MESSAGE_CACHE = 512;
const MAX_TREE_DEPTH = 64;
const DELIVERY_STREAM_ID = globalThis.crypto?.randomUUID?.() ||
  `yttt-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
const deliveryQueue = [];
let nextDeliverySequence = 1;
let deliveryWorker;
let deliveryStopped = false;

function boundedSet(map, key, value, limit = MAX_CACHE) {
  if (!key) return;
  map.delete(key);
  map.set(key, value);
  while (map.size > limit) map.delete(map.keys().next().value);
}

function text(value, limit = 2048) {
  if (typeof value !== "string") return undefined;
  const normalized = value.trim().replace(/\s+/g, " ");
  if (!normalized) return undefined;
  return normalized.length <= limit ? normalized : `${normalized.slice(0, limit - 1)}…`;
}

function integer(value) {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}

function sourceTimestamp(value) {
  const parsed = integer(value);
  return parsed && parsed > 0 ? parsed : undefined;
}

function timestamp(...values) {
  for (const value of values) {
    const parsed = integer(value);
    if (parsed && parsed > 0) return parsed;
  }
  return Date.now();
}

function costMicrousd(value) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return undefined;
  const converted = Math.round(value * 1_000_000);
  return Number.isSafeInteger(converted) && converted >= 0 ? converted : undefined;
}

function responseData(response) {
  return response && typeof response === "object" && "data" in response ? response.data : response;
}

function infoId(info) {
  return text(info?.id) || text(info?.sessionID) || text(info?.sessionId);
}

function sessionId(properties, record) {
  return text(properties?.sessionID) || text(properties?.sessionId) ||
    text(record?.sessionID) || text(record?.sessionId) ||
    text(properties?.info?.sessionID) || text(properties?.info?.sessionId);
}

function sessionParent(info) {
  // OpenCode's generated Session type exposes this exact field. Status events
  // deliberately have no parent information, so they are checked via get().
  return text(info?.parentID);
}

function sessionName(info) {
  return text(info?.title, 256) || "OpenCode";
}

function isBusy(status) {
  const type = typeof status === "object" ? status?.type : status;
  return type === "busy" || type === "retry";
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

export const YtttAgentStatusPlugin = async ({ client }) => {
  const roles = new Map();
  const messageTimes = new Map();
  const toolStates = new Map();
  const sessionStates = new Map();
  const sessions = new Map();
  const runs = new Map();
  const finishedMessages = new Map();
  const messageRuns = new Map();
  const sessionTasks = new Map();
  let rootSessionId;
  let rootInfo;
  let rootModel;
  let nextRunSequence = 1;
  const telemetryFields = ["input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens", "cost_microusd"];
  const tokenFields = ["input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens"];

  function remember(info) {
    const id = infoId(info);
    if (!id) return;
    const current = sessions.get(id);
    const next = { ...(current || {}), ...info, id };
    if (!sessionParent(info)) delete next.parentID;
    boundedSet(sessions, id, next);
    if (id === rootSessionId) rootInfo = next;
    return next;
  }

  async function loadSession(id) {
    if (!id) return;
    const cached = id === rootSessionId ? rootInfo || sessions.get(id) : sessions.get(id);
    if (cached) return cached;
    try {
      return remember(responseData(await client.session.get({ path: { id } })));
    } catch {
      return undefined;
    }
  }

  async function ownedInfo(id) {
    if (!id || !rootSessionId) return;
    let current = await loadSession(id);
    const original = current;
    const visited = new Set();
    for (let depth = 0; current && depth < MAX_TREE_DEPTH; depth += 1) {
      const currentId = infoId(current);
      if (!currentId || visited.has(currentId)) return;
      visited.add(currentId);
      if (currentId === rootSessionId) {
        return !sessionParent(current) ? original : undefined;
      }
      const parentId = sessionParent(current);
      if (!parentId) return;
      current = await loadSession(parentId);
    }
  }

  async function isCurrentRoot(id) {
    const info = await ownedInfo(id);
    return Boolean(info && infoId(info) === rootSessionId && !sessionParent(info));
  }

  function rootPayload(info, id) {
    return { ...(info || {}), sessionID: id, sessionId: id };
  }

  function emitRun(run) {
    void post("board_event", {
      board: {
        kind: "execution",
        execution: {
          id: run.id,
          parent_id: run.parentId || null,
          name: run.name,
          task: run.task || null,
          state: run.state,
          started_at: run.startedAt,
          updated_at: run.updatedAt,
          ended_at: run.endedAt || null,
          telemetry: run.telemetry,
          current_action: run.currentAction || null,
        },
      },
    });
  }

  function updateRun(run, observedAt, changes = {}) {
    if (run.endedAt) return false;
    if (changes.name) run.name = changes.name;
    if (changes.task) run.task = changes.task;
    if (Object.prototype.hasOwnProperty.call(changes, "action")) run.currentAction = changes.action;
    if (changes.state) run.state = changes.state;
    run.updatedAt = Math.max(run.updatedAt, timestamp(observedAt));
    return true;
  }

  async function ensureRun(info, observedAt) {
    const id = infoId(info);
    const startedAt = sourceTimestamp(observedAt);
    if (!id || !rootSessionId || !startedAt) return;
    const existing = runs.get(id);
    if (existing) {
      if (!existing.endedAt) return existing;
      if (startedAt <= existing.endedAt) return;
    }

    const parentSessionId = sessionParent(info);
    let parentRun;
    if (parentSessionId) {
      const parentInfo = await ownedInfo(parentSessionId);
      if (!parentInfo) return;
      // Child activity proves ancestry, not that an ended parent has started again.
      parentRun = runs.get(parentSessionId) || await ensureRun(parentInfo, startedAt);
      if (!parentRun) return;
    }

    const run = {
      id: `opencode:${id}:${DELIVERY_STREAM_ID}:${nextRunSequence++}`,
      rootId: rootSessionId,
      sessionId: id,
      parentId: parentRun?.id,
      name: sessionName(info),
      task: sessionTasks.get(id),
      state: "working",
      startedAt,
      updatedAt: startedAt,
      endedAt: undefined,
      currentAction: undefined,
      telemetry: {},
      usage: new Map(),
      usageBase: {},
      usageSeen: {},
      settledMessageIds: new Map(),
      messageIds: new Map(),
      childStarted: false,
      childFinished: false,
    };
    boundedSet(runs, id, run);
    emitRun(run);
    return run;
  }

  function rememberMessage(run, messageId) {
    boundedSet(run.messageIds, messageId, true, MAX_MESSAGE_CACHE);
    boundedSet(messageRuns, `${run.sessionId}\u0000${messageId}`, run.id, MAX_MESSAGE_CACHE);
  }

  function reportMessageUsage(run, messageId, usage) {
    let tokens = 0;
    let observed = false;
    let partial = false;
    for (const field of tokenFields) {
      const value = integer(usage[field]);
      if (value === undefined) {
        partial = true;
        continue;
      }
      observed = true;
      tokens += value;
    }
    if (!observed || !Number.isSafeInteger(tokens)) return;
    void post("token_usage", {
      usage: {
        id: `opencode:${run.sessionId}:message:${messageId}`,
        parent_id: null,
        tokens,
        includes_children: false,
        partial,
      },
    });
  }

  function aggregateTelemetry(run, message, observedAt) {
    const messageId = text(message?.id);
    if (!messageId || run.settledMessageIds.has(messageId)) return false;
    const prior = run.usage.get(messageId);
    const next = {
      input_tokens: integer(message?.tokens?.input) ?? prior?.input_tokens,
      output_tokens: integer(message?.tokens?.output) ?? prior?.output_tokens,
      cache_read_tokens: integer(message?.tokens?.cache?.read) ?? prior?.cache_read_tokens,
      cache_write_tokens: integer(message?.tokens?.cache?.write) ?? prior?.cache_write_tokens,
      cost_microusd: costMicrousd(message?.cost) ?? prior?.cost_microusd,
    };
    const model = text(message?.modelID, 256) || run.telemetry.model;
    if (prior && telemetryFields.every(field => prior[field] === next[field]) && model === run.telemetry.model) {
      return false;
    }
    if (!prior && run.usage.size >= MAX_MESSAGE_CACHE) {
      const oldest = run.usage.entries().next().value;
      if (oldest) {
        const [oldestId, oldestUsage] = oldest;
        run.usage.delete(oldestId);
        boundedSet(run.settledMessageIds, oldestId, true, MAX_MESSAGE_CACHE);
        for (const field of telemetryFields) {
          const value = integer(oldestUsage[field]);
          if (value === undefined) continue;
          const total = (integer(run.usageBase[field]) || 0) + value;
          if (!Number.isSafeInteger(total)) continue;
          run.usageBase[field] = total;
          run.usageSeen[field] = true;
        }
      }
    }
    boundedSet(run.usage, messageId, next, MAX_MESSAGE_CACHE);
    const telemetry = {
      ...run.telemetry,
      model,
      observed_at: timestamp(observedAt),
    };
    for (const field of telemetryFields) {
      let total = integer(run.usageBase[field]) || 0;
      let observed = run.usageSeen[field] === true;
      for (const usage of run.usage.values()) {
        const value = integer(usage[field]);
        if (value !== undefined) {
          total += value;
          observed = true;
        }
      }
      if (observed && Number.isSafeInteger(total)) telemetry[field] = total;
    }
    run.telemetry = telemetry;
    reportMessageUsage(run, messageId, next);
    rememberMessage(run, messageId);
    return true;
  }

  function startChild(run) {
    if (run.childStarted) return;
    run.childStarted = true;
    void post("child_started", {
      childId: run.sessionId,
      name: run.name,
      task: run.task,
    });
  }

  function updateChild(run, update) {
    startChild(run);
    void post("child_updated", { childId: run.sessionId, ...update });
  }

  function finishRun(run, outcome, observedAt) {
    if (!run || run.endedAt) return;
    const endedAt = timestamp(observedAt);
    run.endedAt = Math.max(run.startedAt, run.updatedAt, endedAt);
    run.updatedAt = run.endedAt;
    run.currentAction = undefined;
    run.state = outcome === "completed" ? "completed" : "interrupted";
    for (const messageId of [...run.messageIds.keys(), ...run.settledMessageIds.keys()]) {
      boundedSet(finishedMessages, `${run.sessionId}\u0000${messageId}`, true, MAX_MESSAGE_CACHE);
    }
    emitRun(run);
    if (run.parentId && !run.childFinished) {
      run.childFinished = true;
      void post("child_finished", { childId: run.sessionId, outcome });
    }
  }

  function finishRootTree(rootId, outcome, observedAt) {
    for (const run of [...runs.values()].reverse()) {
      if (run.rootId === rootId) finishRun(run, outcome, observedAt);
    }
  }

  function selectRoot(info) {
    const id = infoId(info);
    if (!id || sessionParent(info) || rootSessionId) return false;
    rootSessionId = id;
    rootInfo = info;
    rootModel = undefined;
    remember(info);
    void post("session_start", rootPayload(info, id));
    return true;
  }

  async function discoverDescendants(rootId) {
    if (typeof client?.session?.children !== "function") return [];
    const discovered = [];
    const queue = [rootId];
    const seen = new Set(queue);
    while (queue.length > 0 && seen.size <= MAX_CACHE) {
      const parentId = queue.shift();
      let children;
      try {
        children = responseData(await client.session.children({ path: { id: parentId } }));
      } catch {
        continue;
      }
      for (const candidate of Array.isArray(children) ? children : []) {
        const info = remember(candidate);
        const id = infoId(info);
        if (!id || sessionParent(info) !== parentId || seen.has(id)) continue;
        if (!await ownedInfo(id)) continue;
        seen.add(id);
        queue.push(id);
        discovered.push(info);
      }
    }
    return discovered;
  }

  async function activateChild(info, observedAt, action) {
    const run = await ensureRun(info, observedAt);
    if (!run) return;
    const changes = { state: "working" };
    if (action) changes.action = action;
    updateRun(run, observedAt, changes);
    startChild(run);
    emitRun(run);
    return run;
  }

  async function activateRootRun(info, observedAt, action) {
    const run = await ensureRun(info, observedAt);
    if (!run) return;
    const changes = { state: "working" };
    if (action) changes.action = action;
    updateRun(run, observedAt, changes);
    emitRun(run);
    return run;
  }

  async function activateRoot(info) {
    if (!selectRoot(info)) return false;
    await discoverDescendants(infoId(info));
    return true;
  }

  async function updateRootModel(info, message) {
    if (infoId(info) !== rootSessionId) return;
    const model = text(message?.modelID, 256);
    if (!model || model === rootModel) return;
    rootModel = model;
    void post("session_updated", { ...rootPayload(rootInfo || info, rootSessionId), model });
  }

  async function observeAssistant(message) {
    const id = sessionId({}, message);
    const messageId = text(message?.id);
    const createdAt = sourceTimestamp(message?.time?.created);
    if (!id || !messageId || !createdAt || finishedMessages.has(`${id}\u0000${messageId}`)) return;
    const info = await ownedInfo(id);
    if (!info) return;
    const knownRunId = messageRuns.get(`${id}\u0000${messageId}`);
    const current = runs.get(id);
    if (knownRunId) {
      if (current?.id !== knownRunId || current.endedAt) return;
      updateRun(current, message?.time?.completed || createdAt);
      if (!aggregateTelemetry(current, message, Date.now())) return;
      if (current.parentId) startChild(current);
      await updateRootModel(info, message);
      emitRun(current);
      return;
    }
    const run = await ensureRun(info, createdAt);
    if (!run || run.endedAt) return;
    updateRun(run, message?.time?.completed || createdAt);
    if (!aggregateTelemetry(run, message, Date.now())) return;
    if (run.parentId) startChild(run);
    await updateRootModel(info, message);
    emitRun(run);
  }

  async function observeUserText(id, prompt, observedAt) {
    const info = await loadSession(id);
    if (!info) return;
    if (!sessionParent(info)) {
      if (id !== rootSessionId || !await isCurrentRoot(id)) return;
      boundedSet(sessionTasks, id, prompt, MAX_CACHE);
      const run = runs.get(id);
      if (run && !run.endedAt) {
        updateRun(run, observedAt, { task: prompt });
        emitRun(run);
      }
      void post("user_prompt", { prompt, sessionID: id, sessionId: id });
      return;
    }
    if (!await ownedInfo(id)) return;
    boundedSet(sessionTasks, id, prompt, MAX_CACHE);
    const run = await activateChild(info, observedAt);
    if (!run) return;
    updateRun(run, observedAt, { task: prompt });
    emitRun(run);
    updateChild(run, { task: prompt, status: "working" });
  }

  async function attachExistingRoot() {
    try {
      const [listed, statuses] = await Promise.all([
        client.session.list().then(responseData),
        client.session.status().then(responseData),
      ]);
      const candidates = [];
      for (const info of Array.isArray(listed) ? listed : []) {
        const id = infoId(info);
        if (id && !sessionParent(info) && isBusy(statuses?.[id])) candidates.push([info, statuses[id]]);
      }
      if (rootSessionId || candidates.length !== 1) return;
      const [info, status] = candidates[0];
      if (!selectRoot(info)) return;
      boundedSet(sessionStates, info.id, "busy");
      await activateRootRun(info, info?.time?.updated);
      void post("session_busy", { sessionID: info.id, sessionId: info.id, status });
      const descendants = await discoverDescendants(info.id);
      for (const child of descendants) {
        if (isBusy(statuses?.[infoId(child)])) {
          boundedSet(sessionStates, infoId(child), "busy");
          await activateChild(child, child?.time?.updated);
        }
      }
    } catch {}
  }

  void attachExistingRoot();

  return {
    event: async ({ event }) => {
      if (!event?.type) return;
      const properties = event.properties || {};
      if (event.type === "session.created") {
        const info = remember(properties.info || properties);
        if (!info) return;
        if (!sessionParent(info)) {
          if (!rootSessionId) await activateRoot(info);
          return;
        }
        if (await ownedInfo(infoId(info))) await activateChild(info, info?.time?.created);
        return;
      }
      if (event.type === "session.updated") {
        const info = remember(properties.info || properties);
        const id = infoId(info);
        if (!info || !id) return;
        if (id === rootSessionId && !sessionParent(info)) {
          void post("session_updated", rootPayload(info, id));
          return;
        }
        if (await ownedInfo(id)) {
          const run = runs.get(id);
          if (run && !run.endedAt) {
            updateRun(run, info?.time?.updated, { name: sessionName(info) });
            emitRun(run);
          }
          return;
        }
        const run = runs.get(id);
        if (run?.rootId === rootSessionId) finishRun(run, "interrupted", info?.time?.updated);
        if (id === rootSessionId) {
          finishRootTree(id, "interrupted", info?.time?.updated);
          sessionStates.delete(id);
          rootSessionId = undefined;
          rootInfo = undefined;
        }
        return;
      }
      if (event.type === "session.deleted") {
        const info = remember(properties.info || properties);
        const id = sessionId(properties) || infoId(info);
        const owned = await ownedInfo(id);
        if (!id || !owned) return;
        if (id === rootSessionId) {
          finishRootTree(id, "interrupted", info?.time?.updated);
          rootSessionId = undefined;
          rootInfo = undefined;
          rootModel = undefined;
        } else {
          finishRun(runs.get(id), "interrupted", info?.time?.updated);
        }
        sessions.delete(id);
        sessionStates.delete(id);
        sessionTasks.delete(id);
        return;
      }
      if (event.type === "session.status" || event.type === "session.idle") {
        const id = sessionId(properties);
        const info = await ownedInfo(id);
        if (!info) return;
        const status = event.type === "session.idle" ? "idle" : properties.status;
        const idle = (typeof status === "object" ? status?.type : status) === "idle";
        const busy = isBusy(status);
        const wasBusy = sessionStates.get(id) === "busy";
        if (busy || idle) boundedSet(sessionStates, id, busy ? "busy" : "idle");
        if (id === rootSessionId) {
          if (!await isCurrentRoot(id)) return;
          if (busy) {
            void post("session_busy", { ...properties, sessionID: id, sessionId: id });
            if (!wasBusy) await activateRootRun(info, info?.time?.updated);
          }
          if (idle) {
            void post("session_idle", { ...properties, sessionID: id, sessionId: id });
            finishRun(runs.get(id), "completed", info?.time?.updated);
          }
          return;
        }
        if (busy && !wasBusy) {
          const run = await activateChild(info, info?.time?.updated);
          if (run) updateChild(run, { status: "working" });
          return;
        }
        if (idle) finishRun(runs.get(id), "completed", info?.time?.updated);
        return;
      }
      if (event.type === "session.error") return;
      if (event.type === "permission.asked" || event.type === "question.asked") {
        const id = sessionId(properties);
        const info = await ownedInfo(id);
        if (!info) return;
        const run = runs.get(id);
        if (run && !run.endedAt) {
          boundedSet(sessionStates, id, "waiting");
          updateRun(run, undefined, { state: "waiting" });
          emitRun(run);
          if (run.parentId) updateChild(run, { status: "waiting" });
        }
        if (id !== rootSessionId) return;
        void post(event.type === "permission.asked" ? "permission_request" : "ask_user_question", {
          ...properties,
          sessionID: id,
          sessionId: id,
        });
        return;
      }
      if (event.type === "message.updated") {
        const info = properties.info || {};
        const id = sessionId(properties, info);
        const messageId = text(info.id);
        if (id && messageId && text(info.role)) {
          const key = `${id}\u0000${messageId}`;
          boundedSet(roles, key, info.role, MAX_MESSAGE_CACHE);
          const createdAt = sourceTimestamp(info?.time?.created);
          if (createdAt) boundedSet(messageTimes, key, createdAt, MAX_MESSAGE_CACHE);
        }
        if (text(info.role) === "assistant") await observeAssistant(info);
        return;
      }
      if (event.type !== "message.part.updated") return;
      const part = properties.part;
      if (!part) return;
      const id = sessionId(properties, part);
      if (!id) return;
      if (part.type === "text" && roles.get(`${id}\u0000${part.messageID}`) === "user" && text(part.text)) {
        await observeUserText(id, text(part.text), messageTimes.get(`${id}\u0000${part.messageID}`));
        return;
      }
      if (part.type !== "tool") return;
      const info = await ownedInfo(id);
      if (!info) return;
      const callId = text(part.callID) || text(part.id);
      const status = part.state?.status;
      const key = `${id}\u0000${callId}`;
      if (!callId || toolStates.get(key) === status) return;
      boundedSet(toolStates, key, status, MAX_MESSAGE_CACHE);
      const action = text(part.state?.title) || text(part.tool);
      const payload = {
        sessionID: id,
        sessionId: id,
        toolName: part.tool,
        toolCallId: callId,
        input: part.state?.input,
        failed: status === "error",
      };
      if (status === "pending" || status === "running") {
        if (id === rootSessionId) {
          await activateRootRun(info, part.state?.time?.start, action);
          void post("tool_execution_start", payload);
        } else {
          const run = await activateChild(info, part.state?.time?.start, action);
          if (run) updateChild(run, { action, status: "working" });
        }
      }
      if (status === "completed" || status === "error") {
        const run = runs.get(id);
        if (!run || run.endedAt) return;
        updateRun(run, part.state?.time?.end, { action: undefined });
        emitRun(run);
        if (id === rootSessionId) void post("tool_execution_end", payload);
        else updateChild(run, { status: "working", clearCurrentAction: true });
      }
    },
  };
};
"#;
