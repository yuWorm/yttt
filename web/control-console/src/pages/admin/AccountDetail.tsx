import { useCallback, useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import {
  Avatar,
  Button,
  Card,
  Col,
  Descriptions,
  Form,
  Modal,
  Popconfirm,
  Progress,
  Row,
  Select,
  Space,
  Spin,
  Table,
  Tag,
  Toast,
  Typography,
} from "@douyinfe/semi-ui";
import { IconArrowLeft, IconDelete, IconKey } from "@douyinfe/semi-icons";
import {
  type AccountDetail,
  type DeviceEntry,
  type NodeView,
  type PlanView,
  del,
  errorText,
  formatBytes,
  formatTime,
  get,
  patch,
  percentOf,
  post,
} from "../../api";
import { useSession } from "../../session";
import { UsageBars } from "../../components/UsageBars";
import { DailyChart, EmptyState, sumDays } from "../../components/ui";
import { randomPassword } from "./password";

export function AdminAccountDetail() {
  const { id } = useParams();
  const navigate = useNavigate();
  const { me } = useSession();
  const [detail, setDetail] = useState<AccountDetail>();
  const [plans, setPlans] = useState<PlanView[]>([]);
  const [nodes, setNodes] = useState<NodeView[]>([]);
  const [resetting, setResetting] = useState(false);
  const load = useCallback(async () => {
    setDetail(await get<AccountDetail>(`/v1/admin/accounts/${id}`));
  }, [id]);
  useEffect(() => {
    void load();
    void get<PlanView[]>("/v1/admin/plans").then(setPlans);
    void get<NodeView[]>("/v1/admin/nodes").then(setNodes);
  }, [load]);
  if (!detail) {
    return <Spin size="large" />;
  }
  const account = detail.account;
  const self = me?.account_id === account.account_id;
  const plan = plans.find((candidate) => candidate.id === account.plan);
  const change = (title: string, content: string | undefined, body: Record<string, unknown>) =>
    Modal.confirm({
      title,
      content,
      onOk: async () => {
        try {
          await patch(`/v1/admin/accounts/${account.account_id}`, body);
          Toast.success("已更新，节点会在下次上报（约 15 秒内）执行");
          await load();
        } catch (failure) {
          Toast.error(errorText(failure));
        }
      },
    });
  const removeDevice = async (device: DeviceEntry) => {
    try {
      await del(`/v1/admin/accounts/${account.account_id}/devices/${device.endpoint_id}`);
      Toast.success("已移除设备");
      await load();
    } catch (failure) {
      Toast.error(errorText(failure));
    }
  };
  const percent = percentOf(account.month_upstream_bytes, plan?.monthly_bytes ?? 0);
  return (
    <>
      <Button
        icon={<IconArrowLeft />}
        theme="borderless"
        type="tertiary"
        style={{ marginBottom: 12, marginLeft: -12 }}
        onClick={() => navigate("/admin/accounts")}
      >
        账号列表
      </Button>
      <div className="stack">
        <Card>
          <div style={{ display: "flex", justifyContent: "space-between", gap: 16, flexWrap: "wrap" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
              <Avatar size="large" color={account.role === "admin" ? "violet" : "light-blue"}>
                {account.email.charAt(0).toUpperCase()}
              </Avatar>
              <div>
                <Typography.Title heading={4} style={{ margin: 0 }}>
                  {account.email}
                </Typography.Title>
                <div style={{ marginTop: 6, display: "flex", gap: 6, alignItems: "center" }}>
                  {account.suspended ? <Tag color="red">已封停</Tag> : <Tag color="green">正常</Tag>}
                  {account.role === "admin" && <Tag color="violet">管理员</Tag>}
                  <Typography.Text type="tertiary" size="small">
                    #{account.account_id} · 注册于 {formatTime(account.created_unix)}
                  </Typography.Text>
                </div>
              </div>
            </div>
            <Space wrap>
              <Button icon={<IconKey />} disabled={self} onClick={() => setResetting(true)}>
                重置密码
              </Button>
              {account.role === "admin" ? (
                <Button disabled={self} onClick={() => change("撤销管理员？", "该账号的所有登录会退出。", { role: "user" })}>
                  撤销管理员
                </Button>
              ) : (
                <Button
                  onClick={() => change("设为管理员？", "此账号将能管理所有账号、节点与套餐。", { role: "admin" })}
                >
                  设为管理员
                </Button>
              )}
              {account.suspended ? (
                <Button theme="solid" onClick={() => change("解除封停？", undefined, { suspended: false })}>
                  解除封停
                </Button>
              ) : (
                <Button
                  type="danger"
                  theme="solid"
                  disabled={self}
                  onClick={() =>
                    change("封停此账号？", "它的所有 relay 连接会在约 15 秒内断开，所有登录会退出。", {
                      suspended: true,
                    })
                  }
                >
                  封停
                </Button>
              )}
            </Space>
          </div>
        </Card>
        <Row gutter={[16, 16]} type="flex">
          <Col xs={24} xl={9}>
            <Card title="套餐与节点" style={{ height: "100%" }}>
              <Descriptions
                align="left"
                data={[
                  {
                    key: "套餐",
                    value: (
                      <Select
                        value={account.plan}
                        style={{ width: 200 }}
                        optionList={plans.map((candidate) => ({
                          value: candidate.id,
                          label: candidate.name || candidate.id,
                          disabled: candidate.archived,
                        }))}
                        onChange={(value) =>
                          change(`改为套餐“${plans.find((candidate) => candidate.id === value)?.name ?? value}”？`, undefined, {
                            plan: value,
                          })
                        }
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
                          .map((node) => ({
                            value: node.id,
                            label: `${node.name}（${node.id}）`,
                            disabled: node.status === "disabled",
                          }))}
                        onChange={(value) =>
                          change(`迁到节点 ${String(value)}？`, "设备在续签 ticket 后改用新节点。", { node: value })
                        }
                      />
                    ),
                  },
                  { key: "设备", value: `${account.devices} / ${plan?.max_devices ?? "—"}` },
                  {
                    key: "本月用量",
                    value: (
                      <div style={{ width: 200 }}>
                        {formatBytes(account.month_upstream_bytes)} / {formatBytes(plan?.monthly_bytes ?? 0)}
                        <Progress
                          percent={percent}
                          size="small"
                          stroke={percent >= 100 ? "var(--semi-color-danger)" : undefined}
                        />
                      </div>
                    ),
                  },
                ]}
              />
            </Card>
          </Col>
          <Col xs={24} xl={15}>
            <Card
              title="近 30 天"
              headerExtraContent={<Typography.Text type="tertiary">{formatBytes(sumDays(detail.daily))}</Typography.Text>}
              style={{ height: "100%" }}
            >
              <DailyChart days={detail.daily} />
            </Card>
          </Col>
        </Row>
        <Card title={`设备（${detail.devices.length}）`} bodyStyle={{ padding: detail.devices.length ? 0 : undefined }}>
          {detail.devices.length === 0 ? (
            <EmptyState title="没有登记的设备" />
          ) : (
            <Table
              dataSource={detail.devices}
              rowKey="endpoint_id"
              pagination={false}
              columns={[
                { title: "名称", dataIndex: "label" },
                {
                  title: "设备密钥",
                  dataIndex: "endpoint_id",
                  render: (key: string) => (
                    <Typography.Text className="mono" copyable={{ content: key }}>
                      {key.slice(0, 16)}…
                    </Typography.Text>
                  ),
                },
                { title: "登记时间", dataIndex: "registered_unix", render: (unix: number) => formatTime(unix) },
                {
                  title: "",
                  align: "right",
                  render: (_: unknown, device: DeviceEntry) => (
                    <Popconfirm title="移除此设备？" content="它将无法再经 relay 连接。" onConfirm={() => removeDevice(device)}>
                      <Button type="danger" theme="borderless" icon={<IconDelete />}>
                        移除
                      </Button>
                    </Popconfirm>
                  ),
                },
              ]}
            />
          )}
        </Card>
        <Card title="近 12 个月">
          <UsageBars usage={detail.usage} quota={plan?.monthly_bytes} />
        </Card>
      </div>
      <ResetPassword
        accountId={account.account_id}
        email={account.email}
        visible={resetting}
        onClose={() => setResetting(false)}
      />
    </>
  );
}

function ResetPassword({
  accountId,
  email,
  visible,
  onClose,
}: {
  accountId: number;
  email: string;
  visible: boolean;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string>();
  const submit = async (values: { password: string }) => {
    setBusy(true);
    try {
      await post(`/v1/admin/accounts/${accountId}/password`, values);
      setDone(values.password);
    } catch (failure) {
      Toast.error(errorText(failure));
    } finally {
      setBusy(false);
    }
  };
  const close = () => {
    setDone(undefined);
    onClose();
  };
  return (
    <Modal title={`重置 ${email} 的密码`} visible={visible} onCancel={close} footer={null} maskClosable={false}>
      {done ? (
        <>
          <Typography.Paragraph>新密码已生效，该账号的所有登录已退出。请把密码告知对方：</Typography.Paragraph>
          <Typography.Paragraph
            copyable={{ content: done }}
            className="mono"
            style={{ fontSize: 16, padding: 12, borderRadius: 6, background: "var(--semi-color-fill-0)", margin: "12px 0" }}
          >
            {done}
          </Typography.Paragraph>
          <Button theme="solid" block onClick={close} style={{ marginBottom: 16 }}>
            完成
          </Button>
        </>
      ) : (
        <Form onSubmit={submit} labelPosition="top" initValues={{ password: randomPassword() }}>
          <Form.Input
            field="password"
            label="新密码"
            rules={[{ required: true, min: 8, message: "至少 8 个字符" }]}
            extraText="用户没有自助找回密码的途径；设置后请提醒对方登录后修改。"
          />
          <Button htmlType="submit" theme="solid" type="danger" block loading={busy} style={{ marginBottom: 16 }}>
            重置并注销该账号的所有登录
          </Button>
        </Form>
      )}
    </Modal>
  );
}
