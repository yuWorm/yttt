import { useEffect, useState } from "react";
import { Card, Spin, Typography } from "@douyinfe/semi-ui";
import { type MonthUsage, formatBytes, get } from "../api";
import { useSession } from "../session";
import { UsageBars } from "../components/UsageBars";

export function Usage() {
  const { me } = useSession();
  const [usage, setUsage] = useState<MonthUsage[]>();
  useEffect(() => {
    void get<MonthUsage[]>("/v1/accounts/me/usage").then(setUsage);
  }, []);
  return (
    <>
      <Typography.Title heading={3} style={{ marginBottom: 16 }}>
        用量
      </Typography.Title>
      <Card title={`近 12 个月（每月额度 ${me ? formatBytes(me.monthly_bytes) : "—"}）`}>
        {usage ? <UsageBars usage={usage} quota={me?.monthly_bytes} /> : <Spin />}
        <Typography.Paragraph type="secondary" style={{ marginTop: 16 }}>
          用量按 relay 节点实际转发的上行字节计，直连的流量不计入。
        </Typography.Paragraph>
      </Card>
    </>
  );
}
