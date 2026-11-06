import { useState } from "react";
import { Link } from "react-router-dom";
import { Banner, Button, Form, Typography } from "@douyinfe/semi-ui";
import { ApiError, type WebSession, errorText, post } from "../api";
import { useSession } from "../session";
import { AuthLayout } from "./Login";

export function Signup() {
  const { refresh } = useSession();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const submit = async (values: { email: string; password: string; confirm: string }) => {
    if (values.password !== values.confirm) {
      setError("两次输入的密码不一致");
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      await post<WebSession>("/v1/accounts", { email: values.email, password: values.password });
      await refresh();
    } catch (failure) {
      const status = failure instanceof ApiError ? failure.status : 0;
      setError(
        status === 409
          ? "该邮箱已注册"
          : status === 404
            ? "暂未开放注册，请联系管理员开户"
            : status === 503
              ? "暂无可用的 relay 节点，请稍后再试"
              : errorText(failure),
      );
    } finally {
      setBusy(false);
    }
  };
  return (
    <AuthLayout title="创建账号" subtitle="注册后在 yttt 应用里登录同一账号即可使用">
      {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
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
          autoComplete="new-password"
          placeholder="至少 8 个字符"
          rules={[{ required: true, min: 8, message: "至少 8 个字符" }]}
        />
        <Form.Input
          field="confirm"
          label="确认密码"
          mode="password"
          size="large"
          autoComplete="new-password"
          rules={[{ required: true, message: "请再输入一次" }]}
        />
        <Button htmlType="submit" theme="solid" size="large" block loading={busy} style={{ marginTop: 8 }}>
          注册并登录
        </Button>
      </Form>
      <Typography.Text type="tertiary" style={{ display: "block", marginTop: 20, textAlign: "center" }}>
        已有账号？<Link to="/">登录</Link>
      </Typography.Text>
    </AuthLayout>
  );
}
