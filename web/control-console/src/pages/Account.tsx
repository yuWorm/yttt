import { useState } from "react";
import { Avatar, Banner, Button, Card, Col, Descriptions, Form, Row, Tag, Toast, Typography } from "@douyinfe/semi-ui";
import { IconExit } from "@douyinfe/semi-icons";
import { ApiError, errorText, post } from "../api";
import { useSession } from "../session";
import { PageHeader } from "../components/ui";

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
      setError(failure instanceof ApiError && failure.status === 403 ? "当前密码不正确" : errorText(failure));
    } finally {
      setBusy(false);
    }
  };
  if (!me) {
    return null;
  }
  return (
    <>
      <PageHeader title="账号" />
      <Row gutter={[16, 16]} type="flex">
        <Col xs={24} lg={10}>
          <Card style={{ height: "100%" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 14, marginBottom: 20 }}>
              <Avatar size="large" color="light-blue">
                {me.email.charAt(0).toUpperCase()}
              </Avatar>
              <div style={{ minWidth: 0 }}>
                <Typography.Title heading={5} ellipsis={{ showTooltip: true }} style={{ margin: 0 }}>
                  {me.email}
                </Typography.Title>
                <div style={{ marginTop: 6, display: "flex", gap: 6 }}>
                  {me.suspended ? <Tag color="red">已封停</Tag> : <Tag color="green">正常</Tag>}
                  {me.role === "admin" && <Tag color="violet">管理员</Tag>}
                </div>
              </div>
            </div>
            <Descriptions
              data={[
                { key: "账号 ID", value: me.account_id },
                { key: "套餐", value: me.plan_name || me.plan },
                { key: "relay 节点", value: me.node_name || me.node_id },
              ]}
            />
            <Button icon={<IconExit />} style={{ marginTop: 20 }} onClick={() => void signOut()}>
              退出登录
            </Button>
          </Card>
        </Col>
        <Col xs={24} lg={14}>
          <Card title="修改密码" style={{ height: "100%" }}>
            {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
            <Form onSubmit={change} labelPosition="top" style={{ maxWidth: 420 }}>
              <Form.Input
                field="current"
                label="当前密码"
                mode="password"
                autoComplete="current-password"
                rules={[{ required: true, message: "请输入当前密码" }]}
              />
              <Form.Input
                field="new"
                label="新密码"
                mode="password"
                autoComplete="new-password"
                rules={[{ required: true, min: 8, message: "至少 8 个字符" }]}
              />
              <Form.Input
                field="confirm"
                label="确认新密码"
                mode="password"
                autoComplete="new-password"
                rules={[{ required: true, message: "请再输入一次" }]}
              />
              <Button htmlType="submit" theme="solid" loading={busy}>
                修改密码
              </Button>
              <Typography.Text type="tertiary" size="small" style={{ display: "block", marginTop: 12 }}>
                修改后，其他浏览器与设备上的登录会退出。
              </Typography.Text>
            </Form>
          </Card>
        </Col>
      </Row>
    </>
  );
}
