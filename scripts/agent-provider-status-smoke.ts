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
		["attached-root", { id: "attached-root", title: "Attached root" }],
		["next-root", { id: "next-root", title: "Resumed root" }],
		["child", { id: "child", parentID: "attached-root", title: "Child" }],
		["other-root", { id: "other-root", title: "Other client" }],
	]);
	const plugin = await YtttAgentStatusPlugin({
		client: {
			session: {
				list: async () => [sessions.get("attached-root"), sessions.get("next-root"), sessions.get("other-root")],
				status: async () => ({ "attached-root": { type: "busy" }, "next-root": { type: "idle" }, "other-root": { type: "idle" } }),
				get: async ({ path: requestPath }: { path: { id: string } }) => sessions.get(requestPath.id),
			},
		},
	});
	const openCodeEvents = () => [...delivered.values()].filter(delivery => delivery.path === "/hook/opencode");
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "session_busy" && delivery.payload.sessionID === "attached-root"),
		"OpenCode did not attach the unique busy root",
	);

	// Metadata from another root must not change selection. A child can then
	// create/idle/complete a tool without clobbering the attached root.
	await plugin.event({ event: { type: "session.updated", properties: { info: sessions.get("other-root") } } });
	await plugin.event({ event: { type: "session.created", properties: { info: sessions.get("child") } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "child", status: { type: "idle" } } } });
	await plugin.event({
		event: {
			type: "message.part.updated",
			properties: { sessionID: "child", part: { id: "child-tool", messageID: "child-message", type: "tool", tool: "bash", state: { status: "completed", input: {} } } },
		},
	});
	await plugin.event({
		event: {
			type: "message.part.updated",
			properties: { sessionID: "attached-root", part: { id: "root-tool", messageID: "root-message", type: "tool", tool: "bash", state: { status: "completed", input: {} } } },
		},
	});
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "tool_execution_end" && delivery.payload.sessionID === "attached-root"),
		"OpenCode root tool completion did not deliver",
	);
	assert.equal(
		openCodeEvents().some(delivery => delivery.payload.sessionID === "child"),
		false,
		"child lifecycle/tool events must not enter the root stream",
	);
	assert.equal(
		openCodeEvents().some(delivery => delivery.event === "session_updated" && delivery.payload.sessionID === "other-root"),
		false,
		"unselected root metadata must not switch the active root",
	);

	// A user prompt is explicit active-session evidence for an attached/resumed
	// root. A late idle from the former root remains ignored.
	await plugin.event({
		event: { type: "message.updated", properties: { info: { id: "next-user", sessionID: "next-root", role: "user" } } },
	});
	await plugin.event({
		event: { type: "message.part.updated", properties: { sessionID: "next-root", part: { id: "next-text", messageID: "next-user", type: "text", text: "continue" } } },
	});
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "attached-root", status: { type: "idle" } } } });
	await plugin.event({ event: { type: "session.status", properties: { sessionID: "next-root", status: { type: "busy" } } } });
	await plugin.event({ event: { type: "permission.asked", properties: { sessionID: "next-root", description: "approve" } } });
	await waitFor(
		() => openCodeEvents().some(delivery => delivery.event === "permission_request" && delivery.payload.sessionID === "next-root"),
		"OpenCode approval prompt did not await ordered delivery",
	);
	assert.equal(
		openCodeEvents().some(delivery => delivery.event === "session_idle" && delivery.payload.sessionID === "attached-root"),
		false,
		"late idle from the prior root must not finish the selected root",
	);
	assert.deepEqual(
		openCodeEvents().filter(delivery => delivery.payload.sessionID === "next-root").map(delivery => delivery.event),
		["session_start", "user_prompt", "session_busy", "permission_request"],
		"attached root switch must keep lifecycle, prompt, work, and approval FIFO",
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

	console.log("PASS: OpenCode root filtering/attachment/switching and Pi duplicate/reload lifecycle preserve ordered deduplicated delivery");
} finally {
	server.stop(true);
}
