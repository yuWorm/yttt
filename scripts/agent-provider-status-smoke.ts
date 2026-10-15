import assert from "node:assert/strict";

type Delivery = {
	protocol: number;
	streamId: string;
	sequence: number;
	event: string;
	payload: Record<string, unknown>;
	path: string;
};

type Handler = (event: Record<string, unknown>, context: Context) => Promise<void> | void;
type Context = {
	sessionManager: {
		getSessionId(): string;
		getSessionFile(): string;
	};
	model: { id: string };
};

const sourcePath = process.argv[2];
if (!sourcePath) {
	throw new Error("usage: bun scripts/agent-provider-status-smoke.ts <crates/yttt-agent-providers/src/sources.rs>");
}

function embeddedSource(source: string, constant: string): string {
	const prefix = `pub const ${constant}: &str = r#\"`;
	const start = source.indexOf(prefix);
	assert.notEqual(start, -1, `${constant} must be present`);
	const bodyStart = start + prefix.length;
	const end = source.indexOf('"#;', bodyStart);
	assert.notEqual(end, -1, `${constant} must close its Rust raw string`);
	return source.slice(bodyStart, end);
}

class MockPi {
	readonly handlers = new Map<string, Handler[]>();

	on(event: string, handler: Handler): void {
		const handlers = this.handlers.get(event) ?? [];
		handlers.push(handler);
		this.handlers.set(event, handlers);
	}

	async emit(event: string, context: Context, payload: Record<string, unknown> = { type: event }): Promise<void> {
		for (const handler of this.handlers.get(event) ?? []) await handler(payload, context);
	}
}

function context(sessionId: string): Context {
	return {
		sessionManager: {
		getSessionId: () => sessionId,
		getSessionFile: () => `/tmp/${sessionId}.jsonl`,
		},
		model: { id: "model-1" },
	};
}

async function waitFor(predicate: () => boolean, message: string): Promise<void> {
	const deadline = performance.now() + 15_000;
	while (!predicate()) {
		if (performance.now() >= deadline) throw new Error(message);
		await Bun.sleep(5);
	}
}

const rustSource = await Bun.file(sourcePath).text();
const piSource = embeddedSource(rustSource, "PI_EXTENSION_SOURCE");
const openCodeSource = embeddedSource(rustSource, "OPENCODE_PLUGIN_SOURCE");
const delivered = new Map<string, Delivery>();
const received: Delivery[] = [];
const responseAttempts = new Map<string, number>();
const faultedRoutes = new Set<string>();
const unauthorizedRequests = new Map<string, number>();

const server = Bun.serve({
	port: 0,
	async fetch(request) {
		const body = await request.json() as Omit<Delivery, "path">;
		const pathName = new URL(request.url).pathname;
		const scope = request.headers.get("x-yttt-agent-hook-scope") ?? "";
		if (scope.startsWith("unauthorized-")) {
			unauthorizedRequests.set(scope, (unauthorizedRequests.get(scope) ?? 0) + 1);
			return new Response(null, { status: 401 });
		}
		const delivery = { ...body, path: pathName };
		received.push(delivery);
		const key = `${pathName}\u0000${delivery.streamId}\u0000${delivery.sequence}`;
		const attempt = (responseAttempts.get(key) ?? 0) + 1;
		responseAttempts.set(key, attempt);

		// Outage lasts beyond one retry batch, then an accepted request loses
		// its acknowledgement. Later events must never skip the missing head.
		if (!faultedRoutes.has(pathName)) {
			if (attempt <= 3) return new Response("temporary", { status: 503 });
			if (attempt === 4) {
				faultedRoutes.add(pathName);
				delivered.set(key, delivery);
				return Response.json({});
			}
		}
		delivered.set(key, delivered.get(key) ?? delivery);
		return Response.json({ acceptedSequence: delivery.sequence });
	},
});

process.env.YTTT_AGENT_HOOK_ENDPOINT = `http://127.0.0.1:${server.port}`;
process.env.YTTT_AGENT_HOOK_TOKEN = "test-token";
process.env.YTTT_AGENT_HOOK_SCOPE = "same-scope";

// The script executes extracted plugin bodies as fresh factories; static imports
// cannot exercise duplicate extension paths or fresh reload factories.
function loadPi(): (pi: MockPi) => void {
	const factorySource = piSource.replace("export default function", "return function");
	return new Function(factorySource)();
}

try {
	// Distinct child factories share the terminal scope but must never publish
	// root lifecycle events. Duplicate path copies must also remain silent.
	const piRoot = new MockPi();
	const piFactoryOne = loadPi();
	const piFactoryTwo = loadPi();
	piFactoryOne(piRoot);
	piFactoryTwo(piRoot);
	const root = context("pi-root");
	const child = context("pi-child");
	await piRoot.emit("session_start", root);
	await piRoot.emit("agent_start", root);
	const piChild = new MockPi();
	loadPi()(piChild);
	await piChild.emit("session_start", child);
	await piChild.emit("session_shutdown", child);
	await piRoot.emit("agent_end", root, { willContinue: true });
	await waitFor(
		() => [...delivered.values()].some(delivery => delivery.path === "/hook/pi" && delivery.event === "agent_end"),
		"Pi did not recover the head delivery after a sustained outage",
	);
	await piRoot.emit("session_shutdown", root);

	// /reload sends old session_shutdown, loads fresh factories, then sends
	// session_start. A stale old shutdown must not release the new reporter.
	const reloaded = new MockPi();
	const piFactoryReloadOne = loadPi();
	const piFactoryReloadTwo = loadPi();
	piFactoryReloadOne(reloaded);
	piFactoryReloadTwo(reloaded);
	await reloaded.emit("session_start", root, { type: "session_start", reason: "reload" });
	await piRoot.emit("session_shutdown", root, { type: "session_shutdown", reason: "reload" });
	await reloaded.emit("agent_start", root);

	const piEvents = () => [...delivered.values()].filter(delivery => delivery.path === "/hook/pi");
	await waitFor(
		() => piEvents().filter(delivery => delivery.event === "session_start" && delivery.payload.sessionId === "pi-root").length === 2,
		"Pi reload lifecycle deliveries did not drain",
	);
	assert.equal(
		piEvents().filter(delivery => delivery.event === "session_start" && delivery.payload.sessionId === "pi-root").length,
		2,
		"duplicate paths must report one root start before and one after reload",
	);
	assert.deepEqual(
		piEvents().filter(delivery => delivery.payload.sessionId === "pi-child").map(delivery => delivery.event),
		[],
		"a child must not publish root lifecycle events to the same terminal scope",
	);
	assert.equal(
		piEvents().filter(delivery => delivery.event === "session_shutdown" && delivery.payload.sessionId === "pi-root").length,
		1,
		"a stale old factory cannot close the new root reporter",
	);
	const resumed = context("pi-resumed");
	await reloaded.emit("session_start", resumed, { type: "session_start", reason: "resume" });
	await reloaded.emit("agent_start", resumed);
	await reloaded.emit("session_shutdown", root);
	await reloaded.emit("agent_end", resumed, { willContinue: true });
	await reloaded.emit("session_shutdown", resumed);
	assert.deepEqual(
		piEvents().filter(delivery => delivery.payload.sessionId === "pi-resumed").map(delivery => delivery.event),
		["session_start", "agent_start", "agent_end", "session_shutdown"],
		"legitimate resume must transfer ownership without stale shutdown closing it",
	);
	// This generated OpenCode plugin has no imports, so a fresh function is the
	// direct plugin-loader boundary the smoke needs to exercise.
	const YtttAgentStatusPlugin = new Function(
		`${openCodeSource.replace("export const YtttAgentStatusPlugin", "const YtttAgentStatusPlugin")}\nreturn YtttAgentStatusPlugin;`,
	)();
	const sessions = new Map<string, Record<string, unknown>>([
		["attached-root", { id: "attached-root", title: "Attached root", time: { created: 1_000, updated: 1_001 } }],
		["next-root", { id: "next-root", title: "Resumed root", time: { created: 2_000, updated: 2_001 } }],
		["child", { id: "child", parentID: "attached-root", title: "Child worker", time: { created: 1_100, updated: 1_101 } }],
		["grandchild", { id: "grandchild", parentID: "child", title: "Grandchild worker", time: { created: 1_200, updated: 1_201 } }],
		["late-child", { id: "late-child", parentID: "attached-root", title: "Late child", time: { created: 1_300, updated: 1_301 } }],
		["usage-child", { id: "usage-child", parentID: "attached-root", title: "Usage child", time: { created: 1_350, updated: 1_351 } }],
		["other-root", { id: "other-root", title: "Other client", time: { created: 3_000, updated: 3_001 } }],
		["other-child", { id: "other-child", parentID: "other-root", title: "Other child", time: { created: 3_100, updated: 3_101 } }],
		["spoof-child", { id: "spoof-child", parentID: "other-root", title: "Spoofed child", time: { created: 3_200, updated: 3_201 } }],
	]);
	let exposeLateChild = false;
	const loadedSessionIds: string[] = [];
	const openCodeClient = {
		session: {
			list: async () => [sessions.get("attached-root"), sessions.get("next-root"), sessions.get("other-root")],
			status: async () => ({
				"attached-root": { type: "busy" },
				"next-root": { type: "idle" },
				"other-root": { type: "idle" },
				child: { type: "idle" },
				grandchild: { type: "idle" },
			}),
			get: async ({ path: requestPath }: { path: { id: string } }) => {
				loadedSessionIds.push(requestPath.id);
				return sessions.get(requestPath.id);
			},
			children: async ({ path: requestPath }: { path: { id: string } }) => {
				const children = [...sessions.values()].filter(info =>
					info.parentID === requestPath.id && (info.id !== "late-child" || exposeLateChild),
				);
				return requestPath.id === "attached-root"
					? [...children, sessions.get("spoof-child")]
					: children;
			},
		},
	};
	const plugin = await YtttAgentStatusPlugin({ client: openCodeClient });
	const openCodeEvents = () => [...delivered.values()].filter(delivery => delivery.path === "/hook/opencode");
	const openCodeTokenUsages = () => {
		const usages: Record<string, unknown>[] = [];
		for (const delivery of openCodeEvents()) {
			if (delivery.event !== "token_usage") continue;
			const usage = delivery.payload.usage;
			if (usage && typeof usage === "object" && !Array.isArray(usage)) {
				usages.push(usage as Record<string, unknown>);
			}
		}
		return usages;
	};
	const boardExecutions = (): Record<string, unknown>[] => openCodeEvents().flatMap(delivery => {
		const board = delivery.payload.board;
		if (!board || typeof board !== "object" || (board as Record<string, unknown>).kind !== "execution") return [];
		const execution = (board as Record<string, unknown>).execution;
		return execution && typeof execution === "object" ? [execution as Record<string, unknown>] : [];
	});
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "session_busy" && delivery.payload.sessionID === "attached-root"),
		"OpenCode did not attach the unique busy root",
	);

	// Discovery starts at the selected root, validates each advertised direct
	// parent, and can still resolve a status event arriving before a child cache.
	await plugin.event({ event: { type: "session.updated", properties: { info: sessions.get("other-root") } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "child", status: { type: "busy" } } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "grandchild", status: { type: "busy" } } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "late-child", status: { type: "busy" } } } });
	await waitFor(
		() => ["child", "grandchild", "late-child"].every(childId =>
			openCodeEvents().some(delivery => delivery.event === "child_started" && delivery.payload.childId === childId)),
		"OpenCode did not report the verified child tree",
	);
	assert.ok(loadedSessionIds.includes("late-child"), "a child status before cache must verify its parent chain through session.get");
	assert.equal(
		openCodeEvents().some(delivery => delivery.payload.childId === "spoof-child") ||
			boardExecutions().some(execution => execution.name === "Spoofed child"),
		false,
		"a children response with the wrong parentID must not enter the owned tree",
	);

	await plugin.event({
		event: { type: "message.updated", properties: { info: { id: "child-task", sessionID: "child", role: "user", time: { created: 1_300 } } } },
	});
	await plugin.event({
		event: {
			type: "message.part.updated",
			properties: { sessionID: "child", part: { id: "child-task-text", messageID: "child-task", type: "text", text: "Investigate the tree" } },
		},
	});
	await plugin.event({
		event: {
			type: "session.updated",
			properties: {
				info: { ...sessions.get("child"), title: "Child worker", time: { created: 1_100, updated: 1_400 } },
			},
		},
	});
	const childAssistant = {
		id: "child-assistant",
		sessionID: "child",
		role: "assistant",
		providerID: "openai",
		modelID: "gpt-5.6",
		time: { created: 1_500, completed: 1_600 },
		cost: 0.000321,
		tokens: { input: 120, output: 45, reasoning: 9, cache: { read: 30, write: 4 } },
	};
	await plugin.event({ event: { type: "message.updated", properties: { info: childAssistant } } });
	await plugin.event({ event: { type: "message.updated", properties: { info: childAssistant } } });
	const rootAssistant = {
		id: "root-assistant",
		sessionID: "attached-root",
		role: "assistant",
		providerID: "openai",
		modelID: "gpt-5.6",
		time: { created: 1_550, completed: 1_650 },
		tokens: { input: 10, output: 5, reasoning: 2, cache: { read: 3, write: 1 } },
	};
	const missingAssistantUsage = {
		id: "child-assistant-missing-usage",
		sessionID: "child",
		role: "assistant",
		providerID: "openai",
		modelID: "gpt-5.6",
		time: { created: 1_650, completed: 1_700 },
	};
	await plugin.event({ event: { type: "message.updated", properties: { info: rootAssistant } } });
	await plugin.event({ event: { type: "message.updated", properties: { info: missingAssistantUsage } } });
	const childUsageId = "opencode:child:message:child-assistant";
	const rootUsageId = "opencode:attached-root:message:root-assistant";
	await waitFor(
		() => openCodeTokenUsages().some(usage => usage.id === childUsageId) &&
			openCodeTokenUsages().some(usage => usage.id === rootUsageId),
		"OpenCode did not report native assistant usage",
	);
	assert.deepEqual(
		openCodeTokenUsages()
			.filter(usage => usage.id === childUsageId || usage.id === rootUsageId)
			.map(usage => ({
				id: usage.id,
				tokens: usage.tokens,
				includes_children: usage.includes_children,
				partial: usage.partial,
				parent_id: usage.parent_id,
			})),
		[
			{ id: childUsageId, tokens: 199, includes_children: false, partial: false, parent_id: null },
			{ id: rootUsageId, tokens: 19, includes_children: false, partial: false, parent_id: null },
		],
		"OpenCode usage must use one native message identity per exclusive request",
	);
	assert.equal(
		openCodeTokenUsages().some(usage => usage.id === "opencode:child:message:child-assistant-missing-usage"),
		false,
		"OpenCode messages without native usage must remain unreported rather than becoming zero",
	);
	await plugin.event({
		event: {
			type: "message.part.updated",
			properties: {
				sessionID: "grandchild",
				part: {
					id: "grandchild-tool",
					messageID: "grandchild-message",
					type: "tool",
					callID: "grandchild-tool-call",
					tool: "bash",
					state: { status: "running", input: {}, title: "Inspect files", time: { start: 1_700 } },
				},
			},
		},
	});
	await waitFor(
		() => boardExecutions().some(execution => execution.name === "Grandchild worker" && execution.current_action === "Inspect files"),
		"OpenCode did not project a child tool action onto its child execution",
	);
	await plugin.event({ event: { type: "permission.asked", properties: { sessionID: "grandchild", id: "child-approval" } } });
	await waitFor(
		() => boardExecutions().filter(execution => execution.name === "Grandchild worker").at(-1)?.state === "waiting",
		"a child's approval must make only its own execution wait",
	);
	assert.notEqual(boardExecutions().filter(execution => execution.name === "Attached root").at(-1)?.state, "waiting");
	assert.equal(openCodeEvents().some(delivery => delivery.event === "permission_request" && delivery.payload.sessionID === "grandchild"), false);
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "grandchild", status: { type: "busy" } } } });
	await plugin.event({
		event: {
			type: "message.part.updated",
			properties: {
				sessionID: "grandchild",
				part: {
					id: "grandchild-tool",
					messageID: "grandchild-message",
					type: "tool",
					callID: "grandchild-tool-call",
					tool: "bash",
					state: { status: "completed", input: {}, title: "Inspect files", time: { start: 1_700, end: 1_800 } },
				},
			},
		},
	});
	await waitFor(
		() => boardExecutions().filter(execution => execution.name === "Grandchild worker").at(-1)?.current_action === null
			&& openCodeEvents().some(delivery => delivery.event === "child_updated" && delivery.payload.childId === "grandchild" && delivery.payload.clearCurrentAction === true),
		"OpenCode did not clear a completed child tool action on its retained execution",
	);
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "grandchild", status: { type: "idle" } } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "child", status: { type: "idle" } } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "late-child", status: { type: "idle" } } } });
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "child_finished" && delivery.payload.childId === "child"),
		"OpenCode did not finish a completed child execution",
	);

	const executions = boardExecutions();
	const rootExecution = executions.filter(execution => execution.name === "Attached root").at(-1);
	const childExecution = executions.filter(execution => execution.name === "Child worker").at(-1);
	const grandchildExecution = executions.filter(execution => execution.name === "Grandchild worker").at(-1);
	assert.ok(rootExecution && childExecution && grandchildExecution, "owned tree executions must be retained");
	assert.equal(childExecution.parent_id, rootExecution.id, "child execution must point to the current root execution");
	assert.equal(grandchildExecution.parent_id, childExecution.id, "grandchild execution must point to its real parent execution");
	assert.equal(childExecution.task, "Investigate the tree");
	assert.equal(childExecution.state, "completed");
	assert.equal(childExecution.ended_at, missingAssistantUsage.time.completed, "the latest official assistant/session timestamp must be retained");
	const childTelemetry = childExecution.telemetry as Record<string, unknown>;
	assert.deepEqual(
		{
			model: childTelemetry.model,
			input_tokens: childTelemetry.input_tokens,
			output_tokens: childTelemetry.output_tokens,
			cache_read_tokens: childTelemetry.cache_read_tokens,
			cache_write_tokens: childTelemetry.cache_write_tokens,
			cost_microusd: childTelemetry.cost_microusd,
		},
		{
			model: "gpt-5.6",
			input_tokens: 120,
			output_tokens: 45,
			cache_read_tokens: 30,
			cache_write_tokens: 4,
			cost_microusd: 321,
		},
		"assistant usage must use official fields once without synthesizing a total",
	);
	const childBoardEventsBeforeReplay = boardExecutions()
		.filter(execution => execution.id === childExecution.id).length;
	await plugin.event({ event: { type: "message.updated", properties: { info: childAssistant } } });
	await Bun.sleep(25);
	assert.equal(
		boardExecutions().filter(execution => execution.id === childExecution.id).length,
		childBoardEventsBeforeReplay,
		"a late replayed assistant usage event must not revive or duplicate a finished execution",
	);
	const childExecutionEventsBeforeLateLifecycle = boardExecutions()
		.filter(execution => execution.id === childExecution.id).length;
	const childStartsBeforeLateLifecycle = openCodeEvents()
		.filter(delivery => delivery.event === "child_started" && delivery.payload.childId === "child").length;
	await plugin.event({
		event: {
			type: "session.updated",
			properties: { info: { ...sessions.get("child"), time: { created: 1_100, updated: 1_900 } } },
		},
	});
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "child", status: { type: "idle" } } } });
	await plugin.event({
		event: {
			type: "message.updated",
			properties: { info: { ...childAssistant, id: "late-child-assistant", time: { created: 1_500, completed: 1_600 } } },
		},
	});
	await plugin.event({
		event: {
			type: "message.part.updated",
			properties: {
				sessionID: "child",
				part: {
					id: "late-child-tool",
					messageID: "late-child-message",
					type: "tool",
					callID: "late-child-tool-call",
					tool: "bash",
					state: { status: "completed", input: {}, time: { start: 1_800, end: 2_000 } },
				},
			},
		},
	});
	await Bun.sleep(25);
	assert.equal(
		boardExecutions().filter(execution => execution.id === childExecution.id).length,
		childExecutionEventsBeforeLateLifecycle,
		"metadata, duplicate idle, and late completion events must not reopen an ended child run",
	);
	assert.equal(
		openCodeEvents().filter(delivery => delivery.event === "child_started" && delivery.payload.childId === "child").length,
		childStartsBeforeLateLifecycle,
		"late lifecycle events must not publish a second child start",
	);

	await plugin.event({ event: { type: "session.status", properties: { sessionID: "usage-child", status: { type: "busy" } } } });
	const zeroUsageMessage = {
		id: "usage-zero",
		sessionID: "usage-child",
		role: "assistant",
		providerID: "openai",
		modelID: "cache-model",
		time: { created: 2_050, completed: 2_051 },
		cost: 0,
		tokens: { input: 0, output: 0, reasoning: 0, cache: { read: 0, write: 0 } },
	};
	const usageOnlyUpdate = {
		...zeroUsageMessage,
		tokens: { input: 2, output: 0, reasoning: 0, cache: { read: 0, write: 0 } },
	};
	const missingUsageMessage = {
		id: "usage-missing",
		sessionID: "usage-child",
		role: "assistant",
		providerID: "openai",
		modelID: "cache-model",
		time: { created: 2_052, completed: 2_053 },
	};
	const partialUsageMessage = {
		id: "usage-partial",
		sessionID: "usage-child",
		role: "assistant",
		providerID: "openai",
		modelID: "cache-model",
		time: { created: 2_054, completed: 2_055 },
		tokens: { input: 3 },
	};
	await plugin.event({ event: { type: "message.updated", properties: { info: zeroUsageMessage } } });
	await plugin.event({ event: { type: "message.updated", properties: { info: usageOnlyUpdate } } });
	await plugin.event({ event: { type: "message.updated", properties: { info: usageOnlyUpdate } } });
	await plugin.event({ event: { type: "message.updated", properties: { info: missingUsageMessage } } });
	await plugin.event({ event: { type: "message.updated", properties: { info: partialUsageMessage } } });
	const zeroUsageId = "opencode:usage-child:message:usage-zero";
	const partialUsageId = "opencode:usage-child:message:usage-partial";
	await waitFor(
		() => openCodeTokenUsages().filter(usage => usage.id === zeroUsageId).length === 2 &&
			openCodeTokenUsages().some(usage => usage.id === partialUsageId),
		"OpenCode usage-only counter update was not delivered",
	);
	assert.deepEqual(
		openCodeTokenUsages()
			.filter(usage => usage.id === zeroUsageId)
			.map(usage => ({ tokens: usage.tokens, partial: usage.partial })),
		[{ tokens: 0, partial: false }, { tokens: 2, partial: false }],
		"OpenCode zero is valid, later native usage updates are cumulative per message, and duplicate delivery is ignored",
	);
	assert.deepEqual(
		openCodeTokenUsages()
			.filter(usage => usage.id === partialUsageId)
			.map(usage => ({ tokens: usage.tokens, partial: usage.partial })),
		[{ tokens: 3, partial: true }],
		"partially populated native OpenCode usage must retain only known components and mark coverage partial",
	);
	assert.equal(
		openCodeTokenUsages().some(usage => usage.id === "opencode:usage-child:message:usage-missing"),
		false,
		"OpenCode missing usage must not be represented by a zero counter",
	);
	for (let index = 0; index < 513; index += 1) {
		await plugin.event({
			event: {
				type: "message.updated",
				properties: {
					info: {
						id: `usage-${index}`,
						sessionID: "usage-child",
						role: "assistant",
						providerID: "openai",
						modelID: "cache-model",
						time: { created: 2_100 + index },
						cost: 0,
						tokens: { input: 1, output: 0, reasoning: 0, cache: { read: 0, write: 0 } },
					},
				},
			},
		});
	}
	await waitFor(
		() => boardExecutions().some(execution =>
			execution.name === "Usage child" &&
			(execution.telemetry as Record<string, unknown>).input_tokens === 518),
		"OpenCode usage aggregation dropped totals when its per-message cache evicted",
	);
	await plugin.event({
		event: {
			type: "message.updated",
			properties: {
				info: {
					id: "usage-0",
					sessionID: "usage-child",
					role: "assistant",
					providerID: "openai",
					modelID: "cache-model",
					time: { created: 2_100 },
					cost: 0,
					tokens: { input: 1, output: 0, reasoning: 0, cache: { read: 0, write: 0 } },
				},
			},
		},
	});
	await Bun.sleep(25);
	assert.equal(
		(boardExecutions().filter(execution => execution.name === "Usage child").at(-1)?.telemetry as Record<string, unknown>).input_tokens,
		518,
		"a replayed evicted usage message must not change the settled aggregate",
	);

	await plugin.event({ event: { type: "session.status", properties: { sessionID: "other-child", status: { type: "busy" } } } });
	await plugin.event({
		event: {
			type: "message.part.updated",
			properties: {
				sessionID: "attached-root",
				part: { id: "root-tool", messageID: "root-message", type: "tool", tool: "bash", state: { status: "completed", input: {} } },
			},
		},
	});
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "tool_execution_end" && delivery.payload.sessionID === "attached-root"),
		"OpenCode root tool completion did not deliver",
	);
	assert.equal(
		openCodeEvents().some(delivery => delivery.payload.sessionID === "other-root" || delivery.payload.childId === "other-child") ||
			boardExecutions().some(execution => execution.name === "Other child"),
		false,
		"unrelated roots and their descendants must not enter the selected tree",
	);

	// Server events do not identify the active TUI session. A user message from
	// another root is therefore not provenance and cannot switch ownership.
	await plugin.event({
		event: { type: "message.updated", properties: { info: { id: "next-user", sessionID: "next-root", role: "user", time: { created: 2_010 } } } },
	});
	await plugin.event({
		event: { type: "message.part.updated", properties: { sessionID: "next-root", part: { id: "next-text", messageID: "next-user", type: "text", text: "continue" } } },
	});
	await Bun.sleep(25);
	assert.equal(
		openCodeEvents().some(delivery => delivery.payload.sessionID === "next-root"),
		false,
		"another root's user message must not replace the selected yttt root",
	);

	await plugin.event({ event: { type: "session.status", properties: { sessionID: "attached-root", status: { type: "idle" } } } });
	await waitFor(
		() => boardExecutions().filter(execution => execution.name === "Attached root").at(-1)?.ended_at != null,
		"root idle must finish its execution before detached child activity",
	);
	const finishedRoot = boardExecutions().filter(execution => execution.name === "Attached root").at(-1)!;
	const detachedStart = Number(finishedRoot.ended_at) + 1;
	await plugin.event({ event: { type: "session.created", properties: { info: {
		id: "detached-child", parentID: "attached-root", title: "Detached worker",
		time: { created: detachedStart, updated: detachedStart },
	} } } });
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "child_started" && delivery.payload.childId === "detached-child"),
		"verified detached child activity must remain observable",
	);
	const detached = boardExecutions().filter(execution => execution.name === "Detached worker").at(-1)!;
	assert.equal(detached.parent_id, finishedRoot.id, "a child must retain the observed parent execution, not invent another parent run");
	assert.equal(boardExecutions().filter(execution => execution.name === "Attached root").at(-1)?.state, "completed", "child activity cannot restart its ended parent");
	const ownedRootIdles = openCodeEvents().filter(delivery => delivery.event === "session_idle" && delivery.payload.sessionID === "attached-root").length;

	// A new root can be selected only after the owned root is deleted, then a
	// root session is explicitly created by the observed OpenCode process.
	await plugin.event({ event: { type: "session.deleted", properties: { info: sessions.get("attached-root") } } });
	await plugin.event({ event: { type: "session.created", properties: { info: sessions.get("next-root") } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "attached-root", status: { type: "idle" } } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "next-root", status: { type: "busy" } } } });
	await plugin.event({ event: { type: "permission.asked", properties: { sessionID: "next-root", description: "approve" } } });
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "permission_request" && delivery.payload.sessionID === "next-root"),
		"OpenCode approval prompt did not await ordered delivery",
	);
	assert.equal(
		openCodeEvents().filter(delivery => delivery.event === "session_idle" && delivery.payload.sessionID === "attached-root").length,
		ownedRootIdles,
		"late idle from a deleted root must not finish the replacement root",
	);
	assert.deepEqual(
		openCodeEvents().filter(delivery => delivery.payload.sessionID === "next-root").map(delivery => delivery.event),
		["session_start", "session_busy", "permission_request"],
		"replacement root lifecycle must be FIFO without accepting an unrelated prompt",
	);
	const nextExecution = boardExecutions().filter(execution => execution.name === "Resumed root").at(-1);
	assert.ok(nextExecution, "the replacement root must have a fresh retained execution");
	await plugin.event({ event: { type: "session.deleted", properties: { info: sessions.get("next-root") } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "next-root", status: { type: "busy" } } } });
	await waitFor(
		() => boardExecutions().some(execution => execution.id === nextExecution.id && execution.state === "interrupted" && execution.ended_at !== null),
		"OpenCode deletion did not finish the selected execution without reselecting it",
	);

	for (const route of ["/hook/pi", "/hook/opencode"]) {
		assert.ok(
			received.filter(delivery => delivery.path === route).some(delivery => (responseAttempts.get(`${route}\u0000${delivery.streamId}\u0000${delivery.sequence}`) ?? 0) >= 5),
			`${route} must retry a transient failure and lost acknowledgement`,
		);
		const streams = new Map<string, number[]>();
		for (const delivery of [...delivered.values()].filter(delivery => delivery.path === route)) {
			const sequences = streams.get(delivery.streamId) ?? [];
			sequences.push(delivery.sequence);
			streams.set(delivery.streamId, sequences);
		}
		for (const sequences of streams.values()) {
			assert.deepEqual(sequences, Array.from({ length: sequences.length }, (_, index) => index + 1),
				`${route} deliveries must stay FIFO without gaps`);
		}
	}

	// Discovery may complete after a live session has already been selected.
	const discovery = Promise.withResolvers<Record<string, unknown>[]>();
	const racePlugin = await YtttAgentStatusPlugin({
		client: { session: {
			list: () => discovery.promise,
			status: async () => ({ "old-root": { type: "busy" } }),
		} },
	});
	await racePlugin.event({ event: { type: "session.created", properties: { info: { id: "live-root" } } } });
	discovery.resolve([{ id: "old-root" }]);
	await Bun.sleep(25);
	await racePlugin.event({ event: { type: "session.status", properties: { sessionID: "live-root", status: "busy" } } });
	await waitFor(() => openCodeEvents().some(delivery => delivery.event === "session_busy" && delivery.payload.sessionID === "live-root"),
		"late discovery replaced the live selected root");
	assert.equal(openCodeEvents().some(delivery => delivery.payload.sessionID === "old-root"), false);

	const originalError = console.error;
	const diagnostics: string[] = [];
	console.error = (...args: unknown[]) => { diagnostics.push(args.join(" ")); };
	try {
		process.env.YTTT_AGENT_HOOK_SCOPE = "unauthorized-pi";
		const unauthorizedPi = new MockPi();
		loadPi()(unauthorizedPi);
		const unauthorizedContext = context("unauthorized-root");
		await unauthorizedPi.emit("session_start", unauthorizedContext);
		await unauthorizedPi.emit("agent_start", unauthorizedContext);
		await waitFor(() => diagnostics.length === 1, "Pi did not report permanent delivery failure");
		await unauthorizedPi.emit("session_shutdown", unauthorizedContext);

		process.env.YTTT_AGENT_HOOK_SCOPE = "unauthorized-opencode";
		const unauthorizedPlugin = await new Function(
			`${openCodeSource.replace("export const YtttAgentStatusPlugin", "const YtttAgentStatusPlugin")}\nreturn YtttAgentStatusPlugin;`,
		)()({ client: { session: { list: async () => [], status: async () => ({}) } } });
		await unauthorizedPlugin.event({ event: { type: "session.created", properties: { info: { id: "unauthorized-root" } } } });
		await waitFor(() => diagnostics.length === 2, "OpenCode did not report permanent delivery failure");
		await unauthorizedPlugin.event({ event: { type: "session.status", properties: { sessionID: "unauthorized-root", status: "busy" } } });
		await Bun.sleep(1200);
		assert.equal(unauthorizedRequests.get("unauthorized-pi"), 1, "Pi must stop, not retry a rejected stream");
		assert.equal(unauthorizedRequests.get("unauthorized-opencode"), 1, "OpenCode must stop, not retry a rejected stream");
		assert.equal(diagnostics.length, 2, "one diagnostic per stopped stream");
		assert.equal(diagnostics.some(message => message.includes("test-token")), false);
	} finally {
		console.error = originalError;
		process.env.YTTT_AGENT_HOOK_SCOPE = "same-scope";
	}
	const streamsBeforeReload = new Set(openCodeEvents().map(delivery => delivery.streamId));
	const ReloadedOpenCodePlugin = new Function(
		`${openCodeSource.replace("export const YtttAgentStatusPlugin", "const YtttAgentStatusPlugin")}\nreturn YtttAgentStatusPlugin;`,
	)();
	const reloadedOpenCode = await ReloadedOpenCodePlugin({ client: openCodeClient });
	await waitFor(
		() => openCodeEvents().some(
			delivery => delivery.event === "session_busy" && !streamsBeforeReload.has(delivery.streamId),
		),
		"reloaded OpenCode plugin did not attach the active root",
	);
	await reloadedOpenCode.event({ event: { type: "message.updated", properties: { info: childAssistant } } });
	await waitFor(
		() => openCodeTokenUsages().filter(usage => usage.id === childUsageId).length === 2,
		"reloaded OpenCode plugin did not replay the native message billing identity",
	);
	assert.equal(
		openCodeTokenUsages().filter(usage => usage.id === childUsageId).length,
		2,
		"replayed OpenCode usage must retain the message identity instead of the delivery stream id",
	);

	// Legacy OSC transport needs the same ownership rule as HTTP.
	delete process.env.YTTT_AGENT_HOOK_ENDPOINT;
	process.env.YTTT_AGENT_INSTANCE_ID = "osc-instance";
	process.env.YTTT_AGENT_TOKEN = "osc-test-token";
	process.env.YTTT_AGENT_GENERATION = "1";
	const frames: string[] = [];
	const originalWrite = process.stdout.write;
	process.stdout.write = ((chunk: unknown) => { frames.push(String(chunk)); return true; }) as typeof process.stdout.write;
	try {
		const oscRoot = new MockPi();
		loadPi()(oscRoot);
		loadPi()(oscRoot);
		await oscRoot.emit("session_start", root);
		const oscChild = new MockPi();
		loadPi()(oscChild);
		await oscChild.emit("session_start", child);
		await oscChild.emit("session_shutdown", child);
		await oscRoot.emit("agent_start", root);
		await oscRoot.emit("session_shutdown", root);
	} finally {
		process.stdout.write = originalWrite;
	}
	const oscEvents = frames.map(frame => {
		const encoded = frame.slice("\u001b]2;yttt-agent-v1:".length, -1);
		const decoded = JSON.parse(Buffer.from(encoded, "base64url").toString());
		return [decoded.event, decoded.payload.sessionId];
	});
	assert.deepEqual(oscEvents, [
		["session_start", "pi-root"], ["agent_start", "pi-root"], ["session_shutdown", "pi-root"],
	], "OSC child and duplicate copies must not publish root lifecycle events");

	console.log("PASS: OpenCode message-identity token usage, root filtering/attachment/switching, and Pi duplicate/reload lifecycle preserve ordered delivery");
} finally {
	server.stop(true);
}
