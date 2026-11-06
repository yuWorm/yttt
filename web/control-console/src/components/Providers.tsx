import { useEffect, useState } from "react";
import { Button, Divider } from "@douyinfe/semi-ui";
import { IconGithubLogo } from "@douyinfe/semi-icons";
import { get } from "../api";

export type Provider = "github" | "google";

export const PROVIDER_NAMES: Record<Provider, string> = { github: "GitHub", google: "Google" };

export function GoogleLogo({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 48 48" aria-hidden="true">
      <path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z" />
      <path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z" />
      <path fill="#FBBC05" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z" />
      <path fill="#34A853" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z" />
    </svg>
  );
}

export function ProviderIcon({ provider }: { provider: Provider }) {
  return provider === "github" ? <IconGithubLogo /> : <GoogleLogo />;
}

/** The providers the control plane is configured with; empty until known. */
export function useProviders(): Provider[] {
  const [providers, setProviders] = useState<Provider[]>([]);
  useEffect(() => {
    get<Provider[]>("/v1/auth/providers")
      .then(setProviders)
      .catch(() => setProviders([]));
  }, []);
  return providers;
}

/** Leaves for the provider; it comes back to `next` on this site. */
export function startProvider(provider: Provider, next: string) {
  window.location.assign(`/v1/auth/${provider}/start?next=${encodeURIComponent(next)}`);
}

/** Where to come back to after signing in: this page, without the error a
 * failed attempt left on it; the sign-in pages go home. */
export function here(): string {
  const { pathname } = window.location;
  if (pathname === "/signup" || pathname === "/login") {
    return "/";
  }
  const params = new URLSearchParams(window.location.search);
  params.delete("oauth_error");
  params.delete("provider");
  const query = params.toString();
  return query ? `${pathname}?${query}` : pathname;
}

export function ProviderButtons({ action }: { action: string }) {
  const providers = useProviders();
  if (providers.length === 0) {
    return null;
  }
  const next = here();
  return (
    <>
      <div style={{ display: "grid", gap: 10 }}>
        {providers.map((provider) => (
          <Button
            key={provider}
            size="large"
            block
            icon={<ProviderIcon provider={provider} />}
            onClick={() => startProvider(provider, next)}
          >
            使用 {PROVIDER_NAMES[provider]} {action}
          </Button>
        ))}
      </div>
      <Divider margin={20} align="center">
        <span style={{ color: "var(--semi-color-text-2)", fontSize: 12 }}>或使用邮箱</span>
      </Divider>
    </>
  );
}

/** What went wrong at the provider, from `?oauth_error=`. */
export function providerError(): string | undefined {
  const params = new URLSearchParams(window.location.search);
  const reason = params.get("oauth_error");
  if (!reason) {
    return undefined;
  }
  const name = PROVIDER_NAMES[params.get("provider") as Provider] ?? "第三方账号";
  switch (reason) {
    case "denied":
      return `已取消用 ${name} 登录。`;
    case "unverified_email":
      return `${name} 账号没有已验证的主邮箱，请先在 ${name} 验证邮箱。`;
    case "email_taken":
      return `该 ${name} 账号的邮箱已有账号：请用密码登录后，在“账号”页关联 ${name}。如果这个账号不是你注册的，请联系管理员。`;
    case "signup_closed":
      return "暂未开放注册，或暂无可用的 relay 节点，请联系管理员。";
    case "already_linked":
      return `这个 ${name} 账号已关联到其他账号，或本账号已关联过 ${name}。`;
    default:
      return `用 ${name} 登录失败，请稍后再试。`;
  }
}
