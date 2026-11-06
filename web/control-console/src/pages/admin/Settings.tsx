import { useEffect, useState } from "react";
import { Banner, Button, Card, Form, Spin, Toast, Typography } from "@douyinfe/semi-ui";
import { type NodeView, type PlanView, type Settings, errorText, get, patch } from "../../api";
import { PageHeader } from "../../components/ui";

export function AdminSettings() {
  const [settings, setSettings] = useState<Settings>();
  const [plans, setPlans] = useState<PlanView[]>([]);
  const [nodes, setNodes] = useState<NodeView[]>([]);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void get<Settings>("/v1/admin/settings").then(setSettings);
    void get<PlanView[]>("/v1/admin/plans").then(setPlans);
    void get<NodeView[]>("/v1/admin/nodes").then(setNodes);
  }, []);
  const submit = async (values: Settings) => {
    setBusy(true);
    try {
      await patch("/v1/admin/settings", values);
      setSettings(values);
      Toast.success("已保存");
    } catch (failure) {
      Toast.error(errorText(failure));
    } finally {
      setBusy(false);
    }
  };
  if (!settings) {
    return <Spin size="large" />;
  }
  const node = nodes.find((candidate) => candidate.id === settings.signup_node);
  return (
    <>
      <PageHeader title="设置" description="这里的设置保存在数据库，立即生效，不需要重启控制面。" />
      <Card title="注册" style={{ maxWidth: 640 }}>
        {settings.signup_enabled && node && node.status !== "online" && (
          <Banner
            type="warning"
            fullMode={false}
            closeIcon={null}
            description={`注册节点 ${node.id} 当前${node.status === "pending" ? "尚未部署" : "不在线"}，新用户暂时无法注册。`}
            style={{ marginBottom: 16 }}
          />
        )}
        <Form<Settings> onSubmit={submit} labelPosition="left" labelWidth={120} initValues={settings}>
          <Form.Switch field="signup_enabled" label="开放注册" />
          <Form.Select
            field="signup_plan"
            label="新账号套餐"
            style={{ width: 280 }}
            optionList={plans
              .filter((plan) => !plan.archived)
              .map((plan) => ({ value: plan.id, label: plan.name || plan.id }))}
          />
          <Form.Select
            field="signup_node"
            label="新账号节点"
            style={{ width: 280 }}
            optionList={nodes
              .filter((candidate) => candidate.status !== "disabled")
              .map((candidate) => ({ value: candidate.id, label: `${candidate.name}（${candidate.id}）` }))}
          />
          <Typography.Text type="tertiary" size="small" style={{ display: "block", margin: "4px 0 16px 120px" }}>
            关闭注册后，仍可在“账号管理”里为用户开户。
          </Typography.Text>
          <Button htmlType="submit" theme="solid" loading={busy} style={{ marginLeft: 120 }}>
            保存
          </Button>
        </Form>
      </Card>
    </>
  );
}
