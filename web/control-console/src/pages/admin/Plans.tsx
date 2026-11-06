import { useCallback, useEffect, useState } from "react";
import { Banner, Button, Card, Dropdown, Form, Modal, SideSheet, Spin, Tag, Toast, Typography } from "@douyinfe/semi-ui";
import { IconEdit, IconMore, IconPlus } from "@douyinfe/semi-icons";
import {
  GIB,
  MBPS,
  type NodeView,
  type PlanView,
  type Settings,
  del,
  errorText,
  formatBytes,
  formatRate,
  get,
  patch,
  post,
} from "../../api";
import { EmptyState, PageHeader } from "../../components/ui";

/** The form edits rates in Mbps and quota in GB; the API takes bytes. */
interface PlanForm {
  id: string;
  name: string;
  description: string;
  rate_mbps: number;
  burst_mb: number;
  monthly_gb: number;
  throttled_mbps: number;
  max_devices: number;
  max_nodes: number;
  nodes: string[];
  sort: number;
}

const round = (value: number) => Math.round(value * 100) / 100;

function toForm(plan: PlanView): PlanForm {
  return {
    id: plan.id,
    name: plan.name,
    description: plan.description,
    rate_mbps: round(plan.rate_bps / MBPS),
    burst_mb: round(plan.burst_bytes / 1024 ** 2),
    monthly_gb: round(plan.monthly_bytes / GIB),
    throttled_mbps: round(plan.throttled_rate_bps / MBPS),
    max_devices: plan.max_devices,
    max_nodes: plan.max_nodes,
    nodes: plan.nodes,
    sort: plan.sort,
  };
}

function toApi(form: PlanForm) {
  return {
    name: form.name,
    description: form.description ?? "",
    rate_bps: Math.round(form.rate_mbps * MBPS),
    burst_bytes: Math.round(form.burst_mb * 1024 ** 2),
    monthly_bytes: Math.round(form.monthly_gb * GIB),
    throttled_rate_bps: Math.round(form.throttled_mbps * MBPS),
    max_devices: form.max_devices,
    max_nodes: form.max_nodes,
    sort: form.sort ?? 0,
  };
}

const NEW_PLAN: Partial<PlanForm> = {
  rate_mbps: 16,
  burst_mb: 0.5,
  monthly_gb: 50,
  throttled_mbps: 0.5,
  max_devices: 5,
  max_nodes: 1,
  sort: 0,
};

export function AdminPlans() {
  const [plans, setPlans] = useState<PlanView[]>();
  const [settings, setSettings] = useState<Settings>();
  const [editing, setEditing] = useState<PlanView | "new">();
  const [nodes, setNodes] = useState<NodeView[]>([]);
  const load = useCallback(async () => {
    const [nextPlans, nextSettings, nextNodes] = await Promise.all([
      get<PlanView[]>("/v1/admin/plans"),
      get<Settings>("/v1/admin/settings"),
      get<NodeView[]>("/v1/admin/nodes"),
    ]);
    setPlans(nextPlans);
    setSettings(nextSettings);
    setNodes(nextNodes);
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
  const run = async (action: () => Promise<unknown>, done: string) => {
    try {
      await action();
      Toast.success(done);
      await load();
    } catch (failure) {
      Toast.error(errorText(failure));
    }
  };
  const archive = (plan: PlanView, archived: boolean) =>
    Modal.confirm({
      title: archived ? `归档套餐“${plan.name}”？` : `恢复套餐“${plan.name}”？`,
      content: archived ? "已有的账号保持不变，但不能再把新账号设为此套餐。" : "此套餐可以再分配给账号。",
      onOk: () => run(() => patch(`/v1/admin/plans/${plan.id}`, { archived }), archived ? "已归档" : "已恢复"),
    });
  const remove = (plan: PlanView) =>
    Modal.confirm({
      title: `删除套餐“${plan.name}”？`,
      content: "删除后无法恢复。",
      okButtonProps: { type: "danger" },
      onOk: () => run(() => del(`/v1/admin/plans/${plan.id}`), "已删除"),
    });
  return (
    <>
      <PageHeader
        title="套餐"
        description="速率与额度在节点下一次上报（约 15 秒）时对所有在用账号生效；调低设备上限只限制新登记。"
        extra={
          <Button icon={<IconPlus />} theme="solid" onClick={() => setEditing("new")}>
            新建套餐
          </Button>
        }
      />
      {plans === undefined ? (
        <Spin size="large" />
      ) : plans.length === 0 ? (
        <Card>
          <EmptyState title="还没有套餐" description="新建一个套餐，再在“设置”里把它设为新注册账号的套餐。" />
        </Card>
      ) : (
        <div className="plan-grid">
          {plans.map((plan) => (
            <Card
              key={plan.id}
              shadows="hover"
              style={{ opacity: plan.archived ? 0.65 : 1 }}
              title={
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <Typography.Title heading={5} style={{ margin: 0 }}>
                    {plan.name || plan.id}
                  </Typography.Title>
                  {settings?.signup_plan === plan.id && <Tag color="blue">注册默认</Tag>}
                  {plan.archived && <Tag color="grey">已归档</Tag>}
                </div>
              }
              headerExtraContent={
                <Dropdown
                  trigger="click"
                  position="bottomRight"
                  clickToHide
                  render={
                    <Dropdown.Menu>
                      <Dropdown.Item icon={<IconEdit />} onClick={() => setEditing(plan)}>
                        编辑
                      </Dropdown.Item>
                      <Dropdown.Item onClick={() => archive(plan, !plan.archived)}>
                        {plan.archived ? "恢复" : "归档"}
                      </Dropdown.Item>
                      <Dropdown.Item type="danger" disabled={plan.accounts > 0} onClick={() => remove(plan)}>
                        删除
                      </Dropdown.Item>
                    </Dropdown.Menu>
                  }
                >
                  <Button icon={<IconMore />} theme="borderless" type="tertiary" />
                </Dropdown>
              }
            >
              <Typography.Text type="tertiary" size="small" className="mono">
                {plan.id}
              </Typography.Text>
              <Typography.Paragraph
                type="secondary"
                ellipsis={{ rows: 2, showTooltip: true }}
                style={{ margin: "8px 0 12px", minHeight: 40 }}
              >
                {plan.description || "—"}
              </Typography.Paragraph>
              <div style={{ display: "flex", alignItems: "baseline", gap: 6, marginBottom: 8 }}>
                <span style={{ fontSize: 26, fontWeight: 600 }}>{formatBytes(plan.monthly_bytes)}</span>
                <Typography.Text type="tertiary">/ 月</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">速率</Typography.Text>
                <Typography.Text>{formatRate(plan.rate_bps)}</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">突发</Typography.Text>
                <Typography.Text>{formatBytes(plan.burst_bytes)}</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">超额后速率</Typography.Text>
                <Typography.Text>{formatRate(plan.throttled_rate_bps)}</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">设备上限</Typography.Text>
                <Typography.Text>{plan.max_devices}</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">节点</Typography.Text>
                <Typography.Text>
                  {plan.nodes.length} 个可用，同时 {plan.max_nodes} 个
                </Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">使用中的账号</Typography.Text>
                <Typography.Text strong>{plan.accounts}</Typography.Text>
              </div>
            </Card>
          ))}
        </div>
      )}
      <PlanEditor
        plan={editing}
        nodes={nodes}
        onClose={() => setEditing(undefined)}
        onSaved={async () => {
          setEditing(undefined);
          await load();
        }}
      />
    </>
  );
}

function PlanEditor({
  plan,
  nodes,
  onClose,
  onSaved,
}: {
  plan: PlanView | "new" | undefined;
  nodes: NodeView[];
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const creating = plan === "new";
  const submit = async (form: PlanForm) => {
    setBusy(true);
    try {
      if (creating) {
        await post("/v1/admin/plans", { id: form.id, ...toApi(form) });
        // A new plan starts on every node; narrow it when fewer were chosen.
        if (form.nodes?.length && form.nodes.length < nodes.length) {
          await patch(`/v1/admin/plans/${form.id}`, { nodes: form.nodes });
        }
        Toast.success("套餐已创建");
      } else if (plan) {
        await patch(`/v1/admin/plans/${plan.id}`, { ...toApi(form), nodes: form.nodes });
        Toast.success("已保存，约 15 秒内对在用账号生效");
      }
      await onSaved();
    } catch (failure) {
      Toast.error(errorText(failure));
    } finally {
      setBusy(false);
    }
  };
  const positive = { type: "number" as const, min: 0.001, message: "必须大于 0" };
  return (
    <SideSheet
      title={creating ? "新建套餐" : `编辑套餐 ${plan ? (plan as PlanView).name : ""}`}
      visible={plan !== undefined}
      onCancel={onClose}
      width={480}
    >
      {plan !== undefined && (
        <Form<PlanForm>
          key={creating ? "new" : (plan as PlanView).id}
          onSubmit={submit}
          labelPosition="top"
          initValues={
            creating ? ({ ...NEW_PLAN, nodes: nodes.map((node) => node.id) } as PlanForm) : toForm(plan as PlanView)
          }
        >
          {!creating && (plan as PlanView).accounts > 0 && (
            <Banner
              type="info"
              closeIcon={null}
              fullMode={false}
              description={`${(plan as PlanView).accounts} 个账号正在使用此套餐，修改会对它们生效。`}
              style={{ marginBottom: 12 }}
            />
          )}
          <Form.Input
            field="id"
            label="套餐 ID"
            disabled={!creating}
            placeholder="pro"
            extraText={creating ? "小写字母、数字与连字符，创建后不能修改" : undefined}
            rules={[
              { required: true, message: "请输入套餐 ID" },
              { pattern: /^[a-z0-9-]{1,32}$/, message: "小写字母、数字与连字符，至多 32 个" },
            ]}
          />
          <Form.Input field="name" label="名称" placeholder="专业版" rules={[{ required: true, message: "请输入名称" }]} />
          <Form.TextArea field="description" label="说明" placeholder="给用户看的一句话介绍" maxCount={500} autosize />
          <Form.InputNumber
            field="monthly_gb"
            label="每月额度"
            suffix="GB"
            min={0}
            style={{ width: "100%" }}
            rules={[{ required: true, message: "请输入额度" }]}
          />
          <Form.InputNumber
            field="rate_mbps"
            label="速率"
            suffix="Mbps"
            min={0}
            style={{ width: "100%" }}
            extraText="整个账号经 relay 的持续上行速率。"
            rules={[{ required: true, message: "请输入速率" }, positive]}
          />
          <Form.InputNumber
            field="burst_mb"
            label="突发"
            suffix="MB"
            min={0}
            style={{ width: "100%" }}
            extraText="短时间内可超出速率发送的量，让交互更跟手。"
            rules={[{ required: true, message: "请输入突发" }, positive]}
          />
          <Form.InputNumber
            field="throttled_mbps"
            label="超额后速率"
            suffix="Mbps"
            min={0}
            style={{ width: "100%" }}
            rules={[{ required: true, message: "请输入超额后速率" }, positive]}
          />
          <Form.InputNumber
            field="max_devices"
            label="设备上限"
            min={1}
            precision={0}
            style={{ width: "100%" }}
            rules={[{ required: true, message: "请输入设备上限" }]}
          />
          <Form.Select
            field="nodes"
            label="可用节点"
            multiple
            style={{ width: "100%" }}
            optionList={nodes.map((node) => ({
              value: node.id,
              label: node.region ? `${node.name}（${node.region}）` : node.name,
            }))}
            extraText="用户只能把 Host 放在这些节点上。移出某节点后，其上的 Host 在下次取 ticket 时回到账号默认节点。"
            rules={[{ required: true, type: "array", min: 1, message: "至少选择一个节点" }]}
          />
          <Form.InputNumber
            field="max_nodes"
            label="同时使用的节点数"
            min={1}
            precision={0}
            style={{ width: "100%" }}
            extraText="每个节点各自限速，账号的 Host 分布在 N 个节点上最多得到 N 倍速率。为 1 时，切换节点会让整个账号一起迁移。"
            rules={[{ required: true, message: "请输入节点数" }]}
          />
          <Form.InputNumber
            field="sort"
            label="排序"
            precision={0}
            style={{ width: "100%" }}
            extraText="数字小的排在前面。"
          />
          <Button htmlType="submit" theme="solid" block loading={busy} style={{ margin: "12px 0 24px" }}>
            {creating ? "创建套餐" : "保存"}
          </Button>
        </Form>
      )}
    </SideSheet>
  );
}
