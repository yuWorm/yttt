import { useState } from "react";
import { Banner, Button, Card, Descriptions, Form, Toast, Typography } from "@douyinfe/semi-ui";
import { ApiError, post } from "../api";
import { useSession } from "../session";

export function Account() {
  const { me, signOut } = useSession();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const change = async (values: { current: string; new: string; confirm: string }) => {
    if (values.new !== values.confirm) {
      setError("两次输入的新密码不一致");
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      await post("/v1/accounts/me/password", { current: values.current, new: values.new });
      Toast.success("密码已修改，其他设备上的登录已退出");
    } catch (failure) {
      setError(failure instanceof ApiError && failure.status === 403 ? "当前密码不正确" : String(failure));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Typography.Title heading={3} style={{ marginBottom: 16 }}>
        账号
      </Typography.Title>
      <Card style={{ marginBottom: 16 }}>
        <Descriptions
          data={[
            { key: "邮箱", value: me?.email },
            { key: "账号 ID", value: me?.account_id },
          ]}
        />
      </Card>
      <Card title="修改密码" style={{ maxWidth: 480 }}>
        {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
        <Form onSubmit={change} labelPosition="top">
          <Form.Input field="current" label="当前密码" mode="password" rules={[{ required: true, message: "请输入当前密码" }]} />
          <Form.Input field="new" label="新密码" mode="password" rules={[{ required: true, min: 8, message: "至少 8 个字符" }]} />
          <Form.Input field="confirm" label="确认新密码" mode="password" rules={[{ required: true, message: "请再输入一次" }]} />
          <Button htmlType="submit" theme="solid" loading={busy}>
            修改密码
          </Button>
        </Form>
      </Card>
      <Button style={{ marginTop: 16 }} onClick={() => void signOut()}>
        退出登录
      </Button>
    </>
  );
}
