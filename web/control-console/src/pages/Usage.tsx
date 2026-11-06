import { useEffect, useState } from "react";
import { Card, Col, Row, Spin, Typography } from "@douyinfe/semi-ui";
import { IconCalendar, IconHistogram, IconPieChartStroked } from "@douyinfe/semi-icons";
import { type DayUsage, type MonthUsage, formatBytes, get } from "../api";
import { useSession } from "../session";
import { UsageBars } from "../components/UsageBars";
import { DailyChart, PageHeader, StatCard, sumDays } from "../components/ui";

export function Usage() {
  const { me } = useSession();
  const [months, setMonths] = useState<MonthUsage[]>();
  const [daily, setDaily] = useState<DayUsage[]>();
  useEffect(() => {
    void get<MonthUsage[]>("/v1/accounts/me/usage").then(setMonths);
    void get<DayUsage[]>("/v1/accounts/me/usage/daily").then(setDaily);
  }, []);
  const today = daily?.[daily.length - 1]?.upstream_bytes ?? 0;
  const left = me ? Math.max(me.monthly_bytes - me.month_upstream_bytes, 0) : 0;
  return (
    <>
      <PageHeader title="用量" description="按 relay 节点实际转发的上行字节计，直连的流量不计入；按 UTC 自然月重置。" />
      <div className="stack">
        <Row gutter={[16, 16]} type="flex">
          <Col xs={24} md={8}>
            <StatCard icon={<IconCalendar />} title="今日" value={formatBytes(today)} />
          </Col>
          <Col xs={24} md={8}>
            <StatCard
              icon={<IconHistogram />}
              tone="violet"
              title="本月"
              value={me ? formatBytes(me.month_upstream_bytes) : "—"}
            />
          </Col>
          <Col xs={24} md={8}>
            <StatCard
              icon={<IconPieChartStroked />}
              tone="green"
              title="本月剩余额度"
              value={formatBytes(left)}
              footer={me ? `共 ${formatBytes(me.monthly_bytes)}` : undefined}
            />
          </Col>
        </Row>
        <Card
          title="近 30 天"
          headerExtraContent={<Typography.Text type="tertiary">{daily ? formatBytes(sumDays(daily)) : ""}</Typography.Text>}
        >
          {daily ? <DailyChart days={daily} height={220} /> : <Spin />}
        </Card>
        <Card title="近 12 个月">
          {months ? <UsageBars usage={months} quota={me?.monthly_bytes} /> : <Spin />}
        </Card>
      </div>
    </>
  );
}
