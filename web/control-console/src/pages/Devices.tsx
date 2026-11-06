import { useCallback, useEffect, useState } from "react";
import { Button, Card, Popconfirm, Table, Toast, Typography } from "@douyinfe/semi-ui";
import { IconDelete, IconMonitorStroked } from "@douyinfe/semi-icons";
import { type DeviceEntry, del, errorText, formatTime, get } from "../api";
import { useSession } from "../session";
import { EmptyState, PageHeader } from "../components/ui";

export function Devices() {
  const { me, refresh } = useSession();
  const [devices, setDevices] = useState<DeviceEntry[]>();
  const load = useCallback(async () => {
    setDevices((await get<{ devices: DeviceEntry[] }>("/v1/devices")).devices);
  }, []);
  useEffect(() => {
    void load();
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
                render: (label: string) => (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                    <IconMonitorStroked style={{ color: "var(--semi-color-text-2)" }} />
                    {label}
                  </span>
                ),
              },
              {
                title: "设备密钥",
                dataIndex: "endpoint_id",
                render: (key: string) => (
                  <Typography.Text className="mono" copyable={{ content: key }}>
                    {key.slice(0, 16)}…
                  </Typography.Text>
                ),
              },
              { title: "登记时间", dataIndex: "registered_unix", render: (unix: number) => formatTime(unix) },
              {
                title: "",
                align: "right",
                render: (_: unknown, device: DeviceEntry) => (
                  <Popconfirm
                    title="移除此设备？"
                    content="它将无法再经 relay 连接，但不影响 Host 上的配对。"
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
