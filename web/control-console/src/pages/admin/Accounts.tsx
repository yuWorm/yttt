import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Avatar, Button, Card, Form, Input, Progress, Select, SideSheet, Table, Tag, Toast, Typography } from "@douyinfe/semi-ui";
import { IconPlus, IconSearch } from "@douyinfe/semi-icons";
import {
  type AccountPage,
  type AccountRow,
  type NodeView,
  type PlanView,
  type Settings,
  errorText,
  formatBytes,
  formatTime,
  get,
  percentOf,
  post,
} from "../../api";
import { EmptyState, PageHeader } from "../../components/ui";
import { randomPassword } from "./password";

const PAGE = 20;

export function AdminAccounts() {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [plan, setPlan] = useState<string>();
  const [status, setStatus] = useState<string>();
  const [page, setPage] = useState(1);
  const [result, setResult] = useState<AccountPage>();
  const [plans, setPlans] = useState<PlanView[]>([]);
  const [creating, setCreating] = useState(false);
  const load = useCallback(async () => {
    const params = new URLSearchParams({
      q: query,
      offset: String((page - 1) * PAGE),
      limit: String(PAGE),
      ...(plan ? { plan } : {}),
      ...(status ? { status } : {}),
    });
    setResult(await get<AccountPage>(`/v1/admin/accounts?${params}`));
  }, [query, page, plan, status]);
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    void get<PlanView[]>("/v1/admin/plans").then(setPlans);
  }, []);
  const quota = (row: AccountRow) => plans.find((candidate) => candidate.id === row.plan)?.monthly_bytes ?? 0;
  const planName = (id: string) => plans.find((candidate) => candidate.id === id)?.name || id;
  const filtered = query !== "" || plan !== undefined || status !== undefined;
  return (
    <>
      <PageHeader
        title="账号管理"
        description={result ? `共 ${result.total} 个账号` : undefined}
        extra={
          <Button icon={<IconPlus />} theme="solid" onClick={() => setCreating(true)}>
            新建账号
          </Button>
        }
      />
      <Card bodyStyle={{ padding: 0 }}>
        <div style={{ display: "flex", gap: 12, flexWrap: "wrap", padding: 16 }}>
          <Input
            prefix={<IconSearch />}
            placeholder="按邮箱搜索"
            showClear
            style={{ width: 260 }}
            onChange={(value) => {
              setPage(1);
              setQuery(value);
            }}
          />
          <Select
            placeholder="全部套餐"
            showClear
            style={{ width: 160 }}
            value={plan}
            onChange={(value) => {
              setPage(1);
              setPlan(value === undefined ? undefined : String(value));
            }}
            optionList={plans.map((candidate) => ({ value: candidate.id, label: candidate.name || candidate.id }))}
          />
          <Select
            placeholder="全部状态"
            showClear
            style={{ width: 140 }}
            value={status}
            onChange={(value) => {
              setPage(1);
              setStatus(value === undefined ? undefined : String(value));
            }}
            optionList={[
              { value: "active", label: "正常" },
              { value: "suspended", label: "已封停" },
              { value: "admin", label: "管理员" },
            ]}
          />
        </div>
        <Table
          loading={result === undefined}
          dataSource={result?.accounts}
          rowKey="account_id"
          empty={<EmptyState search={filtered} title={filtered ? "没有符合条件的账号" : "还没有账号"} />}
          onRow={(row?: AccountRow) => ({
            onClick: () => row && navigate(`/admin/accounts/${row.account_id}`),
            className: "clickable-row",
          })}
          pagination={{
            currentPage: page,
            pageSize: PAGE,
            total: result?.total ?? 0,
            onPageChange: setPage,
          }}
          columns={[
            {
              title: "账号",
              render: (_: unknown, row: AccountRow) => (
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <Avatar size="small" color={row.role === "admin" ? "violet" : "light-blue"}>
                    {row.email.charAt(0).toUpperCase()}
                  </Avatar>
                  <div>
                    <Typography.Text>{row.email}</Typography.Text>
                    <Typography.Text type="tertiary" size="small" style={{ display: "block" }}>
                      #{row.account_id}
                    </Typography.Text>
                  </div>
                </div>
              ),
            },
            {
              title: "状态",
              render: (_: unknown, row: AccountRow) => (
                <div style={{ display: "flex", gap: 4 }}>
                  {row.suspended ? <Tag color="red">已封停</Tag> : <Tag color="green">正常</Tag>}
                  {row.role === "admin" && <Tag color="violet">管理员</Tag>}
                </div>
              ),
            },
            { title: "套餐", dataIndex: "plan", render: (id: string) => planName(id) },
            { title: "节点", dataIndex: "node_id" },
            { title: "设备", dataIndex: "devices" },
            {
              title: "本月用量",
              width: 200,
              render: (_: unknown, row: AccountRow) => {
                const percent = percentOf(row.month_upstream_bytes, quota(row));
                return (
                  <div>
                    <Typography.Text size="small">
                      {formatBytes(row.month_upstream_bytes)}
                      <Typography.Text type="tertiary" size="small">
                        {" "}
                        / {formatBytes(quota(row))}
                      </Typography.Text>
                    </Typography.Text>
                    <Progress
                      percent={percent}
                      size="small"
                      stroke={percent >= 100 ? "var(--semi-color-danger)" : undefined}
                    />
                  </div>
                );
              },
            },
            { title: "注册时间", dataIndex: "created_unix", render: (unix: number) => formatTime(unix) },
          ]}
        />
      </Card>
      <NewAccount
        visible={creating}
        plans={plans}
        onClose={() => setCreating(false)}
        onCreated={(id) => {
          setCreating(false);
          navigate(`/admin/accounts/${id}`);
        }}
      />
    </>
  );
}

function NewAccount({
  visible,
  plans,
  onClose,
  onCreated,
}: {
  visible: boolean;
  plans: PlanView[];
  onClose: () => void;
  onCreated: (id: number) => void;
}) {
  const [nodes, setNodes] = useState<NodeView[]>([]);
  const [settings, setSettings] = useState<Settings>();
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (visible) {
      void get<NodeView[]>("/v1/admin/nodes").then(setNodes);
      void get<Settings>("/v1/admin/settings").then(setSettings);
    }
  }, [visible]);
  const submit = async (values: { email: string; password: string; plan: string; node: string; role: string }) => {
    setBusy(true);
    try {
      const created = await post<{ account_id: number }>("/v1/admin/accounts", values);
      Toast.success("账号已创建，请把密码告知对方");
      onCreated(created.account_id);
    } catch (failure) {
      Toast.error(errorText(failure));
    } finally {
      setBusy(false);
    }
  };
  return (
    <SideSheet title="新建账号" visible={visible} onCancel={onClose} width={440}>
      {settings && (
        <Form
          onSubmit={submit}
          labelPosition="top"
          initValues={{
            password: randomPassword(),
            plan: settings.signup_plan ?? undefined,
            node: settings.signup_node ?? undefined,
            role: "user",
          }}
        >
          {({ formApi }) => (
            <>
              <Form.Input field="email" label="邮箱" rules={[{ required: true, message: "请输入邮箱" }]} />
              <Form.Input
                field="password"
                label="初始密码"
                rules={[{ required: true, min: 8, message: "至少 8 个字符" }]}
                extraText="已随机生成，创建后请告知对方，并提醒登录后修改。"
                suffix={
                  <Button size="small" theme="borderless" onClick={() => formApi.setValue("password", randomPassword())}>
                    换一个
                  </Button>
                }
              />
              <Form.Select
                field="plan"
                label="套餐"
                style={{ width: "100%" }}
                rules={[{ required: true, message: "请选择套餐" }]}
                optionList={plans
                  .filter((plan) => !plan.archived)
                  .map((plan) => ({ value: plan.id, label: plan.name || plan.id }))}
              />
              <Form.Select
                field="node"
                label="relay 节点"
                style={{ width: "100%" }}
                rules={[{ required: true, message: "请选择节点" }]}
                optionList={nodes
                  .filter((node) => node.status !== "disabled")
                  .map((node) => ({ value: node.id, label: `${node.name}（${node.id}）` }))}
              />
              <Form.RadioGroup field="role" label="角色" type="button">
                <Form.Radio value="user">普通用户</Form.Radio>
                <Form.Radio value="admin">管理员</Form.Radio>
              </Form.RadioGroup>
              <Button htmlType="submit" theme="solid" block loading={busy} style={{ marginTop: 12 }}>
                创建账号
              </Button>
            </>
          )}
        </Form>
      )}
    </SideSheet>
  );
}
