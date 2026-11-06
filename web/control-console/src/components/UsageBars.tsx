import { Empty, Typography } from "@douyinfe/semi-ui";
import { type MonthUsage, formatBytes, formatMonth } from "../api";

/** Monthly usage as horizontal bars, newest first; quota marks the limit. */
export function UsageBars({ usage, quota }: { usage: MonthUsage[]; quota?: number }) {
  if (usage.length === 0) {
    return <Empty description="暂无用量" />;
  }
  const top = Math.max(quota ?? 0, ...usage.map((month) => month.upstream_bytes), 1);
  return (
    <div style={{ display: "grid", gap: 8 }}>
      {usage.map((month) => {
        const over = quota !== undefined && month.upstream_bytes >= quota;
        return (
          <div
            key={month.month}
            style={{ display: "grid", gridTemplateColumns: "72px 1fr 96px", alignItems: "center", gap: 12 }}
          >
            <Typography.Text type="secondary">{formatMonth(month.month)}</Typography.Text>
            <div style={{ background: "var(--semi-color-fill-0)", borderRadius: 4, height: 12 }}>
              <div
                style={{
                  width: `${Math.max((month.upstream_bytes / top) * 100, month.upstream_bytes > 0 ? 1 : 0)}%`,
                  height: "100%",
                  borderRadius: 4,
                  background: over ? "var(--semi-color-danger)" : "var(--semi-color-primary)",
                }}
              />
            </div>
            <Typography.Text style={{ textAlign: "right" }}>{formatBytes(month.upstream_bytes)}</Typography.Text>
          </div>
        );
      })}
    </div>
  );
}
