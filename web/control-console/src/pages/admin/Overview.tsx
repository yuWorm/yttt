import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Button, Card, Col, Row, Spin, Table, Typography } from "@douyinfe/semi-ui";
import { IconCloud, IconHistogram, IconMonitorStroked, IconServer, IconUserGroup } from "@douyinfe/semi-icons";
import { type DayUsage, type NodeView, type Overview, formatBytes, formatTime, get } from "../../api";
import { DailyChart, PageHeader, StatCard, sumDays } from "../../components/ui";
import { NodeStatus } from "./status";

export function AdminOverview() {
  const navigate = useNavigate();
  const [overview, setOverview] = useState<Overview>();
  const [daily, setDaily] = useState<DayUsage[]>();
  useEffect(() => {
    const load = () => {
      void get<Overview>("/v1/admin/overview").then(setOverview);
      void get<DayUsage[]>("/v1/admin/usage/daily?days=30").then(setDaily);
    };
    load();
    const timer = window.setInterval(load, 15000);
    return () => window.clearInterval(timer);
  }, []);
  if (!overview) {
    return <Spin size="large" />;
  }
  const online = overview.nodes.filter((node) => node.status === "online").length;
  const active = overview.nodes.filter((node) => node.status !== "disabled" && node.status !== "pending").length;
  return (
    <>
      <PageHeader title="运营总览" description="每 15 秒刷新" />
      <div className="stack">
        <Row gutter={[16, 16]} type="flex">
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconUserGroup />}
              title="账号"
              value={overview.accounts}
              footer={`今日新增 ${overview.accounts_today} · 封停 ${overview.suspended}`}
            />
          </Col>
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconHistogram />}
              tone="violet"
              title="本月转发"
              value={formatBytes(overview.month_upstream_bytes)}
              footer={`今日 ${formatBytes(overview.today_upstream_bytes)}`}
            />
          </Col>
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconMonitorStroked />}
              tone="cyan"
              title="在线设备"
              value={overview.connected_devices}
              footer="各在线节点最近一次上报"
            />
          </Col>
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconServer />}
              tone={online < active ? "orange" : "green"}
              title="在线节点"
              value={`${online} / ${overview.nodes.length}`}
              footer={online < active ? `${active - online} 个节点离线` : "全部正常"}
            />
          </Col>
        </Row>
        <Card
          title="近 30 天全站转发"
          headerExtraContent={<Typography.Text type="tertiary">{daily ? formatBytes(sumDays(daily)) : ""}</Typography.Text>}
        >
          {daily ? <DailyChart days={daily} height={220} /> : <Spin />}
        </Card>
        <Card
          title="节点"
          headerExtraContent={
            <Button theme="borderless" icon={<IconCloud />} onClick={() => navigate("/admin/nodes")}>
              管理节点
            </Button>
          }
          bodyStyle={{ padding: 0 }}
        >
          <Table
            dataSource={overview.nodes}
            rowKey="id"
            pagination={false}
            columns={[
              {
                title: "节点",
                render: (_: unknown, node: NodeView) => (
                  <div>
                    <Typography.Text strong>{node.name}</Typography.Text>
                    <Typography.Text type="tertiary" size="small" style={{ display: "block" }}>
                      {node.id}
                    </Typography.Text>
                  </div>
                ),
              },
              { title: "状态", render: (_: unknown, node: NodeView) => <NodeStatus node={node} /> },
              { title: "在线设备", dataIndex: "connected_devices", render: (count: number | null) => count ?? "—" },
              { title: "账号", dataIndex: "accounts" },
              { title: "本月流量", dataIndex: "month_bytes", render: (bytes: number) => formatBytes(bytes) },
              { title: "最后上报", dataIndex: "last_report_unix", render: (unix: number | null) => formatTime(unix) },
            ]}
          />
        </Card>
      </div>
    </>
  );
}
