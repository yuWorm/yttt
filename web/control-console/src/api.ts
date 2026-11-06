// The control plane's API as the console uses it. The session lives in an
// HttpOnly cookie the page cannot read; every request also carries
// `x-yttt-web`, which the server requires before it changes anything.

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

let onSignedOut: () => void = () => {};

/** Called when the server says the session is gone. */
export function whenSignedOut(handler: () => void) {
  onSignedOut = handler;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method,
    credentials: "same-origin",
    headers: {
      "x-yttt-web": "1",
      ...(body === undefined ? {} : { "content-type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 401 && !path.startsWith("/v1/web/session")) {
    onSignedOut();
  }
  if (!response.ok) {
    let message = response.statusText;
    try {
      message = ((await response.json()) as { error?: string }).error ?? message;
    } catch {
      // Not JSON: keep the status text.
    }
    throw new ApiError(response.status, message);
  }
  if (response.status === 204) {
    return undefined as T;
  }
  return (await response.json()) as T;
}

export const get = <T>(path: string) => request<T>("GET", path);
export const post = <T>(path: string, body?: unknown) => request<T>("POST", path, body ?? {});
export const patch = <T>(path: string, body: unknown) => request<T>("PATCH", path, body);
export const del = <T>(path: string) => request<T>("DELETE", path);

export interface WebSession {
  account_id: number;
  role: "user" | "admin";
}

export interface AccountView {
  account_id: number;
  email: string;
  plan: string;
  node_id: string;
  relay_url: string;
  month_upstream_bytes: number;
  monthly_bytes: number;
  suspended: boolean;
  devices: number;
  max_devices: number;
  role: "user" | "admin";
  plan_name: string;
  plan_description: string;
  node_name: string;
  /** Bytes per second. */
  rate_bps: number;
  throttled_rate_bps: number;
}

export interface SignInMethods {
  has_password: boolean;
  /** Whether this session signed in recently enough to set a first password. */
  may_set_password: boolean;
  identities: { provider: "github" | "google"; email: string; created_unix: number }[];
  providers: ("github" | "google")[];
}

export interface DeviceEntry {
  endpoint_id: string;
  label: string;
  registered_unix: number;
}

export interface MonthUsage {
  month: number;
  upstream_bytes: number;
}

export interface DayUsage {
  /** Days since the Unix epoch, UTC. */
  day: number;
  upstream_bytes: number;
}

export interface NodeView {
  id: string;
  name: string;
  url: string;
  status: "pending" | "online" | "offline" | "disabled";
  last_report_unix: number | null;
  connected_devices: number | null;
  version: string | null;
  reachable: boolean | null;
  accounts: number;
  month_bytes: number;
  enrollment_expires_unix: number | null;
}

export interface Overview {
  accounts: number;
  suspended: number;
  admins: number;
  accounts_today: number;
  month_upstream_bytes: number;
  today_upstream_bytes: number;
  connected_devices: number;
  nodes: NodeView[];
}

export interface AccountRow {
  account_id: number;
  email: string;
  role: "user" | "admin";
  plan: string;
  node_id: string;
  suspended: boolean;
  created_unix: number;
  month_upstream_bytes: number;
  devices: number;
}

export interface AccountPage {
  total: number;
  accounts: AccountRow[];
}

export interface AccountDetail {
  account: AccountRow;
  devices: DeviceEntry[];
  usage: MonthUsage[];
  daily: DayUsage[];
}

export interface PlanView {
  id: string;
  name: string;
  description: string;
  /** Bytes per second. */
  rate_bps: number;
  burst_bytes: number;
  monthly_bytes: number;
  throttled_rate_bps: number;
  max_devices: number;
  archived: boolean;
  sort: number;
  accounts: number;
}

export interface Settings {
  signup_enabled: boolean;
  signup_plan: string | null;
  signup_node: string | null;
}

export interface AuditEntry {
  id: number;
  admin_email: string | null;
  action: string;
  target_account: number | null;
  target_node: string | null;
  detail: string;
  at_unix: number;
}

export interface DeployCommand {
  node: NodeView;
  command: string;
  expires_unix: number;
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${UNITS[unit]}`;
}

/** A rate in bytes per second, shown in megabits per second as networks are sold. */
export function formatRate(bytesPerSecond: number): string {
  const mbps = (bytesPerSecond * 8) / 1_000_000;
  return mbps >= 1 ? `${trim(mbps)} Mbps` : `${trim(mbps * 1000)} Kbps`;
}

function trim(value: number): string {
  return value >= 100 ? value.toFixed(0) : String(Number(value.toFixed(2)));
}

export const MBPS = 125_000;
export const GIB = 1024 ** 3;

/** Month-day of a day since the epoch, e.g. 11-06. */
export function formatDay(day: number): string {
  const date = new Date(day * 86_400_000);
  return `${String(date.getUTCMonth() + 1).padStart(2, "0")}-${String(date.getUTCDate()).padStart(2, "0")}`;
}

export function percentOf(used: number, quota: number): number {
  return quota > 0 ? Math.min((used / quota) * 100, 100) : 0;
}

export function errorText(failure: unknown): string {
  return failure instanceof Error ? failure.message : String(failure);
}

export function formatTime(unix: number | null | undefined): string {
  if (!unix) {
    return "—";
  }
  return new Date(unix * 1000).toLocaleString();
}

export function formatMonth(month: number): string {
  return `${Math.floor(month / 100)}-${String(month % 100).padStart(2, "0")}`;
}
