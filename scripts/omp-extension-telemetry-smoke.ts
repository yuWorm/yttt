import assert from "node:assert/strict";
import { cp, mkdtemp, mkdir, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

type Delivery = {
	protocol: number;
	streamId: string;
	sequence: number;
	event: string;
	payload: Record<string, unknown>;
};

type Handler = (event: Record<string, unknown>, context: Record<string, unknown>) => Promise<void> | void;

const source = process.argv[2];
const packageRoot = process.env.OMP_PACKAGE_ROOT;
if (!source || !packageRoot) {
	throw new Error(
		"usage: OMP_PACKAGE_ROOT=/path/to/@oh-my-pi/pi-coding-agent bun scripts/omp-extension-telemetry-smoke.ts <crates/yttt-agent-providers/src/yttt-agent-extension.ts>",
	);
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
	return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

async function waitFor(predicate: () => boolean, message: string, timeout = 5_000): Promise<void> {
	const deadline = performance.now() + timeout;
	while (!predicate()) {
		if (performance.now() >= deadline) throw new Error(message);
		await Bun.sleep(5);
	}
}

// The installed OMP package is a runtime-selected test input; static imports
// cannot exercise its extension loader and EventBus boundary.
const loader = await import(
	`${pathToFileURL(path.join(packageRoot, "src/extensibility/extensions/loader.ts")).href}?telemetry-smoke=${Date.now()}`,
);
const eventBusModule = await import(
	`${pathToFileURL(path.join(packageRoot, "src/utils/event-bus.ts")).href}?telemetry-smoke=${Date.now()}`,
);
const { SessionManager } = await import(
	pathToFileURL(path.join(packageRoot, "src/session/session-manager.ts")).href,
);
const deliveries: Delivery[] = [];
const server = Bun.serve({
	port: 0,
	async fetch(request) {
		const delivery = await request.json() as Delivery;
		deliveries.push(delivery);
		return Response.json({ acceptedSequence: delivery.sequence });
	},
});

function executionEvents(): Record<string, unknown>[] {
	const executions: Record<string, unknown>[] = [];
	for (const delivery of deliveries) {
		if (delivery.event !== "board_event") continue;
		const board = asRecord(delivery.payload.board);
		if (board?.kind !== "execution") continue;
		const execution = asRecord(board.execution);
		if (execution) executions.push(execution);
	}
	return executions;
}

function executionEventsFor(childId: string): Record<string, unknown>[] {
	return executionEvents().filter(execution => typeof execution.id === "string" && execution.id.includes(`:${childId}:`));
}

function latestExecution(id: string): Record<string, unknown> {
	const events = executionEvents().filter(execution => execution.id === id);
	assert.ok(events.length > 0, `execution ${id} must be observed`);
	return events.at(-1)!;
}

function tokenUsageEvents(): Record<string, unknown>[] {
	const usages: Record<string, unknown>[] = [];
	for (const delivery of deliveries) {
		if (delivery.event !== "token_usage") continue;
		const usage = asRecord(delivery.payload.usage);
		if (usage) usages.push(usage);
	}
	return usages;
}

async function emit(
	loaded: { extensions: Array<{ handlers: Map<string, unknown[]> }> },
	name: string,
	event: Record<string, unknown>,
	context: Record<string, unknown>,
): Promise<void> {
	for (const extension of loaded.extensions) {
		for (const handler of extension.handlers.get(name) ?? []) {
			await (handler as Handler)(event, context);
		}
	}
}

try {
	process.env.YTTT_AGENT_HOOK_ENDPOINT = `http://127.0.0.1:${server.port}`;
	process.env.YTTT_AGENT_HOOK_TOKEN = "test-token";
	process.env.YTTT_AGENT_HOOK_SCOPE = "omp-telemetry-smoke";
	const root = await mkdtemp(path.join(tmpdir(), "yttt-omp-telemetry-"));
	try {
		const modules = path.join(root, "node_modules", "@oh-my-pi");
		await mkdir(modules, { recursive: true });
		await symlink(packageRoot, path.join(modules, "pi-coding-agent"));
		const extensionPath = path.join(root, "yttt-agent-extension.ts");
		const duplicateExtensionPath = path.join(root, "yttt-agent-extension-duplicate.ts");
		const replayExtensionPath = path.join(root, "yttt-agent-extension-replay.ts");
		await cp(source, extensionPath);
		await cp(source, duplicateExtensionPath);
		await cp(source, replayExtensionPath);

		const eventBus = new eventBusModule.EventBus();
		const loaded = await loader.loadExtensions([extensionPath, duplicateExtensionPath], root, eventBus);
		assert.deepEqual(loaded.errors, [], `extension load errors: ${JSON.stringify(loaded.errors)}`);
		const sessionManager = SessionManager.inMemory(root);
		const persistedUsageEntry = (totalTokens: number | undefined) =>
			sessionManager.appendMessage({
				role: "assistant",
				content: [{ type: "text", text: "Persisted assistant usage" }],
				api: "openai-codex-responses",
				provider: "openai-codex",
				model: "gpt-5.6",
				usage: {
					input: 7,
					output: 2,
					cacheRead: 0,
					cacheWrite: 0,
					...(totalTokens === undefined ? {} : { totalTokens }),
				},
				stopReason: "stop",
				timestamp: Date.now(),
			} as never);
		const rootUsageEntryId = persistedUsageEntry(9);
		const zeroUsageEntryId = persistedUsageEntry(0);
		const missingUsageEntryId = persistedUsageEntry(undefined);
		const boundedUsageEntryIds = Array.from({ length: 257 }, (_value, index) => persistedUsageEntry(index + 1));
		// SessionManager normalizes absent usage into zero counters. Inject a missing
		// measurement at the read boundary to exercise older/incomplete observations,
		// rather than accidentally testing a canonical SDK-reported zero.
		const nativeBranch = sessionManager.getBranch.bind(sessionManager);
		sessionManager.getBranch = (...args: unknown[]) => nativeBranch(...args).map((entry: any) =>
			entry.id === missingUsageEntryId
				? { ...entry, message: { ...entry.message, usage: undefined } }
				: entry,
		);
		loaded.runtime.appendEntry = (customType: string, data?: unknown) => {
			sessionManager.appendCustomEntry(customType, data);
		};
		let idle = true;
		const context = {
			sessionManager,
			model: { id: "root-model" },
			isIdle: () => idle,
		};

		await emit(loaded, "session_start", { type: "session_start" }, context);
		const sessionId = sessionManager.getSessionId();
		const rootUsageId = `omp:${sessionId}:message:${rootUsageEntryId}`;
		const zeroUsageId = `omp:${sessionId}:message:${zeroUsageEntryId}`;
		const missingUsageId = `omp:${sessionId}:message:${missingUsageEntryId}`;
		const boundedUsageId = `omp:${sessionId}:message:${boundedUsageEntryIds[0]!}`;
		await waitFor(
			() => tokenUsageEvents().some(usage => usage.id === rootUsageId) &&
				tokenUsageEvents().some(usage => usage.id === zeroUsageId) &&
				tokenUsageEvents().some(usage => usage.id === boundedUsageId),
			"persisted root assistant usage was not replayed",
		);
		assert.deepEqual(
			tokenUsageEvents()
				.filter(usage => usage.id === rootUsageId || usage.id === zeroUsageId)
				.map(usage => ({
					id: usage.id,
					tokens: usage.tokens,
					includes_children: usage.includes_children,
					partial: usage.partial,
					parent_id: usage.parent_id,
				})),
			[
				{ id: rootUsageId, tokens: 9, includes_children: false, partial: false, parent_id: null },
				{ id: zeroUsageId, tokens: 0, includes_children: false, partial: false, parent_id: null },
			],
			"persisted root request usage must retain a native entry identity, report zero, and exclude children",
		);
		assert.equal(
			tokenUsageEvents().some(usage => usage.id === boundedUsageId && usage.tokens === 1),
			true,
			"usage replay must scan entries older than the 256-message board display bound",
		);
		assert.equal(
			tokenUsageEvents().some(usage => usage.id === missingUsageId),
			false,
			"missing native totalTokens must remain unreported rather than becoming zero",
		);
		idle = false;
		await emit(loaded, "before_agent_start", { type: "before_agent_start", prompt: "delegate two tasks" }, context);
		await emit(loaded, "agent_start", { type: "agent_start" }, context);

		const activity = () => deliveries.filter(delivery => delivery.event === "session_updated");
		await waitFor(() => activity().some(delivery => delivery.payload.isIdle === false), "busy turn lost its periodic activity", 35_000);
		await Bun.sleep(100);
		assert.equal(activity().length, 1, "duplicate extensions must not duplicate the activity timer");

		const rootUsageBeforeChildren = tokenUsageEvents();
		const childAStarted = {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			description: "Analyze the bridge",
			status: "started",
			index: 0,
		};
		eventBus.emit("task:subagent:lifecycle", childAStarted);
		await waitFor(
			() => executionEventsFor("child-a").length === 1,
			"duplicate extension paths must leave one root event owner",
		);
		const firstChildAId = executionEventsFor("child-a")[0]!.id as string;

		// The root turn ends while detached work continues. The child progress must
		// remain attached to the ended root instead of starting a new root run.
		idle = true;
		await emit(loaded, "agent_end", { type: "agent_end", willContinue: false, messages: [] }, context);
		const childBStarted = {
			id: "child-b",
			agent: "scout",
			agentSource: "bundled",
			description: "Inspect isolation",
			status: "started",
			index: 1,
		};
		eventBus.emit("task:subagent:lifecycle", childBStarted);
		await waitFor(() => executionEventsFor("child-b").length === 1, "child B start after root end was dropped");
		const childBId = executionEventsFor("child-b")[0]!.id as string;

		eventBus.emit("task:subagent:progress", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			task: "Analyze the bridge",
			index: 0,
			sessionFile: "/tmp/omp-child-a.jsonl",
			progress: {
				id: "child-a",
				status: "running",
				resolvedModel: "openai/gpt-5.6:high",
				tokens: 101,
				requests: 2,
				contextTokens: 50,
				contextWindow: 200,
				toolCount: 3,
				durationMs: 1_000,
				cost: 0.0000016,
				completionPercent: 40,
				currentTool: "read",
				currentToolIntent: "Read bridge source",
			},
		});
		eventBus.emit("task:subagent:progress", {
			id: "child-b",
			agent: "scout",
			agentSource: "bundled",
			task: "Inspect isolation",
			index: 1,
			progress: {
				id: "child-b",
				status: "running",
				tokens: 11,
				requests: 1,
				toolCount: 1,
				durationMs: 200,
				currentTool: "grep",
			},
		});
		eventBus.emit("task:subagent:progress", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			task: "Analyze the bridge",
			index: 0,
			sessionFile: "/tmp/omp-child-a.jsonl",
			progress: {
				id: "child-a",
				status: "running",
				tokens: 222,
				requests: 4,
				toolCount: 5,
				durationMs: 2_000,
				currentTool: "edit",
			},
		});
		await waitFor(
			() => latestExecution(firstChildAId).current_action === "edit" && latestExecution(childBId).current_action === "grep",
			"partial child telemetry did not reach the board",
		);

		const partialA = latestExecution(firstChildAId);
		const partialATelemetry = asRecord(partialA.telemetry)!;
		assert.equal(partialATelemetry.model, "openai/gpt-5.6:high");
		assert.equal(partialATelemetry.tokens, 222);
		assert.equal(partialATelemetry.cost_microusd, 2, "USD cost must be rounded once to microusd");
		assert.equal(partialATelemetry.self_estimated_completion, 40);
		assert.equal(partialA.current_action, "edit");
		const partialBTelemetry = asRecord(latestExecution(childBId).telemetry)!;
		assert.equal(partialBTelemetry.model, undefined, "missing model must remain absent");
		assert.equal(partialBTelemetry.cost_microusd, undefined, "missing cost must remain absent");
		assert.equal(partialBTelemetry.self_estimated_completion, undefined, "missing self-estimate must remain absent");
		assert.deepEqual(
			tokenUsageEvents(),
			rootUsageBeforeChildren,
			"child progress without a stable invocation ID must not enter durable billing",
		);

		eventBus.emit("task:subagent:progress", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			task: "Analyze the bridge",
			index: 0,
			progress: {
				id: "child-a",
				status: "running",
				tokens: 222,
				requests: 4,
				toolCount: 5,
				durationMs: 2_100,
			},
		});
		await waitFor(
			() =>
				latestExecution(firstChildAId).current_action === null &&
				deliveries.some(
					delivery =>
						delivery.event === "child_updated" &&
						delivery.payload.childId === "child-a" &&
						delivery.payload.clearCurrentAction === true,
				),
			"reported action clear did not reach the board and native child bridge",
		);

		eventBus.emit("task:subagent:lifecycle", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			status: "completed",
			index: 0,
		});
		await waitFor(
			() => latestExecution(firstChildAId).ended_at !== null && latestExecution(firstChildAId).ended_at !== undefined,
			"child A terminal lifecycle did not reach the board",
		);
		const terminalA = latestExecution(firstChildAId);
		assert.equal(terminalA.current_action, null, "terminal child execution must clear current action");
		assert.equal(asRecord(terminalA.telemetry)?.tokens, 222, "terminal child must retain partial telemetry");
		assert.equal(asRecord(terminalA.telemetry)?.cost_microusd, 2, "terminal child must retain reported cost");

		const executionsBeforeLateProgress = executionEvents().length;
		eventBus.emit("task:subagent:progress", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			task: "stale update",
			index: 0,
			progress: { id: "child-a", status: "running", tokens: 999, currentTool: "write" },
		});
		await Bun.sleep(25);
		assert.equal(executionEvents().length, executionsBeforeLateProgress, "late progress must not revive a completed run");

		// A kept-alive agent can receive a real follow-up turn under the same OMP
		// child id. It must get a fresh board identity and must not inherit telemetry.
		eventBus.emit("task:subagent:lifecycle", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			description: "Follow up on the bridge",
			status: "started",
			index: 0,
		});
		await waitFor(() => executionEventsFor("child-a").some(execution => execution.id !== firstChildAId), "follow-up did not get a new run id");
		const secondChildAId = executionEventsFor("child-a").find(execution => execution.id !== firstChildAId)!.id as string;
		eventBus.emit("task:subagent:progress", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			task: "Follow up on the bridge",
			index: 0,
			sessionFile: "/tmp/omp-child-a.jsonl",
			progress: { id: "child-a", status: "running", tokens: 5, requests: 1, toolCount: 0, durationMs: 50 },
		});
		eventBus.emit("task:subagent:lifecycle", {
			id: "child-a",
			agent: "task",
			agentSource: "project",
			status: "completed",
			index: 0,
		});
		await waitFor(
			() => latestExecution(secondChildAId).ended_at !== null && latestExecution(secondChildAId).ended_at !== undefined,
			"follow-up terminal lifecycle did not reach the board",
		);
		assert.notEqual(secondChildAId, firstChildAId, "same child id must create a per-run execution id");
		assert.deepEqual(
			tokenUsageEvents(),
			rootUsageBeforeChildren,
			"reset counters in a reused child session must not be merged into project totals",
		);
		const followUpTelemetry = asRecord(latestExecution(secondChildAId).telemetry)!;
		assert.equal(followUpTelemetry.tokens, 5);
		assert.equal(followUpTelemetry.model, undefined, "follow-up must not inherit prior model telemetry");
		assert.equal(followUpTelemetry.cost_microusd, undefined, "follow-up must not inherit prior cost telemetry");

		await waitFor(() => activity().some(delivery => delivery.payload.isIdle === true), "idle turn was not confirmed", 35_000);
		await emit(loaded, "session_shutdown", { type: "session_shutdown" }, context);
		const activityBeforeShutdown = activity().length;
		await Bun.sleep(31_000);
		assert.equal(activity().length, activityBeforeShutdown, "shutdown must stop activity observations");

		const reloaded = await loader.loadExtensions([replayExtensionPath], root, eventBus);
		assert.deepEqual(reloaded.errors, [], `reloaded extension errors: ${JSON.stringify(reloaded.errors)}`);
		await emit(reloaded, "session_start", { type: "session_start", reason: "reload" }, context);
		await waitFor(
			() => tokenUsageEvents().filter(usage => usage.id === rootUsageId).length === 2 &&
				tokenUsageEvents().filter(usage => usage.id === zeroUsageId).length === 2,
			"session reload did not replay persisted root usage with its original identities",
		);
		assert.equal(
			tokenUsageEvents().filter(usage => usage.id === rootUsageId).length,
			2,
			"replayed root usage must use the same stable billing subject rather than inventing another total",
		);
		await emit(reloaded, "session_shutdown", { type: "session_shutdown" }, context);
		console.log("PASS: OMP source bridge preserves child telemetry, replay-safe token usage, and lifecycle boundaries with one activity owner");
	} finally {
		await rm(root, { recursive: true, force: true });
	}
} finally {
	server.stop(true);
}
