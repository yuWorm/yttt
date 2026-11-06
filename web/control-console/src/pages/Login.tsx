import { useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { Banner, Button, Form, Typography } from "@douyinfe/semi-ui";
import { IconBolt, IconHistogram, IconLock } from "@douyinfe/semi-icons";
import { ApiError, type WebSession, errorText, post } from "../api";
import { useSession } from "../session";
import { BrandMark } from "../components/Brand";
import { ProviderButtons, providerError } from "../components/Providers";

export function Login() {
  const { refresh } = useSession();
  const [error, setError] = useState<string | undefined>(providerError);
  const [busy, setBusy] = useState(false);
  const submit = async (values: { email: string; password: string }) => {
    setBusy(true);
    setError(undefined);
    try {
      await post<WebSession>("/v1/web/session", values);
      await refresh();
    } catch (failure) {
      setError(
        failure instanceof ApiError && failure.status === 401
          ? "邮箱或密码不正确"
          : failure instanceof ApiError && failure.status === 429
            ? "尝试次数过多，请稍后再试"
            : errorText(failure),
      );
    } finally {
      setBusy(false);
    }
  };
  return (
    <AuthLayout title="欢迎回来" subtitle="登录你的 yttt relay 账号">
      {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
      <ProviderButtons action="登录" />
      <Form onSubmit={submit} labelPosition="top">
        <Form.Input
          field="email"
          label="邮箱"
          size="large"
          placeholder="you@example.com"
          autoComplete="email"
          rules={[{ required: true, message: "请输入邮箱" }]}
        />
        <Form.Input
          field="password"
          label="密码"
          mode="password"
          size="large"
          autoComplete="current-password"
          rules={[{ required: true, message: "请输入密码" }]}
        />
        <Button htmlType="submit" theme="solid" size="large" block loading={busy} style={{ marginTop: 8 }}>
          登录
        </Button>
      </Form>
      <Typography.Text type="tertiary" style={{ display: "block", marginTop: 20, textAlign: "center" }}>
        还没有账号？<Link to="/signup">免费注册</Link>
      </Typography.Text>
      <Typography.Text type="quaternary" size="small" style={{ display: "block", marginTop: 8, textAlign: "center" }}>
        忘记密码请联系管理员重置
      </Typography.Text>
    </AuthLayout>
  );
}

const FEATURES = [
  { icon: <IconLock />, title: "端到端加密", text: "relay 只转发密文，看不到内容，也决定不了谁能连你的电脑。" },
  { icon: <IconBolt />, title: "直连优先", text: "设备之间先尝试直连，打不通时才经 relay 中转。" },
  { icon: <IconHistogram />, title: "按量计费", text: "只有经 relay 转发的流量计入用量，直连不计。" },
];

export function AuthLayout({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
}) {
  return (
    <div className="auth">
      <div className="auth-brand">
        <div className="brand" style={{ color: "#fff" }}>
          <BrandMark />
          yttt relay
        </div>
        <div style={{ maxWidth: 440 }}>
          <h1>在任何网络下，连回你的电脑</h1>
          <div style={{ opacity: 0.8, fontSize: 15 }}>为 yttt 的 Host 与设备提供可靠的中转通道。</div>
          {FEATURES.map((feature) => (
            <div className="auth-feature" key={feature.title}>
              <span className="auth-feature-icon">{feature.icon}</span>
              <div>
                <div style={{ fontWeight: 600 }}>{feature.title}</div>
                <div style={{ opacity: 0.75, fontSize: 13, marginTop: 2 }}>{feature.text}</div>
              </div>
            </div>
          ))}
        </div>
        <div style={{ opacity: 0.55, fontSize: 12 }}>yttt · 开源终端工作台</div>
      </div>
      <div className="auth-form">
        <div className="auth-card">
          <Typography.Title heading={2} style={{ marginBottom: 6 }}>
            {title}
          </Typography.Title>
          <Typography.Text type="tertiary" style={{ display: "block", marginBottom: 28 }}>
            {subtitle}
          </Typography.Text>
          {children}
        </div>
      </div>
    </div>
  );
}
