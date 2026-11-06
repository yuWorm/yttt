import { Banner, Card, Descriptions, Progress, Space, Typography } from "@douyinfe/semi-ui";
import { formatBytes } from "../api";
import { useSession } from "../session";

export function Overview() {
  const { me } = useSession();
  if (!me) {
    return null;
  }
  const percent = me.monthly_bytes > 0 ? Math.min((me.month_upstream_bytes / me.monthly_bytes) * 100, 100) : 0;
  return (
    <Space vertical align="start" spacing="loose" style={{ width: "100%" }}>
      <Typography.Title heading={3}>概览</Typography.Title>
      {me.suspended && <Banner type="danger" description="此账号已被封停，relay 暂不可用。如有疑问请联系管理员。" closeIcon={null} />}
      <Card title="套餐与节点" style={{ width: "100%" }}>
        <Descriptions
          data={[
            { key: "套餐", value: me.plan },
            { key: "relay 节点", value: me.node_id },
            { key: "relay 地址", value: me.relay_url || "—" },
            { key: "设备", value: `${me.devices} / ${me.max_devices}` },
          ]}
        />
      </Card>
      <Card title="本月用量" style={{ width: "100%" }}>
        <Progress percent={percent} showInfo stroke={percent >= 100 ? "var(--semi-color-danger)" : undefined} />
        <Typography.Text type="secondary">
          {formatBytes(me.month_upstream_bytes)} / {formatBytes(me.monthly_bytes)}
          {percent >= 100 && "，已超出额度，relay 已降速"}
        </Typography.Text>
      </Card>
      <Card title="如何使用" style={{ width: "100%" }}>
        <Typography.Paragraph>
          在 Host 和它的设备上，用同一账号在 yttt 的“远程访问此电脑 → relay 账号”中登录。直连不通时，设备之间会经此账号的 relay 节点互通。
        </Typography.Paragraph>
        <Typography.Paragraph type="secondary">
          relay 只转发端到端加密的流量，看不到内容，也不能决定谁可以连接你的电脑。
        </Typography.Paragraph>
      </Card>
    </Space>
  );
}
