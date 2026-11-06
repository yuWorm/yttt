import { useEffect, useState } from "react";
import { Card, Col, Row, Spin, Table, Typography } from "@douyinfe/semi-ui";
import { type NodeView, type Overview, formatBytes, formatTime, get } from "../../api";
import { NodeStatus } from "./status";

function Stat({ title, value }: { title: string; value: string | number }) {
  return (
    <Card>
      <Typography.Text type="secondary">{title}</Typography.Text>
      <Typography.Title heading={2} style={{ margin: "8px 0 0" }}>
        {value}
      </Typography.Title>
    </Card>
  );
}

export function AdminOverview() {
  const [overview, setOverview] = useState<Overview>();
  useEffect(() => {
    const load = () => void get<Overview>("/v1/admin/overview").then(setOverview);
    load();
    const timer = window.setInterval(load, 15000);
    return () => window.clearInterval(timer);
  }, []);
  if (!overview) {
    return <Spin />;
  }
  return (
    <>
      <Typography.Title heading={3} style={{ marginBottom: 16 }}>
        总览
      </Typography.Title>
      <Row gutter={16} style={{ marginBottom: 16 }}>
        <Col span={6}>
          <Stat title="账号" value={overview.accounts} />
        </Col>
        <Col span={6}>
          <Stat title="已封停" value={overview.suspended} />
        </Col>
        <Col span={6}>
          <Stat title="管理员" value={overview.admins} />
        </Col>
        <Col span={6}>
          <Stat title="本月转发" value={formatBytes(overview.month_upstream_bytes)} />
        </Col>
      </Row>
      <Card title="节点">
        <Table
          dataSource={overview.nodes}
          rowKey="id"
          pagination={false}
          columns={[
            { title: "节点", dataIndex: "id" },
            { title: "名称", dataIndex: "name" },
            { title: "状态", render: (_: unknown, node: NodeView) => <NodeStatus node={node} /> },
            { title: "在线设备", dataIndex: "connected_devices", render: (count: number | null) => count ?? "—" },
            { title: "账号", dataIndex: "accounts" },
            { title: "最后上报", dataIndex: "last_report_unix", render: (unix: number | null) => formatTime(unix) },
          ]}
        />
      </Card>
    </>
  );
}
