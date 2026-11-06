import { useState } from "react";
import { Link } from "react-router-dom";
import { Banner, Button, Card, Form, Typography } from "@douyinfe/semi-ui";
import { ApiError, type WebSession, post } from "../api";
import { useSession } from "../session";

export function Login() {
  const { refresh } = useSession();
  const [error, setError] = useState<string>();
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
            : String(failure instanceof Error ? failure.message : failure),
      );
    } finally {
      setBusy(false);
    }
  };
  return (
    <Centered title="登录 yttt relay">
      {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
      <Form onSubmit={submit} labelPosition="top">
        <Form.Input field="email" label="邮箱" rules={[{ required: true, message: "请输入邮箱" }]} />
        <Form.Input field="password" label="密码" mode="password" rules={[{ required: true, message: "请输入密码" }]} />
        <Button htmlType="submit" theme="solid" block loading={busy}>
          登录
        </Button>
      </Form>
      <Typography.Text style={{ display: "block", marginTop: 16 }}>
        还没有账号？<Link to="/signup">注册</Link>
      </Typography.Text>
    </Centered>
  );
}

export function Centered({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ display: "grid", placeItems: "center", minHeight: "100vh", padding: 16 }}>
      <Card style={{ width: "100%", maxWidth: 400 }} title={title}>
        {children}
      </Card>
    </div>
  );
}
