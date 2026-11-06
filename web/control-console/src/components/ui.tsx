// Building blocks every page shares: its header, statistic cards, empty states
// and the daily traffic chart.
import { useState, type ReactNode } from "react";
import { Card, Empty, Typography } from "@douyinfe/semi-ui";
import {
  IllustrationNoContent,
  IllustrationNoContentDark,
  IllustrationNoResult,
  IllustrationNoResultDark,
} from "@douyinfe/semi-illustrations";
import { type DayUsage, formatBytes, formatDay } from "../api";

export function PageHeader({
  title,
  description,
  extra,
}: {
  title: ReactNode;
  description?: ReactNode;
  extra?: ReactNode;
}) {
  return (
    <div className="page-header">
      <div>
        <Typography.Title heading={3}>{title}</Typography.Title>
        {description && (
          <Typography.Text type="tertiary" style={{ display: "block", marginTop: 6 }}>
            {description}
          </Typography.Text>
        )}
      </div>
      {extra && <div>{extra}</div>}
    </div>
  );
}

export type Tone = "blue" | "green" | "orange" | "violet" | "red" | "cyan";

const TONES: Record<Tone, string> = {
  blue: "var(--semi-color-primary)",
  green: "var(--semi-color-success)",
  orange: "var(--semi-color-warning)",
  violet: "var(--semi-color-tertiary)",
  red: "var(--semi-color-danger)",
  cyan: "var(--semi-color-info)",
};

const TONE_BACKGROUNDS: Record<Tone, string> = {
  blue: "var(--semi-color-primary-light-default)",
  green: "var(--semi-color-success-light-default)",
  orange: "var(--semi-color-warning-light-default)",
  violet: "var(--semi-color-tertiary-light-default)",
  red: "var(--semi-color-danger-light-default)",
  cyan: "var(--semi-color-info-light-default)",
};

export function StatCard({
  icon,
  tone = "blue",
  title,
  value,
  footer,
}: {
  icon: ReactNode;
  tone?: Tone;
  title: ReactNode;
  value: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <Card className="stat" shadows="hover" bodyStyle={{ padding: 20 }}>
      <div className="stat-icon" style={{ color: TONES[tone], background: TONE_BACKGROUNDS[tone] }}>
        {icon}
      </div>
      <div style={{ minWidth: 0, flex: 1 }}>
        <Typography.Text type="tertiary" size="small">
          {title}
        </Typography.Text>
        <div className="stat-value">{value}</div>
        {footer && (
          <Typography.Text type="tertiary" size="small" style={{ display: "block" }}>
            {footer}
          </Typography.Text>
        )}
      </div>
    </Card>
  );
}

/** An empty list, with what to do next. */
export function EmptyState({
  title,
  description,
  search = false,
  children,
}: {
  title: ReactNode;
  description?: ReactNode;
  search?: boolean;
  children?: ReactNode;
}) {
  const size = { width: 120, height: 120 };
  return (
    <Empty
      style={{ padding: "32px 0" }}
      image={search ? <IllustrationNoResult style={size} /> : <IllustrationNoContent style={size} />}
      darkModeImage={search ? <IllustrationNoResultDark style={size} /> : <IllustrationNoContentDark style={size} />}
      title={title}
      description={description}
    >
      {children}
    </Empty>
  );
}

const CHART_HEIGHT = 180;
const AXIS = 22;

/** Daily traffic as bars, oldest on the left; hovering shows the day. */
export function DailyChart({ days, height = CHART_HEIGHT }: { days: DayUsage[]; height?: number }) {
  const [active, setActive] = useState<number>();
  if (days.length === 0) {
    return null;
  }
  const width = 720;
  const plot = height - AXIS;
  const peak = Math.max(...days.map((day) => day.upstream_bytes));
  const top = Math.max(peak, 1);
  const slot = width / days.length;
  const bar = Math.max(slot * 0.62, 2);
  const ticks = [0, 0.5, 1].map((fraction) => ({ fraction, label: formatBytes(top * fraction) }));
  const labelEvery = Math.ceil(days.length / 8);
  // The last day always gets a label; its neighbours give way to it.
  const labelled = (index: number) =>
    index === days.length - 1 || (index % labelEvery === 0 && days.length - 1 - index >= labelEvery / 2 + 1);
  const hovered = active === undefined ? undefined : days[active];
  return (
    <div className="chart" onMouseLeave={() => setActive(undefined)}>
      <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" style={{ height }} role="img">
        {ticks.map(({ fraction }) => (
          <line
            key={fraction}
            x1={0}
            x2={width}
            y1={plot - plot * fraction + 0.5}
            y2={plot - plot * fraction + 0.5}
            stroke="var(--semi-color-border)"
            strokeDasharray={fraction === 0 ? undefined : "3 4"}
            vectorEffect="non-scaling-stroke"
          />
        ))}
        {days.map((day, index) => {
          const barHeight = day.upstream_bytes > 0 ? Math.max((day.upstream_bytes / top) * (plot - 8), 2) : 0;
          return (
            <g key={day.day} onMouseEnter={() => setActive(index)}>
              <rect x={index * slot} y={0} width={slot} height={plot} fill="transparent" />
              <rect
                className={`chart-bar${active === index ? " active" : ""}`}
                x={index * slot + (slot - bar) / 2}
                y={plot - barHeight}
                width={bar}
                height={barHeight}
                rx={Math.min(3, bar / 2)}
              />
            </g>
          );
        })}
      </svg>
      <div style={{ position: "relative", height: AXIS - 4, marginTop: -AXIS + 4 }}>
        {days.map((day, index) =>
          labelled(index) ? (
            <span
              key={day.day}
              style={{
                position: "absolute",
                left: `${((index + 0.5) / days.length) * 100}%`,
                transform: "translateX(-50%)",
                fontSize: 11,
                whiteSpace: "nowrap",
                color: "var(--semi-color-text-2)",
              }}
            >
              {formatDay(day.day)}
            </span>
          ) : null,
        )}
      </div>
      {peak > 0 ? (
        <span style={{ position: "absolute", top: -2, left: 0, fontSize: 11, color: "var(--semi-color-text-2)" }}>
          {ticks[2].label}
        </span>
      ) : (
        <span
          style={{
            position: "absolute",
            top: plot / 2 - 10,
            left: 0,
            right: 0,
            textAlign: "center",
            fontSize: 13,
            color: "var(--semi-color-text-2)",
          }}
        >
          这段时间没有经 relay 转发的流量
        </span>
      )}
      {hovered && active !== undefined && (
        <div className="chart-tip" style={{ left: `${((active + 0.5) / days.length) * 100}%` }}>
          <div style={{ color: "var(--semi-color-text-2)" }}>{formatDay(hovered.day)}</div>
          <div style={{ fontWeight: 600 }}>{formatBytes(hovered.upstream_bytes)}</div>
        </div>
      )}
    </div>
  );
}

/** The total of a run of days. */
export function sumDays(days: DayUsage[]): number {
  return days.reduce((total, day) => total + day.upstream_bytes, 0);
}
