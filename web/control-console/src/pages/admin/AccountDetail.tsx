import { useCallback, useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { Button, Card, Descriptions, Modal, Popconfirm, Select, Space, Table, Tag, Toast, Typography } from "@douyinfe/semi-ui";
import { IconArrowLeft } from "@douyinfe/semi-icons";
import {
  type AccountDetail,
  type DeviceEntry,
  type NodeView,
  type PlanView,
  ApiError,
  del,
  formatBytes,
  formatTime,
  get,
  patch,
} from "../../api";
import { useSession } from "../../session";
import { UsageBars } from "../../components/UsageBars";

export function AdminAccountDetail() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { me } = useSession();
  const [detail, setDetail] = useState<AccountDetail>();
  const [plans, setPlans] = useState<PlanView[]>([]);
  const [nodes, setNodes] = useState<NodeView[]>([]);
  const load = useCallback(async () => {
    setDetail(await get<AccountDetail>(`/v1/admin/accounts/${id}`));
  }, [id]);
  useEffect(() => {
    void load();
    void get<PlanView[]>("/v1/admin/plans").then(setPlans);
    void get<NodeView[]>("/v1/admin/nodes").then(setNodes);
  }, [load]);
  if (!detail) {
    return null;
  }
  const account = detail.account;
  const self = me?.account_id === account.account_id;
  const change = (title: string, body: Record<string, unknown>) =>
    Modal.confirm({
      title,
      onOk: async () => {
        try {
          await patch(`/v1/admin/accounts/${account.account_id}`, body);
          Toast.success("已更新，节点会在下次上报（约 15 秒内）执行");
          await load();
        } catch (failure) {
          Toast.error(failure instanceof ApiError ? failure.message : String(failure));
        }
      },
    });
  const removeDevice = async (device: DeviceEntry) => {
    await del(`/v1/admin/accounts/${account.account_id}/devices/${device.endpoint_id}`);
    Toast.success("已移除设备");
    await load();
  };
  return (
    <Space vertical align="start" spacing="loose" style={{ width: "100%" }}>
      <Button icon={<IconArrowLeft />} theme="borderless" onClick={() => navigate("/admin/accounts")}>
        返回
      </Button>
      <Typography.Title heading={3}>
        {account.email}{" "}
        {account.suspended ? <Tag color="red">已封停</Tag> : <Tag color="green">正常</Tag>}
        {account.role === "admin" && <Tag color="blue">管理员</Tag>}
      </Typography.Title>
      <Card title="账号" style={{ width: "100%" }}>
        <Descriptions
          data={[
            { key: "账号 ID", value: account.account_id },
            { key: "注册时间", value: formatTime(account.created_unix) },
            { key: "本月用量", value: formatBytes(account.month_upstream_bytes) },
            {
              key: "套餐",
              value: (
                <Select
                  value={account.plan}
                  style={{ width: 200 }}
                  optionList={plans.map((plan) => ({ value: plan.id, label: plan.id }))}
                  onChange={(plan) => change(`把套餐改为 ${String(plan)}？`, { plan })}
                />
              ),
            },
            {
              key: "节点",
              value: (
                <Select
                  value={account.node_id}
                  style={{ width: 200 }}
                  optionList={nodes
                    .filter((node) => node.status !== "pending")
                    .map((node) => ({ value: node.id, label: `${node.id}（${node.name}）` }))}
                  onChange={(node) =>
                    change(`把账号迁到节点 ${String(node)}？`, { node })
                  }
                />
              ),
            },
          ]}
        />
        <Space style={{ marginTop: 16 }}>
          {account.suspended ? (
            <Button onClick={() => change("解除封停？", { suspended: false })}>解除封停</Button>
          ) : (
            <Button type="danger" disabled={self} onClick={() => change("封停此账号？它的所有连接会断开，所有登录会退出。", { suspended: true })}>
              封停
            </Button>
          )}
          {account.role === "admin" ? (
            <Button disabled={self} onClick={() => change("撤销管理员？", { role: "user" })}>
              撤销管理员
            </Button>
          ) : (
            <Button onClick={() => change("设为管理员？此账号将能管理所有账号与节点。", { role: "admin" })}>设为管理员</Button>
          )}
        </Space>
      </Card>
      <Card title={`设备（${detail.devices.length}）`} style={{ width: "100%" }}>
        <Table
          dataSource={detail.devices}
          rowKey="endpoint_id"
          pagination={false}
          columns={[
            { title: "名称", dataIndex: "label" },
            { title: "设备密钥", dataIndex: "endpoint_id", render: (key: string) => `${key.slice(0, 16)}…` },
            { title: "登记时间", dataIndex: "registered_unix", render: (unix: number) => formatTime(unix) },
            {
              title: "",
              render: (_: unknown, device: DeviceEntry) => (
                <Popconfirm title="移除此设备？" onConfirm={() => removeDevice(device)}>
                  <Button type="danger" theme="borderless">
                    移除
                  </Button>
                </Popconfirm>
              ),
            },
          ]}
        />
      </Card>
      <Card title="用量" style={{ width: "100%" }}>
        <UsageBars usage={detail.usage} quota={plans.find((plan) => plan.id === account.plan)?.monthly_bytes} />
      </Card>
    </Space>
  );
}
