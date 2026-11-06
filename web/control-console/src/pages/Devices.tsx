import { useCallback, useEffect, useState } from "react";
import { Button, Card, Popconfirm, Table, Toast, Typography } from "@douyinfe/semi-ui";
import { type DeviceEntry, del, formatTime, get } from "../api";
import { useSession } from "../session";

export function Devices() {
  const { refresh } = useSession();
  const [devices, setDevices] = useState<DeviceEntry[]>();
  const load = useCallback(async () => {
    setDevices((await get<{ devices: DeviceEntry[] }>("/v1/devices")).devices);
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
  const remove = async (device: DeviceEntry) => {
    await del(`/v1/devices/${device.endpoint_id}`);
    Toast.success("已移除");
    await Promise.all([load(), refresh()]);
  };
  return (
    <>
      <Typography.Title heading={3} style={{ marginBottom: 16 }}>
        设备
      </Typography.Title>
      <Card>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 16 }}>
          设备在 yttt 应用里登录此账号后自动登记，每把设备密钥占一个名额：Host 一个，每台设备对每台 Host 各一个。移除后该密钥不能再经 relay 连接，但不影响 Host 上的配对。
        </Typography.Paragraph>
        <Table
          loading={devices === undefined}
          dataSource={devices}
          rowKey="endpoint_id"
          pagination={false}
          columns={[
            { title: "名称", dataIndex: "label" },
            {
              title: "设备密钥",
              dataIndex: "endpoint_id",
              render: (key: string) => <Typography.Text copyable={{ content: key }}>{key.slice(0, 16)}…</Typography.Text>,
            },
            { title: "登记时间", dataIndex: "registered_unix", render: (unix: number) => formatTime(unix) },
            {
              title: "",
              render: (_: unknown, device: DeviceEntry) => (
                <Popconfirm title="移除此设备？" content="它将无法再经 relay 连接。" onConfirm={() => remove(device)}>
                  <Button type="danger" theme="borderless">
                    移除
                  </Button>
                </Popconfirm>
              ),
            },
          ]}
        />
      </Card>
    </>
  );
}
