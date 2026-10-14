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

async function waitFor(predicate: () => boolean, message: string): Promise<void> {
	const deadline = performance.now() + 5_000;
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
		await cp(source, extensionPath);
		await cp(source, duplicateExtensionPath);

		const eventBus = new eventBusModule.EventBus();
		const loaded = await loader.loadExtensions([extensionPath, duplicateExtensionPath], root, eventBus);
		assert.deepEqual(loaded.errors, [], `extension load errors: ${JSON.stringify(loaded.errors)}`);
		const sessionManager = SessionManager.inMemory(root);
		loaded.runtime.appendEntry = (customType: string, data?: unknown) => {
			sessionManager.appendCustomEntry(customType, data);
		};
		const context = {
			sessionManager,
			model: { id: "root-model" },
		};

		await emit(loaded, "session_start", { type: "session_start" }, context);
		await emit(loaded, "before_agent_start", { type: "before_agent_start", prompt: "delegate two tasks" }, context);
		await emit(loaded, "agent_start", { type: "agent_start" }, context);

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
		const followUpTelemetry = asRecord(latestExecution(secondChildAId).telemetry)!;
		assert.equal(followUpTelemetry.tokens, 5);
		assert.equal(followUpTelemetry.model, undefined, "follow-up must not inherit prior model telemetry");
		assert.equal(followUpTelemetry.cost_microusd, undefined, "follow-up must not inherit prior cost telemetry");

		console.log("PASS: OMP source bridge preserves partial/final child telemetry, optional self-estimates, run isolation, and terminal lifecycle boundaries");
	} finally {
		await rm(root, { recursive: true, force: true });
	}
} finally {
	server.stop(true);
}
