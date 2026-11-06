import { useEffect, useState } from "react";
import { Card, Table, Typography } from "@douyinfe/semi-ui";
import { type AuditEntry, formatTime, get } from "../../api";

const ACTIONS: Record<string, string> = {
  set_plan: "修改套餐",
  move_account: "迁移账号",
  suspend: "封停",
  reinstate: "解除封停",
  set_role: "修改角色",
  remove_device: "移除设备",
  create_node: "新建节点",
  regenerate_enrollment: "重新生成部署命令",
  rename_node: "重命名节点",
  set_node_url: "修改节点地址",
  disable_node: "停用节点",
  enable_node: "启用节点",
  move_node_accounts: "迁移节点账号",
  delete_node: "删除节点",
};

export function AdminAudit() {
  const [entries, setEntries] = useState<AuditEntry[]>();
  useEffect(() => {
    void get<AuditEntry[]>("/v1/admin/audit?limit=200").then(setEntries);
  }, []);
  return (
    <>
      <Typography.Title heading={3} style={{ marginBottom: 16 }}>
        审计日志
      </Typography.Title>
      <Card>
        <Table
          loading={entries === undefined}
          dataSource={entries}
          rowKey="id"
          pagination={{ pageSize: 50 }}
          columns={[
            { title: "时间", dataIndex: "at_unix", render: (unix: number) => formatTime(unix) },
            { title: "管理员", dataIndex: "admin_email" },
            { title: "操作", dataIndex: "action", render: (action: string) => ACTIONS[action] ?? action },
            {
              title: "对象",
              render: (_: unknown, entry: AuditEntry) =>
                [entry.target_account && `账号 ${entry.target_account}`, entry.target_node && `节点 ${entry.target_node}`]
                  .filter(Boolean)
                  .join(" · ") || "—",
            },
            { title: "详情", dataIndex: "detail" },
          ]}
        />
      </Card>
    </>
  );
}
