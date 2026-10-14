import { randomUUID } from "node:crypto";

import type { AgentToolContext, AgentToolResult } from "@oh-my-pi/pi-agent-core";
import type { ExtensionAPI, ExtensionContext, ToolDefinition } from "@oh-my-pi/pi-coding-agent";
import { ToolAbortError } from "@oh-my-pi/pi-coding-agent/tools/tool-errors";
import {
	TASK_SUBAGENT_EVENT_CHANNEL,
	TASK_SUBAGENT_LIFECYCLE_CHANNEL,
	TASK_SUBAGENT_PROGRESS_CHANNEL,
	type SubagentEventPayload,
	type SubagentLifecyclePayload,
	type SubagentProgressPayload,
} from "@oh-my-pi/pi-coding-agent/task";
import {
	getLatestTodoPhasesFromEntries,
	USER_TODO_EDIT_CUSTOM_TYPE,
	type TodoPhase,
} from "@oh-my-pi/pi-coding-agent/tools/todo";
import type { AskToolDetails } from "@oh-my-pi/pi-coding-agent/tools/ask";
import type { ToolSession } from "@oh-my-pi/pi-coding-agent/tools";

const PREFIX = "yttt-agent-v1:";
const MAX_TEXT = 2048;
const MAX_MESSAGE_TEXT = 16 * 1024;
const DELIVERY_PROTOCOL = 1;
const DELIVERY_STREAM_ID = randomUUID();
const MIN_RETRY_DELAY_MS = 25;
const MAX_RETRY_DELAY_MS = 1000;
const QUESTION_POLL_INTERVAL_MS = 500;
const ROOT_EXECUTION_ENTRY = "yttt.board.root-execution";
const TRANSPORT_SCOPE_OWNERS_KEY = Symbol.for("yttt-agent-status.transport-scope-owners");
const globalTransportScopeOwners = globalThis as { [key: symbol]: Set<string> | undefined };
const transportScopeOwners = (globalTransportScopeOwners[TRANSPORT_SCOPE_OWNERS_KEY] ??= new Set<string>());

type JsonRecord = Record<string, unknown>;

type AskOption = {
	label: string;
	description?: string;
	preview?: string;
};

type AskQuestion = {
	id: string;
	question: string;
	header?: string;
	options: AskOption[];
	multi: boolean;
	recommended?: number;
};

type AskParams = { questions: AskQuestion[] };

type BoardAnswer = {
	question_id: string;
	selected_options: string[];
	custom_input?: string;
};

type QuestionState = "pending" | "submitted" | "answered" | "cancelled" | "unavailable";

type QuestionPollResponse = {
	state: QuestionState;
	answers: unknown;
};

type RootExecution = {
	id: string;
	sessionId: string;
	startedAt: number;
	ended: boolean;
	task?: string;
};

type ChildExecution = {
	id: string;
	rootId: string;
	name: string;
	task?: string;
	startedAt: number;
	ended: boolean;
};

type ChildQuestion = {
	requestId: string;
	questions: AskQuestion[];
};

type HookDelivery = {
	endpoint: string;
	token: string;
	scope: string;
	sequence: number;
	event: string;
	payload: Record<string, unknown>;
	delivered: () => void;
};

const deliveryQueue: HookDelivery[] = [];
let nextDeliverySequence = 1;
let deliveryWorker: Promise<void> | undefined;

function reportingScope(): string | undefined {
	const hookEndpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
	const hookToken = process.env.YTTT_AGENT_HOOK_TOKEN;
	const hookScope = process.env.YTTT_AGENT_HOOK_SCOPE;
	if (hookEndpoint && hookToken && hookScope) {
		return JSON.stringify(["hook", hookEndpoint, hookToken, hookScope]);
	}

	const instanceId = process.env.YTTT_AGENT_INSTANCE_ID;
	const token = process.env.YTTT_AGENT_TOKEN;
	const generation = process.env.YTTT_AGENT_GENERATION;
	if (instanceId && token && generation) {
		return JSON.stringify(["osc", instanceId, token, generation]);
	}
}

function startDeliveryWorker(): void {
	if (deliveryWorker) return;
	deliveryWorker = drainDeliveryQueue().finally(() => {
		deliveryWorker = undefined;
		if (deliveryQueue.length > 0) startDeliveryWorker();
	});
}

async function drainDeliveryQueue(): Promise<void> {
	let retryDelay = MIN_RETRY_DELAY_MS;
	while (deliveryQueue.length > 0) {
		const delivery = deliveryQueue[0];
		try {
			const response = await fetch(`${delivery.endpoint}/hook/omp`, {
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
				signal: AbortSignal.timeout(2000),
			});
			if (response.ok) {
				const acknowledgement = (await response.json()) as {
					acceptedSequence?: unknown;
				};
				if (
					typeof acknowledgement.acceptedSequence === "number" &&
					Number.isSafeInteger(acknowledgement.acceptedSequence) &&
					acknowledgement.acceptedSequence >= delivery.sequence
				) {
					deliveryQueue.shift()?.delivered();
					retryDelay = MIN_RETRY_DELAY_MS;
					continue;
				}
			}
		} catch {}
		const retry = Promise.withResolvers<void>();
		setTimeout(retry.resolve, retryDelay).unref();
		await retry.promise;
		retryDelay = Math.min(retryDelay * 2, MAX_RETRY_DELAY_MS);
	}
}

function enqueueDelivery(
	endpoint: string,
	token: string,
	scope: string,
	event: string,
	payload: Record<string, unknown>,
): Promise<void> {
	const completion = Promise.withResolvers<void>();
	deliveryQueue.push({
		endpoint,
		token,
		scope,
		sequence: nextDeliverySequence++,
		event,
		payload,
		delivered: completion.resolve,
	});
	startDeliveryWorker();
	return completion.promise;
}

type StatusContext = {
	sessionManager: { getSessionId(): string; getSessionFile?(): string | undefined };
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

function deliver(name: string, payload: Record<string, unknown>, ctx: StatusContext): Promise<void> | undefined {
	const payloadWithContext = {
		...payload,
		sessionId: ctx.sessionManager.getSessionId(),
		sessionFile: ctx.sessionManager.getSessionFile?.(),
		model: ctx.model?.id,
	};
	const hookEndpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
	const hookToken = process.env.YTTT_AGENT_HOOK_TOKEN;
	const hookScope = process.env.YTTT_AGENT_HOOK_SCOPE;
	if (hookEndpoint && hookToken && hookScope) {
		return enqueueDelivery(hookEndpoint, hookToken, hookScope, name, payloadWithContext);
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

function send(name: string, payload: Record<string, unknown>, ctx: StatusContext): void {
	void deliver(name, payload, ctx);
}

async function sendAndWait(name: string, payload: Record<string, unknown>, ctx: StatusContext): Promise<boolean> {
	const delivery = deliver(name, payload, ctx);
	if (!delivery) return false;
	const timeout = Promise.withResolvers<false>();
	// Session shutdown handlers have a 2 s SDK deadline; leave time to return.
	const timer = setTimeout(() => timeout.resolve(false), 1500);
	try {
		return await Promise.race([delivery.then(() => true), timeout.promise]);
	} finally {
		clearTimeout(timer);
	}
}

function asRecord(value: unknown): JsonRecord | undefined {
	return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : undefined;
}

function asString(value: unknown): string | undefined {
	return typeof value === "string" ? value : undefined;
}

function boundedMessageText(value: string): { text: string; truncated: boolean } | undefined {
	const text = value.trim();
	if (!text) return undefined;
	return text.length <= MAX_MESSAGE_TEXT
		? { text, truncated: false }
		: { text: text.slice(0, MAX_MESSAGE_TEXT), truncated: true };
}

function messageText(message: unknown): { text: string; truncated: boolean } | undefined {
	const record = asRecord(message);
	if (!record) return undefined;
	const content = record.content;
	if (typeof content === "string") return boundedMessageText(content);
	if (!Array.isArray(content)) return undefined;
	let text = "";
	for (const block of content) {
		const blockRecord = asRecord(block);
		if (blockRecord?.type === "text" && typeof blockRecord.text === "string") {
			text += blockRecord.text;
		}
	}
	return boundedMessageText(text);
}

function messageRole(message: unknown): "user" | "assistant" | undefined {
	const role = asRecord(message)?.role;
	return role === "user" || role === "assistant" ? role : undefined;
}

function messageTimestamp(message: unknown, fallback: number): number {
	const timestamp = asRecord(message)?.timestamp;
	return typeof timestamp === "number" && Number.isSafeInteger(timestamp) && timestamp > 0 ? timestamp : fallback;
}

function timestampFromEntry(entry: JsonRecord, fallback: number): number {
	const timestamp = entry.timestamp;
	if (typeof timestamp === "string") {
		const parsed = Date.parse(timestamp);
		if (Number.isFinite(parsed) && parsed > 0) return parsed;
	}
	return messageTimestamp(entry.message, fallback);
}

function hash(value: string): string {
	let hash = 0x811c9dc5;
	for (let index = 0; index < value.length; index += 1) {
		hash ^= value.charCodeAt(index);
		hash = Math.imul(hash, 0x01000193);
	}
	return (hash >>> 0).toString(16).padStart(8, "0");
}

function statusForChild(value: unknown): "idle" | "working" | "completed" | "failed" | "interrupted" {
	switch (value) {
		case "pending":
			return "idle";
		case "running":
			return "working";
		case "completed":
			return "completed";
		case "failed":
			return "failed";
		case "aborted":
			return "interrupted";
		default:
			return "working";
	}
}

function outcomeForAgentEnd(messages: readonly unknown[]): "completed" | "failed" | "interrupted" {
	for (let index = messages.length - 1; index >= 0; index -= 1) {
		const message = asRecord(messages[index]);
		if (message?.role !== "assistant") continue;
		if (message.stopReason === "aborted") return "interrupted";
		if (typeof message.errorMessage === "string" && message.errorMessage.trim()) return "failed";
		if (message.stopReason === "error") return "failed";
		return "completed";
	}
	return "completed";
}

function readAskQuestions(value: unknown): AskParams | undefined {
	const questions = asRecord(value)?.questions;
	if (!Array.isArray(questions) || questions.length === 0) return undefined;
	const parsed: AskQuestion[] = [];
	for (const candidate of questions) {
		const record = asRecord(candidate);
		const id = asString(record?.id);
		const question = asString(record?.question);
		if (!id || !question || !Array.isArray(record?.options)) return undefined;
		const options: AskOption[] = [];
		for (const option of record.options) {
			const optionRecord = asRecord(option);
			const label = asString(optionRecord?.label);
			if (!label) return undefined;
			const description = asString(optionRecord?.description);
			const preview = asString(optionRecord?.preview);
			options.push({
				label,
				...(description ? { description } : {}),
				...(preview ? { preview } : {}),
			});
		}
		const header = asString(record.header);
		const recommended = record.recommended;
		if (recommended !== undefined && (!Number.isInteger(recommended) || recommended < 0 || recommended >= options.length)) {
			return undefined;
		}
		parsed.push({
			id,
			question,
			...(header ? { header } : {}),
			options,
			multi: record.multi === true,
			...(recommended !== undefined ? { recommended } : {}),
		});
	}
	return { questions: parsed };
}

function isBoardQuestionRepresentable(params: AskParams): boolean {
	if (params.questions.length > 16) return false;
	const ids = new Set<string>();
	let totalLength = 0;
	for (const question of params.questions) {
		if (
			!question.id.trim() ||
			question.id.length > 512 ||
			!question.question.trim() ||
			question.question.length > 8192 ||
			(question.header !== undefined && question.header.length > 256) ||
			question.options.length > 32 ||
			ids.has(question.id)
		) {
			return false;
		}
		ids.add(question.id);
		totalLength += question.id.length + question.question.length + (question.header?.length ?? 0);
		for (const option of question.options) {
			if (
				option.label.length === 0 ||
				option.label.length > 1024 ||
				(option.description !== undefined && option.description.length > 4096) ||
				(option.preview !== undefined && option.preview.length > 8192)
			) {
				return false;
			}
			totalLength += option.label.length + (option.description?.length ?? 0) + (option.preview?.length ?? 0);
		}
		if (totalLength > 32 * 1024) return false;
	}
	return true;
}

function parseAnswers(value: unknown, params: AskParams): BoardAnswer[] | undefined {
	if (!Array.isArray(value) || value.length !== params.questions.length) return undefined;
	const answersByQuestionId = new Map<string, JsonRecord>();
	for (const candidate of value) {
		const answer = asRecord(candidate);
		const questionId = asString(answer?.question_id);
		if (!answer || !questionId || answersByQuestionId.has(questionId)) return undefined;
		answersByQuestionId.set(questionId, answer);
	}
	const answers: BoardAnswer[] = [];
	for (const question of params.questions) {
		const answer = answersByQuestionId.get(question.id);
		if (!answer || !Array.isArray(answer.selected_options) || !answer.selected_options.every(option => typeof option === "string")) {
			return undefined;
		}
		const selectedOptions = answer.selected_options as string[];
		if (
			selectedOptions.length > question.options.length ||
			(!question.multi && selectedOptions.length > 1) ||
			new Set(selectedOptions).size !== selectedOptions.length ||
			selectedOptions.some(option => !question.options.some(candidate => candidate.label === option))
		) {
			return undefined;
		}
		const customInput = answer.custom_input ?? undefined;
		if (customInput !== undefined && typeof customInput !== "string") return undefined;
		if (typeof customInput === "string" && customInput.length > 8192) return undefined;
		if (selectedOptions.length === 0 && (!customInput || !customInput.trim())) return undefined;
		answers.push({
			question_id: question.id,
			selected_options: selectedOptions,
			...(typeof customInput === "string" ? { custom_input: customInput } : {}),
		});
	}
	return answers;
}

function answersFromAskResult(result: unknown, params: AskParams): { state: "answered"; answers: BoardAnswer[] } | { state: "cancelled" } | undefined {
	const details = asRecord(asRecord(result)?.details);
	if (!details) return undefined;
	if (details.chatRedirect === true) return { state: "cancelled" };
	const rawResults = Array.isArray(details.results)
		? details.results
		: params.questions.length === 1
			? [
				{
					id: params.questions[0].id,
					selectedOptions: details.selectedOptions,
					customInput: details.customInput,
				},
			]
			: undefined;
	if (!rawResults || rawResults.length !== params.questions.length) return undefined;
	const answers = parseAnswers(
		rawResults.map(result => {
			const record = asRecord(result);
			return {
				question_id: record?.id,
				selected_options: record?.selectedOptions,
				custom_input: record?.customInput,
			};
		}),
		params,
	);
	return answers ? { state: "answered", answers } : { state: "cancelled" };
}

function askResultFromAnswers(params: AskParams, answers: BoardAnswer[]): AgentToolResult<AskToolDetails> {
	const byQuestion = new Map(answers.map(answer => [answer.question_id, answer]));
	const results = params.questions.map(question => {
		const answer = byQuestion.get(question.id)!;
		return {
			id: question.id,
			question: question.question,
			options: question.options.map(option => option.label),
			multi: question.multi,
			selectedOptions: answer.selected_options,
			...(answer.custom_input !== undefined ? { customInput: answer.custom_input } : {}),
		};
	});
	const rendered = results
		.map(result => {
			const selections = result.selectedOptions.length > 0 ? result.selectedOptions.join(", ") : undefined;
			return [selections, result.customInput].filter((part): part is string => Boolean(part)).join("\n");
		})
		.join("\n");
	if (results.length === 1) {
		const result = results[0];
		return {
			content: [{ type: "text", text: rendered }],
			details: {
				question: result.question,
				options: result.options,
				multi: result.multi,
				selectedOptions: result.selectedOptions,
				...(result.customInput !== undefined ? { customInput: result.customInput } : {}),
			},
		};
	}
	return {
		content: [{ type: "text", text: `User answers:\n${rendered}` }],
		details: { results },
	};
}

function isCancellation(error: unknown): boolean {
	return error instanceof Error && (error.name === "ToolAbortError" || error.name === "AbortError");
}

function hookTransport(): { endpoint: string; token: string; scope: string } | undefined {
	const endpoint = process.env.YTTT_AGENT_HOOK_ENDPOINT;
	const token = process.env.YTTT_AGENT_HOOK_TOKEN;
	const scope = process.env.YTTT_AGENT_HOOK_SCOPE;
	return endpoint && token && scope ? { endpoint, token, scope } : undefined;
}

async function waitForPoll(signal: AbortSignal): Promise<boolean> {
	if (signal.aborted) return false;
	const delay = Promise.withResolvers<void>();
	const timer = setTimeout(delay.resolve, QUESTION_POLL_INTERVAL_MS);
	const onAbort = () => delay.resolve();
	signal.addEventListener("abort", onAbort, { once: true });
	try {
		await delay.promise;
		return !signal.aborted;
	} finally {
		clearTimeout(timer);
		signal.removeEventListener("abort", onAbort);
	}
}

function readPollResponse(value: unknown): QuestionPollResponse | undefined {
	const record = asRecord(value);
	const state = record?.state;
	if (
		state !== "pending" &&
		state !== "submitted" &&
		state !== "answered" &&
		state !== "cancelled" &&
		state !== "unavailable"
	) {
		return undefined;
	}
	return { state, answers: record.answers ?? [] };
}

async function pollQuestion(
	transport: { endpoint: string; token: string; scope: string },
	sessionId: string,
	requestId: string,
	signal: AbortSignal,
): Promise<QuestionPollResponse | undefined> {
	while (!signal.aborted) {
		try {
			const requestSignal = AbortSignal.any([signal, AbortSignal.timeout(2000)]);
			const response = await fetch(`${transport.endpoint}/hook/omp`, {
				method: "POST",
				headers: {
					"content-type": "application/json",
					"x-yttt-agent-hook-token": transport.token,
					"x-yttt-agent-hook-scope": transport.scope,
				},
				body: JSON.stringify({
					event: "question_poll",
					payload: { sessionId, requestId },
				}),
				signal: requestSignal,
			});
			if (!response.ok) return undefined;
			const answer = readPollResponse(await response.json());
			if (!answer || answer.state === "unavailable") return undefined;
			if (answer.state !== "pending") return answer;
			if (!(await waitForPoll(signal))) return undefined;
		} catch {
			return undefined;
		}
	}
}

function taskStatus(status: unknown): "pending" | "in_progress" | "completed" | "blocked" | "abandoned" | "unknown" {
	switch (status) {
		case "pending":
		case "in_progress":
		case "completed":
		case "blocked":
		case "abandoned":
			return status;
		default:
			return "unknown";
	}
}

function isTodoPhase(value: unknown): value is TodoPhase {
	const phase = asRecord(value);
	return typeof phase?.name === "string" && Array.isArray(phase.tasks);
}

function planItems(executionId: string, phases: readonly TodoPhase[]): JsonRecord[] {
	const occurrences = new Map<string, number>();
	const items: JsonRecord[] = [];
	for (const phase of phases) {
		if (!isTodoPhase(phase)) continue;
		for (const task of phase.tasks) {
			const taskRecord = asRecord(task);
			if (!taskRecord || typeof taskRecord.content !== "string") continue;
			const fingerprint = `${phase.name}\u0000${taskRecord.content}`;
			const occurrence = occurrences.get(fingerprint) ?? 0;
			occurrences.set(fingerprint, occurrence + 1);
			items.push({
				id: `todo:${executionId}:${hash(fingerprint)}:${occurrence}`,
				title: taskRecord.content,
				phase: phase.name,
				status: taskStatus(taskRecord.status),
				...(typeof taskRecord.blocker === "string" && taskRecord.blocker.trim()
					? { blocker: taskRecord.blocker }
					: {}),
			});
		}
	}
	return items;
}

function latestTodoEntry(entries: readonly unknown[]): JsonRecord | undefined {
	for (let index = entries.length - 1; index >= 0; index -= 1) {
		const entry = asRecord(entries[index]);
		if (!entry) continue;
		if (entry.type === "custom" && entry.customType === USER_TODO_EDIT_CUSTOM_TYPE) {
			if (Array.isArray(asRecord(entry.data)?.phases)) return entry;
			continue;
		}
		const message = asRecord(entry.message);
		if (
			entry.type === "message" &&
			message?.role === "toolResult" &&
			message.toolName === "todo" &&
			message.isError !== true &&
			Array.isArray(asRecord(message.details)?.phases)
		) {
			return entry;
		}
	}
}

export default function (pi: ExtensionAPI) {
	// A subagent gets a separately initialized extension API, but shares the root
	// transport environment. Only the root owns that transport; raw child events
	// arrive through the root EventBus and retain an explicit child execution ID.
	const scope = reportingScope();
	if (scope && transportScopeOwners.has(scope)) return;
	if (scope) transportScopeOwners.add(scope);

	let statusContext: ExtensionContext | undefined;
	let observedSessionId: string | undefined;
	let currentRoot: RootExecution | undefined;
	let historyExecutionId: string | undefined;
	let historyExecutionSent = false;
	const childExecutions = new Map<string, ChildExecution>();
	const childQuestions = new Map<string, ChildQuestion>();
	const sentMessageIds = new Set<string>();
	const sentChildMessageIds = new Set<string>();

	const resetSession = (ctx: ExtensionContext) => {
		const sessionId = ctx.sessionManager.getSessionId();
		if (observedSessionId === sessionId) return;
		observedSessionId = sessionId;
		currentRoot = undefined;
		historyExecutionId = `history:${sessionId}`;
		historyExecutionSent = false;
		childExecutions.clear();
		childQuestions.clear();
		sentMessageIds.clear();
		sentChildMessageIds.clear();
	};

	const board = (ctx: StatusContext, event: JsonRecord) => send("board_event", { board: event }, ctx);

	const ensureRoot = (ctx: ExtensionContext, task?: string): RootExecution => {
		resetSession(ctx);
		if (currentRoot && !currentRoot.ended) return currentRoot;
		const now = Date.now();
		const root: RootExecution = {
			id: `root:${ctx.sessionManager.getSessionId()}:${randomUUID()}`,
			sessionId: ctx.sessionManager.getSessionId(),
			startedAt: now,
			ended: false,
		};
		currentRoot = root;
		root.task = task;
		pi.appendEntry(ROOT_EXECUTION_ENTRY, {
			id: root.id, startedAt: root.startedAt, task, state: "unknown", endedAt: null,
		});
		board(ctx, {
			kind: "execution",
			execution: {
				id: root.id,
				parent_id: null,
				name: "Oh My Pi",
				...(task ? { task } : {}),
				state: "working",
				started_at: now,
				updated_at: now,
				ended_at: null,
			},
		});
		return root;
	};

	const ensureHistoryExecution = (ctx: ExtensionContext, startedAt: number): string => {
		resetSession(ctx);
		const id = historyExecutionId!;
		if (!historyExecutionSent) {
			historyExecutionSent = true;
			const now = Date.now();
			board(ctx, {
				kind: "execution",
				execution: {
					id,
					parent_id: null,
					name: "Session history",
					state: "unknown",
					started_at: startedAt,
					updated_at: now,
					ended_at: null,
				},
			});
		}
		return id;
	};

	const emitMessage = (
		ctx: ExtensionContext,
		message: unknown,
		executionId: string,
		id: string,
		seen: Set<string>,
		timestampFallback: number,
	) => {
		if (seen.has(id)) return;
		const role = messageRole(message);
		const text = messageText(message);
		if (!role || !text) return;
		seen.add(id);
		board(ctx, {
			kind: "message",
			message: {
				id,
				execution_id: executionId,
				role,
				text: text.text,
				timestamp: messageTimestamp(message, timestampFallback),
				truncated: text.truncated,
			},
		});
	};

	const messageId = (ctx: ExtensionContext, message: unknown, timestamp: number): string =>
		`message:${ctx.sessionManager.getSessionId()}:${messageRole(message) ?? "unknown"}:${timestamp}:${hash(messageText(message)?.text ?? "")}`;

	const backfill = (ctx: ExtensionContext) => {
		resetSession(ctx);
		const branch = ctx.sessionManager.getBranch();
		const todoEntry = latestTodoEntry(branch);
		const executions = new Map<string, JsonRecord>();
		const messages: { entry: JsonRecord; executionId: string }[] = [];
		let activeExecutionId: string | undefined;
		let todoExecutionId: string | undefined;
		for (const candidate of branch) {
			const entry = asRecord(candidate);
			if (!entry) continue;
			if (entry.type === "custom" && entry.customType === ROOT_EXECUTION_ENTRY) {
				const data = asRecord(entry.data);
				if (
					data && typeof data.id === "string" &&
					typeof data.startedAt === "number" && Number.isSafeInteger(data.startedAt) &&
					data.startedAt > 0
				) {
					activeExecutionId = data.id;
					const endedAt = typeof data.endedAt === "number" && Number.isSafeInteger(data.endedAt) &&
						data.endedAt >= data.startedAt ? data.endedAt : null;
					const state = endedAt !== null &&
						(data.state === "completed" || data.state === "failed" || data.state === "interrupted")
						? data.state : "unknown";
					executions.set(data.id, {
						id: data.id, parent_id: null, name: "Oh My Pi",
						...(typeof data.task === "string" ? { task: data.task } : {}),
						state, started_at: data.startedAt,
						updated_at: endedAt ?? data.startedAt, ended_at: endedAt,
					});
				}
				continue;
			}
			if (entry === todoEntry) {
				todoExecutionId = activeExecutionId ?? ensureHistoryExecution(ctx, timestampFromEntry(entry, Date.now()));
			}
			if (entry.type !== "message" || !messageRole(entry.message)) continue;
			messages.push({
				entry,
				executionId: activeExecutionId ?? ensureHistoryExecution(ctx, timestampFromEntry(entry, Date.now())),
			});
		}
		for (const execution of [...executions.values()].slice(-256)) {
			board(ctx, { kind: "execution", execution });
		}
		for (const { entry, executionId } of messages.slice(-256)) {
			const timestamp = messageTimestamp(entry.message, timestampFromEntry(entry, Date.now()));
			emitMessage(ctx, entry.message, executionId, messageId(ctx, entry.message, timestamp), sentMessageIds, timestamp);
		}
		if (!todoExecutionId) return;
		board(ctx, {
			kind: "plan",
			execution_id: todoExecutionId,
			items: planItems(todoExecutionId, getLatestTodoPhasesFromEntries(branch)),
		});
	};

	const ensureChild = (
		ctx: ExtensionContext,
		childId: string,
		name?: string,
		task?: string,
	): ChildExecution | undefined => {
		resetSession(ctx);
		const existing = childExecutions.get(childId);
		if (existing) return existing;
		const root = currentRoot;
		if (!root || root.ended || root.sessionId !== ctx.sessionManager.getSessionId()) return undefined;
		const now = Date.now();
		const child: ChildExecution = {
			id: `child:${root.id}:${childId}`,
			rootId: root.id,
			name: name || "Subagent",
			...(task ? { task } : {}),
			startedAt: now,
			ended: false,
		};
		childExecutions.set(childId, child);
		board(ctx, {
			kind: "execution",
			execution: {
				id: child.id,
				parent_id: root.id,
				name: child.name,
				...(child.task ? { task: child.task } : {}),
				state: "working",
				started_at: now,
				updated_at: now,
				ended_at: null,
			},
		});
		return child;
	};

	const updateChild = (
		ctx: ExtensionContext,
		child: ChildExecution,
		state: "idle" | "working" | "completed" | "failed" | "interrupted",
		ended: boolean,
	) => {
		const now = Date.now();
		child.ended = ended;
		board(ctx, {
			kind: "execution",
			execution: {
				id: child.id,
				parent_id: child.rootId,
				name: child.name,
				...(child.task ? { task: child.task } : {}),
				state,
				started_at: child.startedAt,
				updated_at: now,
				ended_at: ended ? now : null,
			},
		});
	};

	const resolveQuestion = (
		ctx: StatusContext,
		requestId: string,
		state: "answered" | "cancelled",
		answers: BoardAnswer[],
		source: "board" | "native",
	) => {
		board(ctx, {
			kind: "question_resolved",
			request_id: requestId,
			state,
			answers,
			source,
		});
	};

	const invokeNativeAsk = (
		toolCallId: string,
		params: AskParams,
		signal: AbortSignal,
		onUpdate: Parameters<NonNullable<ToolDefinition["execute"]>>[3],
		ctx: ExtensionContext,
	): Promise<AgentToolResult<AskToolDetails>> => {
		if (ctx.invokeTool) {
			return ctx.invokeTool<AskToolDetails>(params, { signal, onUpdate });
		}
		const toolSession: ToolSession = {
			cwd: ctx.cwd,
			hasUI: ctx.hasUI,
			getSessionFile: () => ctx.sessionManager.getSessionFile() ?? null,
			getSessionSpawns: () => null,
			settings: pi.pi.settings,
		};
		const toolContext: AgentToolContext = {
			sessionManager: ctx.sessionManager,
			modelRegistry: ctx.modelRegistry,
			model: ctx.model,
			isIdle: () => ctx.isIdle(),
			hasQueuedMessages: () => ctx.hasPendingMessages(),
			abort: () => ctx.abort(),
			settings: pi.pi.settings,
			ui: ctx.ui,
			hasUI: ctx.hasUI,
		};
		return new pi.pi.AskTool(toolSession).execute(toolCallId, params, signal, onUpdate, toolContext);
	};

	const templateAsk = new pi.pi.AskTool({
		cwd: process.cwd(),
		hasUI: true,
		getSessionFile: () => null,
		getSessionSpawns: () => null,
		settings: pi.pi.settings,
	});
	const askTool: ToolDefinition = {
		name: "ask",
		label: templateAsk.label,
		description: templateAsk.description,
		parameters: templateAsk.parameters,
		approval: "read",
		strict: true,
		async execute(toolCallId, rawParams, outerSignal, onUpdate, ctx) {
			const params = readAskQuestions(rawParams);
			if (!params) {
				return {
					content: [{ type: "text", text: "Error: invalid Ask question payload" }],
					isError: true,
					details: {},
				};
			}

			resetSession(ctx);
			const root = ensureRoot(ctx);
			const transport = hookTransport();
			const shouldTrackBoardQuestion = transport !== undefined && isBoardQuestionRepresentable(params);
			const requestId = `ask:${root.sessionId}:${toolCallId}`;
			const questionDelivery = shouldTrackBoardQuestion
				? sendAndWait(
						"board_event",
						{
							board: {
								kind: "question_opened",
								request: {
									id: requestId,
									execution_id: root.id,
									tool_call_id: toolCallId,
									questions: params.questions,
									answerable: true,
								},
							},
						},
						ctx,
					)
				: Promise.resolve(false);
			const nativeAbort = new AbortController();
			const pollAbort = new AbortController();
			const nativeSignal = outerSignal ? AbortSignal.any([outerSignal, nativeAbort.signal]) : nativeAbort.signal;
			const settlement = Promise.withResolvers<
				| { kind: "result"; result: AgentToolResult<AskToolDetails> }
				| { kind: "cancel" }
				| { kind: "error"; error: unknown }
			>();
			let settled = false;
			const settle = (
				value:
					| { kind: "result"; result: AgentToolResult<AskToolDetails> }
					| { kind: "cancel" }
					| { kind: "error"; error: unknown },
				cancelLoser: () => void,
			) => {
				if (settled) return false;
				settled = true;
				try {
					cancelLoser();
				} catch {}
				settlement.resolve(value);
				return true;
			};
			const onOuterAbort = () => {
				if (settled) return;
				if (!shouldTrackBoardQuestion) {
					settle({ kind: "cancel" }, () => {
						pollAbort.abort();
						nativeAbort.abort();
					});
					return;
				}
				resolveQuestion(ctx, requestId, "cancelled", [], "native");
				settle({ kind: "cancel" }, () => {
					pollAbort.abort();
					nativeAbort.abort();
				});
			};
			outerSignal?.addEventListener("abort", onOuterAbort, { once: true });

			void invokeNativeAsk(toolCallId, params, nativeSignal, onUpdate, ctx).then(
				result => {
					if (settled) return;
					const resolution = answersFromAskResult(result, params) ?? { state: "cancelled" as const };
					if (shouldTrackBoardQuestion) {
						resolveQuestion(
							ctx,
							requestId,
							resolution.state,
							resolution.state === "answered" ? resolution.answers : [],
							"native",
						);
					}
					settle({ kind: "result", result }, () => pollAbort.abort());
				},
				error => {
					if (settled) return;
					if (isCancellation(error) || outerSignal?.aborted) {
						if (shouldTrackBoardQuestion) resolveQuestion(ctx, requestId, "cancelled", [], "native");
						settle({ kind: "cancel" }, () => pollAbort.abort());
						return;
					}
					settle({ kind: "error", error }, () => pollAbort.abort());
				},
			);

			if (transport) {
				void questionDelivery.then(async delivered => {
					if (!delivered || settled) return;
					const response = await pollQuestion(transport, root.sessionId, requestId, pollAbort.signal);
					if (!response || settled) return;
					if (response.state === "submitted" || response.state === "answered") {
						const answers = parseAnswers(response.answers, params);
						if (!answers) return;
						resolveQuestion(ctx, requestId, "answered", answers, "board");
						settle({ kind: "result", result: askResultFromAnswers(params, answers) }, () => {
							nativeAbort.abort();
							pollAbort.abort();
						});
						return;
					}
					if (response.state === "cancelled") {
						resolveQuestion(ctx, requestId, "cancelled", [], "board");
						settle({ kind: "cancel" }, () => {
							nativeAbort.abort();
							pollAbort.abort();
						});
					}
				});
			}

			try {
				const winner = await settlement.promise;
				if (winner.kind === "result") return winner.result;
				if (winner.kind === "error") throw winner.error;
				ctx.abort();
				throw new ToolAbortError("Ask tool was cancelled by the user");
			} finally {
				outerSignal?.removeEventListener("abort", onOuterAbort);
				pollAbort.abort();
			}
		},
	};
	pi.registerTool(askTool);

	const trackContext = (ctx: ExtensionContext, event: "session_start" | "session_switch") => {
		resetSession(ctx);
		statusContext = ctx;
		send(event, {}, ctx);
		backfill(ctx);
	};

	pi.on("session_start", (_event, ctx) => trackContext(ctx, "session_start"));
	pi.on("session_switch", (_event, ctx) => trackContext(ctx, "session_switch"));
	pi.on("session_shutdown", async (_event, ctx) => {
		await sendAndWait("session_shutdown", {}, ctx);
		if (scope) transportScopeOwners.delete(scope);
	});
	pi.on("before_agent_start", (event, ctx) => {
		const prompt = clip(event.prompt);
		ensureRoot(ctx, prompt);
		send("before_agent_start", { prompt }, ctx);
	});
	pi.on("agent_start", (_event, ctx) => {
		ensureRoot(ctx);
		send("agent_start", {}, ctx);
	});
	pi.on("agent_end", (event, ctx) => {
		resetSession(ctx);
		const root = currentRoot;
		const outcome = outcomeForAgentEnd(event.messages);
		if (root && !root.ended && event.willContinue !== true) {
			const now = Date.now();
			root.ended = true;
			pi.appendEntry(ROOT_EXECUTION_ENTRY, {
				id: root.id, startedAt: root.startedAt, task: root.task, state: outcome, endedAt: now,
			});
			board(ctx, {
				kind: "execution",
				execution: {
					id: root.id,
					parent_id: null,
					name: "Oh My Pi",
					state: outcome,
					started_at: root.startedAt,
					updated_at: now,
					ended_at: now,
				},
			});
		}
		send("agent_end", { willContinue: event.willContinue === true, outcome }, ctx);
	});
	pi.on("message_end", (event, ctx) => {
		resetSession(ctx);
		const fallback = messageTimestamp(event.message, Date.now());
		const id = messageId(ctx, event.message, fallback);
		const role = messageRole(event.message);
		const root = currentRoot;
		const executionId = role === "user" && (!root || root.ended || root.sessionId !== ctx.sessionManager.getSessionId())
			? ensureRoot(ctx, clip(messageText(event.message)?.text)).id
			: root?.sessionId === ctx.sessionManager.getSessionId()
				? root.id
				: ensureHistoryExecution(ctx, fallback);
		emitMessage(ctx, event.message, executionId, id, sentMessageIds, fallback);
	});
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
	pi.on("tool_result", (event, ctx) => {
		if (event.toolName !== "todo" || event.isError) return;
		const details = asRecord(event.details);
		if (!Array.isArray(details?.phases) || !details.phases.every(isTodoPhase)) return;
		resetSession(ctx);
		const root = currentRoot;
		if (!root || root.ended || root.sessionId !== ctx.sessionManager.getSessionId()) return;
		board(ctx, {
			kind: "plan",
			execution_id: root.id,
			items: planItems(root.id, details.phases),
		});
	});

	pi.events.on(TASK_SUBAGENT_LIFECYCLE_CHANNEL, data => {
		const event = data as SubagentLifecyclePayload;
		const ctx = statusContext;
		if (!ctx) return;
		if (event.status === "started") {
			const child = ensureChild(ctx, event.id, clip(event.id), clip(event.description));
			if (!child) return;
			send(
				"child_started",
				{
					childId: event.id,
					name: clip(event.id),
					task: clip(event.description),
				},
				ctx,
			);
			return;
		}
		const child = childExecutions.get(event.id);
		if (!child || child.ended) return;
		updateChild(ctx, child, statusForChild(event.status), true);
		const childQuestionPrefix = `${event.id}:`;
		for (const [key, question] of childQuestions) {
			if (!key.startsWith(childQuestionPrefix)) continue;
			childQuestions.delete(key);
			resolveQuestion(ctx, question.requestId, "cancelled", [], "native");
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
		const child = childExecutions.get(event.progress.id);
		if (!child || child.ended) return;
		if (typeof event.task === "string" && event.task.trim()) child.task = clip(event.task);
		updateChild(ctx, child, statusForChild(event.progress.status), false);
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
	pi.events.on(TASK_SUBAGENT_EVENT_CHANNEL, data => {
		const payload = data as SubagentEventPayload;
		const ctx = statusContext;
		if (!ctx) return;
		const child = childExecutions.get(payload.id);
		if (!child || child.ended) return;
		const event = payload.event as unknown as JsonRecord;
		if (event.type === "message_end") {
			const message = event.message;
			const timestamp = messageTimestamp(message, Date.now());
			const id = `child-message:${child.id}:${messageRole(message) ?? "unknown"}:${timestamp}:${hash(messageText(message)?.text ?? "")}`;
			emitMessage(ctx, message, child.id, id, sentChildMessageIds, timestamp);
			return;
		}
		if (event.type === "tool_execution_start" && event.toolName === "ask") {
			const params = readAskQuestions(event.args);
			const toolCallId = asString(event.toolCallId);
			if (!params || !toolCallId || !isBoardQuestionRepresentable(params)) return;
			const requestId = `ask:${child.id}:${toolCallId}`;
			childQuestions.set(`${payload.id}:${toolCallId}`, { requestId, questions: params.questions });
			board(ctx, {
				kind: "question_opened",
				request: {
					id: requestId,
					execution_id: child.id,
					tool_call_id: toolCallId,
					questions: params.questions,
					answerable: false,
				},
			});
			return;
		}
		if (event.type === "tool_execution_end" && event.toolName === "ask") {
			const toolCallId = asString(event.toolCallId);
			if (!toolCallId) return;
			const key = `${payload.id}:${toolCallId}`;
			const question = childQuestions.get(key);
			if (!question) return;
			const resolution = event.isError === true
				? { state: "cancelled" as const }
				: answersFromAskResult(event.result, { questions: question.questions }) ?? { state: "cancelled" as const };
			childQuestions.delete(key);
			resolveQuestion(
				ctx,
				question.requestId,
				resolution.state,
				resolution.state === "answered" ? resolution.answers : [],
				"native",
			);
		}
	});
}
