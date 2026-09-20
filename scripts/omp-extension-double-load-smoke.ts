import assert from "node:assert/strict";
import { cp, mkdtemp, mkdir, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

type Delivery = {
	event?: unknown;
	payload?: { willContinue?: unknown };
	sequence?: unknown;
	streamId?: unknown;
	scope: string;
};

const childMode = process.argv[2] === "--child";
const source = process.argv[childMode ? 3 : 2];
const packageRoot = process.env.OMP_PACKAGE_ROOT;
if (!source || !packageRoot) {
	throw new Error(
		"usage: OMP_PACKAGE_ROOT=/path/to/@oh-my-pi/pi-coding-agent bun scripts/omp-extension-double-load-smoke.ts <extension.ts>",
	);
}

// The copies' paths are test inputs, so static imports cannot exercise the loader boundary.
const loader = await import(
	`${pathToFileURL(path.join(packageRoot, "src/extensibility/extensions/loader.ts")).href}?run=${Date.now()}`,
);
const eventBusModule = await import(
	`${pathToFileURL(path.join(packageRoot, "src/utils/event-bus.ts")).href}?run=${Date.now()}`,
);

async function loadTwiceAndEmit(sessionId: string, includeReload: boolean): Promise<string[]> {
	const root = await mkdtemp(path.join(tmpdir(), "yttt-omp-extension-"));
	try {
		const modules = path.join(root, "node_modules", "@oh-my-pi");
		await mkdir(modules, { recursive: true });
		await symlink(packageRoot, path.join(modules, "pi-coding-agent"));
		const first = path.join(root, "first.ts");
		const second = path.join(root, "second.ts");
		await cp(source, first);
		await cp(source, second);

		const eventBus = new eventBusModule.EventBus();
		const loaded = await loader.loadExtensions([first, second], root, eventBus);
		assert.deepEqual(loaded.errors, [], `extension load errors: ${JSON.stringify(loaded.errors)}`);

		const ctx = {
			sessionManager: {
				getSessionId: () => sessionId,
				getSessionFile: () => `/tmp/${sessionId}.jsonl`,
			},
			model: { id: "model-1" },
		};
		const emittedEvents: Array<[string, Record<string, unknown>]> = [
			["session_start", { type: "session_start" }],
			["before_agent_start", { type: "before_agent_start", prompt: "continue working" }],
			["agent_start", { type: "agent_start" }],
			["agent_end", { type: "agent_end", willContinue: true }],
		];
		if (includeReload) {
			// OMP's explicit switch event is a new root-session boundary.
			emittedEvents.push(["session_switch", { type: "session_switch" }]);
		} else {
			emittedEvents.push(["session_shutdown", { type: "session_shutdown" }]);
		}
		for (const [name, event] of emittedEvents) {
			const handlers = loaded.extensions.flatMap((extension: { handlers: Map<string, unknown[]> }) =>
				extension.handlers.get(name) ?? [],
			);
			for (const handler of handlers) {
				await (handler as (event: Record<string, unknown>, context: typeof ctx) => Promise<void>)(event, ctx);
			}
			if (name === "agent_start" && includeReload) {
				// OMP task sessions load the parent's paths with a fresh EventBus,
				// but share process.env and therefore the parent's terminal scope.
				const taskSession = await loader.loadExtensions(
					[first, second], root, new eventBusModule.EventBus(),
				);
				assert.deepEqual(taskSession.errors, []);
				const taskContext = {
					...ctx,
					sessionManager: { getSessionId: () => "task-session" },
				};
				for (const taskEvent of ["session_start", "agent_start", "agent_end", "session_shutdown"]) {
					for (const extension of taskSession.extensions) {
						for (const handler of extension.handlers.get(taskEvent) ?? []) {
							await handler({ type: taskEvent }, taskContext);
						}
					}
				}
			}
		}
		return emittedEvents.map(([name]) => name);
	} finally {
		await rm(root, { recursive: true, force: true });
	}
}

if (childMode) {
	process.env.YTTT_AGENT_HOOK_SCOPE = "child-scope";
	await loadTwiceAndEmit("child-session", false);
} else {
	const deliveries: Delivery[] = [];
	const rejected: Delivery[] = [];
	const activeStreams = new Map<string, unknown>();
	const server = Bun.serve({
		port: 0,
		async fetch(request) {
			const delivery = { ...(await request.json() as Omit<Delivery, "scope">), scope: request.headers.get("x-yttt-agent-hook-scope") ?? "" };
			const activeStream = activeStreams.get(delivery.scope);
			if (activeStream !== undefined && activeStream !== delivery.streamId && delivery.sequence !== 1) {
				rejected.push(delivery);
				return Response.json({ error: "retired stream" }, { status: 409 });
			}
			activeStreams.set(delivery.scope, delivery.streamId);
			deliveries.push(delivery);
			return Response.json({ acceptedSequence: delivery.sequence });
		},
	});
	try {
		process.env.YTTT_AGENT_HOOK_ENDPOINT = `http://127.0.0.1:${server.port}`;
		process.env.YTTT_AGENT_HOOK_TOKEN = "test-token";
		process.env.YTTT_AGENT_HOOK_SCOPE = "parent-scope";
		const parentEvents = await loadTwiceAndEmit("parent-session", true);
		const child = Bun.spawn([process.execPath, process.argv[1]!, "--child", source], {
			cwd: process.cwd(),
			env: process.env,
			stdout: "inherit",
			stderr: "inherit",
		});
		const childTimeout = setTimeout(() => child.kill(), 5_000);
		try {
			assert.equal(await child.exited, 0, "child lifecycle delivery must finish without a blocked stream");
		} finally {
			clearTimeout(childTimeout);
		}

		const expectedDeliveries = parentEvents.length + 5;
		const deadline = performance.now() + 1_000;
		while (deliveries.length < expectedDeliveries && performance.now() < deadline) {
			await Bun.sleep(5);
		}
		assert.deepEqual(rejected, [], "one stream per session must never block behind a retired stream");
		assert.deepEqual(
			deliveries.filter(delivery => delivery.scope === "parent-scope").map(delivery => delivery.event),
			parentEvents,
		);
		assert.deepEqual(
			deliveries.filter(delivery => delivery.scope === "child-scope").map(delivery => delivery.event),
			["session_start", "before_agent_start", "agent_start", "agent_end", "session_shutdown"],
		);
		assert.equal(
			deliveries.find(delivery => delivery.scope === "parent-scope" && delivery.event === "agent_end")?.payload?.willContinue,
			true,
			"continuations must remain working",
		);
		console.log("PASS: duplicate paths, in-process task shutdown, reload, and child process preserve the root lifecycle stream");
	} finally {
		server.stop(true);
	}
}
