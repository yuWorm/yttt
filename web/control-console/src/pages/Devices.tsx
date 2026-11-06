import { useCallback, useEffect, useState } from "react";
import { Badge, Button, Card, Modal, Popconfirm, Select, Table, Tag, Toast, Typography } from "@douyinfe/semi-ui";
import { IconDelete, IconMonitorStroked, IconServer } from "@douyinfe/semi-icons";
import { type DeviceEntry, type NodeChoice, del, errorText, formatTime, get, put } from "../api";
import { useSession } from "../session";
import { EmptyState, PageHeader } from "../components/ui";

export function Devices() {
  const { me, refresh } = useSession();
  const [devices, setDevices] = useState<DeviceEntry[]>();
  const [nodes, setNodes] = useState<NodeChoice[]>([]);
  const load = useCallback(async () => {
    const [list, choices] = await Promise.all([
      get<{ devices: DeviceEntry[] }>("/v1/devices"),
      get<NodeChoice[]>("/v1/nodes"),
    ]);
    setDevices(list.devices);
    setNodes(choices);
  }, []);
  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 30000);
    return () => window.clearInterval(timer);
  }, [load]);
  const remove = async (device: DeviceEntry) => {
    try {
      await del(`/v1/devices/${device.endpoint_id}`);
      Toast.success("已移除");
      await Promise.all([load(), refresh()]);
    } catch (failure) {
      Toast.error(errorText(failure));
    }
  };
  const nodeName = (id: string) => {
    const node = nodes.find((choice) => choice.id === id);
    return node ? (node.region ? `${node.name}（${node.region}）` : node.name) : id;
  };
  const moveHost = (device: DeviceEntry, nodeId: string) =>
    Modal.confirm({
      title: `把 ${device.label} 移到 ${nodeName(nodeId)}？`,
      content:
        "连接这台 Host 的设备会一起移过去，当前的远程会话会断开并自动重连。套餐只允许一个节点时，账号里的所有设备都会一起移动。",
      onOk: async () => {
        try {
          await put(`/v1/devices/${device.endpoint_id}/node`, { node_id: nodeId });
          Toast.success("已切换，约 15 秒内生效");
          await Promise.all([load(), refresh()]);
        } catch (failure) {
          Toast.error(errorText(failure));
        }
      },
    });
  const labelOf = (key?: string) => devices?.find((device) => device.endpoint_id === key)?.label ?? key?.slice(0, 8);
  return (
    <>
      <PageHeader
        title="设备"
        description={me ? `已登记 ${me.devices} / ${me.max_devices}` : undefined}
      />
      <Card bodyStyle={{ padding: devices?.length ? 0 : undefined }}>
        {devices && devices.length === 0 ? (
          <EmptyState
            title="还没有设备"
            description="在 yttt 应用里登录此账号后，设备会自动登记到这里。Host 占一个名额，每台设备对每台 Host 各占一个。"
          />
        ) : (
          <Table
            loading={devices === undefined}
            dataSource={devices}
            rowKey="endpoint_id"
            pagination={false}
            columns={[
              {
                title: "名称",
                dataIndex: "label",
                render: (label: string, device: DeviceEntry) => (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                    <Badge dot type={device.online ? "success" : "tertiary"} />
                    {device.role === "host" ? (
                      <IconServer style={{ color: "var(--semi-color-text-2)" }} />
                    ) : (
                      <IconMonitorStroked style={{ color: "var(--semi-color-text-2)" }} />
                    )}
                    {label}
                  </span>
                ),
              },
              {
                title: "类型",
                dataIndex: "role",
                render: (role: DeviceEntry["role"], device: DeviceEntry) =>
                  role === "host" ? (
                    <Tag color="blue">Host</Tag>
                  ) : role === "client" ? (
                    <span>
                      <Tag>设备</Tag>
                      <Typography.Text type="tertiary" size="small" style={{ marginLeft: 6 }}>
                        连 {labelOf(device.follows_host)}
                      </Typography.Text>
                    </span>
                  ) : (
                    <Tag color="grey">旧版本</Tag>
                  ),
              },
              {
                title: "节点",
                dataIndex: "node_id",
                render: (nodeId: string, device: DeviceEntry) =>
                  device.role === "host" ? (
                    <Select
                      size="small"
                      value={nodeId}
                      style={{ minWidth: 180 }}
                      onChange={(value) => value !== nodeId && moveHost(device, String(value))}
                      optionList={nodes.map((node) => ({
                        value: node.id,
                        disabled: node.full && node.id !== nodeId,
                        label:
                          nodeName(node.id) +
                          (node.full ? " · 已满" : node.load_percent !== undefined ? ` · ${node.load_percent}%` : ""),
                      }))}
                    />
                  ) : (
                    <Typography.Text type="tertiary">
                      {nodeName(nodeId)}
                      {device.role === "client" && "（跟随 Host）"}
                    </Typography.Text>
                  ),
              },
              {
                title: "状态",
                dataIndex: "online",
                render: (online: boolean) => (online ? "在线" : "离线"),
              },
              { title: "登记时间", dataIndex: "registered_unix", render: (unix: number) => formatTime(unix) },
              {
                title: "",
                align: "right",
                render: (_: unknown, device: DeviceEntry) => (
                  <Popconfirm
                    title="移除此设备？"
                    content={
                      device.role === "host"
                        ? "连接这台 Host 的设备也会一起移除，它们将无法再经 relay 连接，但不影响 Host 上的配对。"
                        : "它将无法再经 relay 连接，但不影响 Host 上的配对。"
                    }
                    onConfirm={() => remove(device)}
                  >
                    <Button type="danger" theme="borderless" icon={<IconDelete />}>
                      移除
                    </Button>
                  </Popconfirm>
                ),
              },
            ]}
          />
        )}
      </Card>
    </>
  );
}
