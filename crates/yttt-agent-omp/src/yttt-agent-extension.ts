import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import {
	TASK_SUBAGENT_LIFECYCLE_CHANNEL,
	TASK_SUBAGENT_PROGRESS_CHANNEL,
	type SubagentLifecyclePayload,
	type SubagentProgressPayload,
} from "@oh-my-pi/pi-coding-agent/task";

const PREFIX = "yttt-agent-v1:";
const MAX_TEXT = 2048;

type StatusContext = {
	sessionManager: { getSessionId(): string };
	model?: { id: string };
};

function clip(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const normalized = value.trim().replace(/\s+/g, " ");
	if (!normalized) return undefined;
	return normalized.length <= MAX_TEXT ? normalized : `${normalized.slice(0, MAX_TEXT - 1)}…`;
}

function toolDetail(input: unknown): string | undefined {
	if (!input || typeof input !== "object") return clip(input);
	const record = input as Record<string, unknown>;
	for (const key of ["path", "command", "pattern", "query", "task", "prompt", "description", "url", "i"]) {
		const value = clip(record[key]);
		if (value) return value;
	}
	return undefined;
}

function send(name: string, payload: Record<string, unknown>, ctx: StatusContext): void {
	const payloadWithContext = {
		...payload,
		sessionId: ctx.sessionManager.getSessionId(),
		model: ctx.model?.id,
	};
	const hookEndpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
	const hookToken = process.env.YTTT_AGENT_HOOK_TOKEN;
	const hookScope = process.env.YTTT_AGENT_HOOK_SCOPE;
	if (hookEndpoint && hookToken && hookScope) {
		void fetch(`${hookEndpoint}/hook/omp`, {
			method: "POST",
			headers: {
				"content-type": "application/json",
				"x-yttt-agent-hook-token": hookToken,
				"x-yttt-agent-hook-scope": hookScope,
			},
			body: JSON.stringify({ event: name, payload: payloadWithContext }),
			signal: AbortSignal.timeout(2000),
		}).catch(() => {});
		return;
	}

	const instanceId = process.env.YTTT_AGENT_INSTANCE_ID;
	const token = process.env.YTTT_AGENT_TOKEN;
	const generation = Number(process.env.YTTT_AGENT_GENERATION ?? "0");
	if (!instanceId || !token || !Number.isSafeInteger(generation) || generation <= 0) return;
	const frame = {
		protocol: 1,
		instanceId,
		token,
		generation,
		event: name,
		payload: payloadWithContext,
	};
	const encoded = Buffer.from(JSON.stringify(frame), "utf8").toString("base64url");
	process.stdout.write(`\u001b]2;${PREFIX}${encoded}\u0007`);
}

export default function (pi: ExtensionAPI) {
	let statusContext: StatusContext | undefined;
	const trackContext = (ctx: StatusContext) => {
		statusContext = ctx;
		send("session_start", {}, ctx);
	};

	pi.on("session_start", (_event, ctx) => trackContext(ctx));
	pi.on("session_switch", (_event, ctx) => trackContext(ctx));
	pi.on("before_agent_start", (event, ctx) =>
		send("before_agent_start", { prompt: clip(event.prompt) }, ctx),
	);
	pi.on("agent_start", (_event, ctx) => send("agent_start", {}, ctx));
	pi.on("agent_end", (event, ctx) =>
		send("agent_end", { willContinue: event.willContinue === true }, ctx),
	);
	pi.on("tool_call", (event, ctx) =>
		send(
			"tool_call",
			{
				toolCallId: event.toolCallId,
				toolName: event.toolName,
				detail: toolDetail(event.input),
			},
			ctx,
		),
	);
	pi.on("tool_approval_requested", (event, ctx) =>
		send(
			"tool_approval_requested",
			{
				toolCallId: event.toolCallId,
				toolName: event.toolName,
				reason: clip(event.reason),
			},
			ctx,
		),
	);
	pi.on("tool_approval_resolved", (event, ctx) =>
		send(
			"tool_approval_resolved",
			{
				toolCallId: event.toolCallId,
				toolName: event.toolName,
				approved: event.approved,
				reason: clip(event.reason),
			},
			ctx,
		),
	);
	pi.on("tool_execution_start", (event, ctx) =>
		send(
			"tool_execution_start",
			{
				toolCallId: event.toolCallId,
				toolName: event.toolName,
				detail: clip(event.intent) ?? toolDetail(event.args),
			},
			ctx,
		),
	);
	pi.on("tool_execution_end", (event, ctx) =>
		send(
			"tool_execution_end",
			{
				toolCallId: event.toolCallId,
				toolName: event.toolName,
				isError: event.isError,
			},
			ctx,
		),
	);
	pi.events.on(TASK_SUBAGENT_LIFECYCLE_CHANNEL, data => {
		const event = data as SubagentLifecyclePayload;
		const ctx = statusContext;
		if (!ctx) return;
		if (event.status === "started") {
			send(
				"child_started",
				{
					childId: event.id,
					name: clip(event.agent),
					task: clip(event.description),
				},
				ctx,
			);
			return;
		}
		send(
			"child_finished",
			{
				childId: event.id,
				outcome: event.status,
			},
			ctx,
		);
	});
	pi.events.on(TASK_SUBAGENT_PROGRESS_CHANNEL, data => {
		const event = data as SubagentProgressPayload;
		const ctx = statusContext;
		if (!ctx) return;
		send(
			"child_updated",
			{
				childId: event.progress.id,
				task: clip(event.task),
				action: clip(event.progress.currentTool),
				status: event.progress.status,
			},
			ctx,
		);
	});
}
