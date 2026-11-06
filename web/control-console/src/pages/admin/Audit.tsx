import { useEffect, useState } from "react";
import { Card, Select, Table, Tag, Typography } from "@douyinfe/semi-ui";
import { type AuditEntry, formatTime, get } from "../../api";
import { EmptyState, PageHeader } from "../../components/ui";

const ACTIONS: Record<string, { text: string; color: "blue" | "green" | "orange" | "red" | "violet" | "grey" | "cyan" }> = {
  create_account: { text: "开户", color: "green" },
  reset_password: { text: "重置密码", color: "orange" },
  set_plan: { text: "修改套餐", color: "blue" },
  move_account: { text: "迁移账号", color: "blue" },
  suspend: { text: "封停", color: "red" },
  reinstate: { text: "解除封停", color: "green" },
  set_role: { text: "修改角色", color: "violet" },
  remove_device: { text: "移除设备", color: "orange" },
  create_plan: { text: "新建套餐", color: "green" },
  change_plan: { text: "修改套餐配置", color: "blue" },
  delete_plan: { text: "删除套餐", color: "red" },
  change_settings: { text: "修改设置", color: "violet" },
  create_node: { text: "新建节点", color: "green" },
  regenerate_enrollment: { text: "重新生成部署命令", color: "cyan" },
  rename_node: { text: "重命名节点", color: "blue" },
  set_node_url: { text: "修改节点地址", color: "blue" },
  disable_node: { text: "停用节点", color: "red" },
  enable_node: { text: "启用节点", color: "green" },
  move_node_accounts: { text: "迁移节点账号", color: "blue" },
  delete_node: { text: "删除节点", color: "red" },
};

export function AdminAudit() {
  const [entries, setEntries] = useState<AuditEntry[]>();
  const [action, setAction] = useState<string>();
  useEffect(() => {
    void get<AuditEntry[]>("/v1/admin/audit?limit=500").then(setEntries);
  }, []);
  const shown = entries?.filter((entry) => !action || entry.action === action);
  return (
    <>
      <PageHeader
        title="审计日志"
        description="管理员的每一次修改都记在这里（最近 500 条）。"
        extra={
          <Select
            placeholder="全部操作"
            showClear
            style={{ width: 200 }}
            value={action}
            onChange={(value) => setAction(value === undefined ? undefined : String(value))}
            optionList={Object.entries(ACTIONS).map(([value, { text }]) => ({ value, label: text }))}
          />
        }
      />
      <Card bodyStyle={{ padding: 0 }}>
        <Table
          loading={entries === undefined}
          dataSource={shown}
          rowKey="id"
          empty={<EmptyState search={action !== undefined} title="没有记录" />}
          pagination={{ pageSize: 50 }}
          columns={[
            { title: "时间", dataIndex: "at_unix", width: 190, render: (unix: number) => formatTime(unix) },
            { title: "管理员", dataIndex: "admin_email", width: 200 },
            {
              title: "操作",
              dataIndex: "action",
              width: 170,
              render: (value: string) => {
                const known = ACTIONS[value];
                return <Tag color={known?.color ?? "grey"}>{known?.text ?? value}</Tag>;
              },
            },
            {
              title: "对象",
              width: 180,
              render: (_: unknown, entry: AuditEntry) =>
                [entry.target_account && `账号 #${entry.target_account}`, entry.target_node && `节点 ${entry.target_node}`]
                  .filter(Boolean)
                  .join(" · ") || "—",
            },
            {
              title: "详情",
              dataIndex: "detail",
              render: (detail: string) =>
                detail ? (
                  <Typography.Text size="small" ellipsis={{ showTooltip: { opts: { style: { maxWidth: 480 } } } }}>
                    {detail}
                  </Typography.Text>
                ) : (
                  "—"
                ),
            },
          ]}
        />
      </Card>
    </>
  );
}
