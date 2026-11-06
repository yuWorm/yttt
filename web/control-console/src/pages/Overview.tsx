import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Banner, Button, Card, Col, Progress, Row, Tag, Typography } from "@douyinfe/semi-ui";
import { IconBolt, IconCloud, IconHistogram, IconMonitorStroked } from "@douyinfe/semi-icons";
import { type DayUsage, formatBytes, formatRate, get, percentOf } from "../api";
import { useSession } from "../session";
import { DailyChart, PageHeader, StatCard, sumDays } from "../components/ui";

const STEPS = [
  { title: "在 Host 上登录", text: "打开 yttt →“远程访问此电脑”→“relay 账号”，用此账号登录并开启 P2P。" },
  { title: "配对设备", text: "在另一台设备上登录同一账号，扫 Host 显示的二维码完成配对。" },
  { title: "随处连接", text: "设备之间优先直连，打不通时自动经你的 relay 节点中转。" },
];

export function Overview() {
  const { me } = useSession();
  const navigate = useNavigate();
  const [daily, setDaily] = useState<DayUsage[]>();
  useEffect(() => {
    void get<DayUsage[]>("/v1/accounts/me/usage/daily").then(setDaily);
  }, []);
  if (!me) {
    return null;
  }
  const percent = percentOf(me.month_upstream_bytes, me.monthly_bytes);
  const over = me.monthly_bytes > 0 && me.month_upstream_bytes >= me.monthly_bytes;
  const planName = me.plan_name || me.plan;
  return (
    <>
      <PageHeader title="概览" description={`${me.email} · ${planName}`} />
      <div className="stack">
        {me.suspended && (
          <Banner
            type="danger"
            fullMode={false}
            title="账号已被封停"
            description="relay 暂不可用。如有疑问请联系管理员。"
            closeIcon={null}
          />
        )}
        {!me.suspended && over && (
          <Banner
            type="warning"
            fullMode={false}
            title="本月额度已用完"
            description={`relay 已降速到 ${formatRate(me.throttled_rate_bps)}，下月 1 日（UTC）恢复。`}
            closeIcon={null}
          />
        )}
        <Row gutter={[16, 16]} type="flex">
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconHistogram />}
              title="本月用量"
              value={formatBytes(me.month_upstream_bytes)}
              footer={
                <>
                  <Progress
                    percent={percent}
                    size="small"
                    stroke={over ? "var(--semi-color-danger)" : undefined}
                    style={{ margin: "6px 0 4px" }}
                  />
                  额度 {formatBytes(me.monthly_bytes)}
                </>
              }
            />
          </Col>
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconMonitorStroked />}
              tone="violet"
              title="已登记设备"
              value={`${me.devices} / ${me.max_devices}`}
              footer="每把设备密钥占一个名额"
            />
          </Col>
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconBolt />}
              tone={over ? "orange" : "green"}
              title="当前速率上限"
              value={formatRate(over ? me.throttled_rate_bps : me.rate_bps)}
              footer={over ? "已超额降速" : `超额后 ${formatRate(me.throttled_rate_bps)}`}
            />
          </Col>
          <Col xs={24} sm={12} xl={6}>
            <StatCard
              icon={<IconCloud />}
              tone="cyan"
              title="relay 节点"
              value={me.node_name || me.node_id}
              footer={<span className="mono">{me.relay_url || "节点尚未部署"}</span>}
            />
          </Col>
        </Row>
        <Row gutter={[16, 16]} type="flex">
          <Col xs={24} xl={16}>
            <Card
              title="近 30 天经 relay 转发"
              headerExtraContent={
                <Typography.Text type="tertiary">{daily ? formatBytes(sumDays(daily)) : ""}</Typography.Text>
              }
              style={{ height: "100%" }}
            >
              {daily && <DailyChart days={daily} />}
              <Typography.Text type="tertiary" size="small" style={{ display: "block", marginTop: 12 }}>
                直连的流量不计入用量。
              </Typography.Text>
            </Card>
          </Col>
          <Col xs={24} xl={8}>
            <Card
              title="我的套餐"
              headerExtraContent={<Tag color="blue">{planName}</Tag>}
              style={{ height: "100%" }}
            >
              {me.plan_description && (
                <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
                  {me.plan_description}
                </Typography.Paragraph>
              )}
              <div className="spec">
                <Typography.Text type="tertiary">速率</Typography.Text>
                <Typography.Text>{formatRate(me.rate_bps)}</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">每月额度</Typography.Text>
                <Typography.Text>{formatBytes(me.monthly_bytes)}</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">超额后速率</Typography.Text>
                <Typography.Text>{formatRate(me.throttled_rate_bps)}</Typography.Text>
              </div>
              <div className="spec">
                <Typography.Text type="tertiary">设备上限</Typography.Text>
                <Typography.Text>{me.max_devices}</Typography.Text>
              </div>
            </Card>
          </Col>
        </Row>
        <Card title="开始使用">
          <Row gutter={[16, 16]} type="flex">
            {STEPS.map((step, index) => (
              <Col xs={24} md={8} key={step.title}>
                <div style={{ display: "flex", gap: 12 }}>
                  <span
                    style={{
                      display: "grid",
                      placeItems: "center",
                      width: 28,
                      height: 28,
                      flex: "none",
                      borderRadius: "50%",
                      fontWeight: 600,
                      color: index < Math.min(me.devices, 2) ? "#fff" : "var(--semi-color-primary)",
                      background:
                        index < Math.min(me.devices, 2)
                          ? "var(--semi-color-primary)"
                          : "var(--semi-color-primary-light-default)",
                    }}
                  >
                    {index + 1}
                  </span>
                  <div>
                    <Typography.Text strong>{step.title}</Typography.Text>
                    <Typography.Paragraph type="tertiary" size="small" style={{ marginTop: 4 }}>
                      {step.text}
                    </Typography.Paragraph>
                  </div>
                </div>
              </Col>
            ))}
          </Row>
          <Button theme="light" style={{ marginTop: 16 }} onClick={() => navigate("/devices")}>
            查看设备
          </Button>
        </Card>
      </div>
    </>
  );
}
