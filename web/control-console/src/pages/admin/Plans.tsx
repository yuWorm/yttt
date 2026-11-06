import { useEffect, useState } from "react";
import { Card, Table, Typography } from "@douyinfe/semi-ui";
import { type PlanView, formatBytes, formatRate, get } from "../../api";

export function AdminPlans() {
  const [plans, setPlans] = useState<PlanView[]>();
  useEffect(() => {
    void get<PlanView[]>("/v1/admin/plans").then(setPlans);
  }, []);
  return (
    <>
      <Typography.Title heading={3} style={{ marginBottom: 16 }}>
        套餐
      </Typography.Title>
      <Card>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 16 }}>
          套餐在控制面的配置文件里定义，重启后生效。
        </Typography.Paragraph>
        <Table
          loading={plans === undefined}
          dataSource={plans}
          rowKey="id"
          pagination={false}
          columns={[
            { title: "套餐", dataIndex: "id" },
            { title: "速率", dataIndex: "rate_bps", render: (rate: number) => formatRate(rate) },
            { title: "突发", dataIndex: "burst_bytes", render: (bytes: number) => formatBytes(bytes) },
            { title: "每月额度", dataIndex: "monthly_bytes", render: (bytes: number) => formatBytes(bytes) },
            { title: "超额后速率", dataIndex: "throttled_rate_bps", render: (rate: number) => formatRate(rate) },
            { title: "设备上限", dataIndex: "max_devices" },
            { title: "账号数", dataIndex: "accounts" },
          ]}
        />
      </Card>
    </>
  );
}
