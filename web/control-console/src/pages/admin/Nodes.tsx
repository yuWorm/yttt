import { useCallback, useEffect, useState } from "react";
import {
  Banner,
  Button,
  Card,
  Dropdown,
  Form,
  Modal,
  Select,
  Space,
  Table,
  Toast,
  Typography,
} from "@douyinfe/semi-ui";
import { IconMore, IconPlus } from "@douyinfe/semi-icons";
import {
  type DeployCommand,
  type NodeView,
  ApiError,
  del,
  formatTime,
  get,
  patch,
  post,
} from "../../api";
import { NodeStatus } from "./status";

function failed(failure: unknown) {
  Toast.error(failure instanceof ApiError ? failure.message : String(failure));
}

/** Shows a deploy command once, with what the machine needs. */
function CommandModal({ deploy, onClose }: { deploy?: DeployCommand; onClose: () => void }) {
  return (
    <Modal
      title={deploy ? `部署节点 ${deploy.node.id}` : ""}
      visible={deploy !== undefined}
      onCancel={onClose}
      footer={
        <Button theme="solid" onClick={onClose}>
          我已复制
        </Button>
      }
      width={720}
      maskClosable={false}
    >
      {deploy && (
        <Space vertical align="start" style={{ width: "100%" }}>
          <Typography.Text>在要部署的 Linux 服务器（systemd，x86_64 或 aarch64）上执行：</Typography.Text>
          <Typography.Paragraph
            copyable={{ content: deploy.command, onCopy: () => Toast.success("已复制部署命令") }}
            style={{
              fontFamily: "monospace",
              fontSize: 12,
              background: "var(--semi-color-fill-0)",
              padding: 12,
              borderRadius: 6,
              wordBreak: "break-all",
              width: "100%",
            }}
          >
            {deploy.command}
          </Typography.Paragraph>
          <Banner
            type="warning"
            closeIcon={null}
            description={`命令只显示这一次，单次有效，${formatTime(deploy.expires_unix)} 前使用。关闭后如需再次部署，请“重新生成命令”。`}
          />
          <Typography.Text type="secondary">
            脚本会安装 yttt-relay、向控制面登记、装成 systemd 服务并启动，完成后节点在一分钟内显示“在线”。不带域名时需放行
            TCP 3341；带域名时会自动申请 Let's Encrypt 证书，需放行 TCP 443、80 和 UDP 7842，且域名已解析到该服务器。
          </Typography.Text>
        </Space>
      )}
    </Modal>
  );
}

export function AdminNodes() {
  const [nodes, setNodes] = useState<NodeView[]>();
  const [creating, setCreating] = useState(false);
  const [deploy, setDeploy] = useState<DeployCommand>();
  const [regenerating, setRegenerating] = useState<NodeView>();
  const [editing, setEditing] = useState<NodeView>();
  const [moving, setMoving] = useState<NodeView>();
  const [target, setTarget] = useState<string>();
  const load = useCallback(async () => {
    setNodes(await get<NodeView[]>("/v1/admin/nodes"));
  }, []);
  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 15000);
    return () => window.clearInterval(timer);
  }, [load]);

  const create = async (values: { id: string; name?: string; domain?: string }) => {
    try {
      setDeploy(await post<DeployCommand>("/v1/admin/nodes", values));
      setCreating(false);
      await load();
    } catch (failure) {
      failed(failure);
    }
  };
  const regenerate = async (values: { domain?: string }) => {
    if (!regenerating) {
      return;
    }
    try {
      setDeploy(await post<DeployCommand>(`/v1/admin/nodes/${regenerating.id}/enrollment`, values));
      setRegenerating(undefined);
    } catch (failure) {
      failed(failure);
    }
  };
  const edit = async (values: { name: string; url: string }) => {
    if (!editing) {
      return;
    }
    try {
      await patch(`/v1/admin/nodes/${editing.id}`, {
        name: values.name,
        ...(values.url && values.url !== editing.url ? { url: values.url } : {}),
      });
      setEditing(undefined);
      await load();
    } catch (failure) {
      failed(failure);
    }
  };
  const setDisabled = (node: NodeView, disabled: boolean) =>
    Modal.confirm({
      title: disabled ? `停用节点 ${node.id}？` : `启用节点 ${node.id}？`,
      content: disabled
        ? "节点会在下次上报时断开其上所有账号的设备，并不再为它们签发 ticket。请先把账号迁到其他节点。"
        : "节点恢复为账号服务。",
      onOk: async () => {
        try {
          await patch(`/v1/admin/nodes/${node.id}`, { disabled });
          await load();
        } catch (failure) {
          failed(failure);
        }
      },
    });
  const remove = (node: NodeView) =>
    Modal.confirm({
      title: `删除节点 ${node.id}？`,
      content: "节点上不能还有账号。删除后它的上报会被拒绝，记得在服务器上执行 sudo yttt-relay-install --uninstall。",
      onOk: async () => {
        try {
          await del(`/v1/admin/nodes/${node.id}`);
          await load();
        } catch (failure) {
          failed(failure);
        }
      },
    });
  const move = async () => {
    if (!moving || !target) {
      return;
    }
    try {
      const moved = await post<number>(`/v1/admin/nodes/${moving.id}/accounts/move/${target}`);
      Toast.success(`已迁移 ${moved} 个账号`);
      setMoving(undefined);
      setTarget(undefined);
      await load();
    } catch (failure) {
      failed(failure);
    }
  };

  return (
    <>
      <Space style={{ marginBottom: 16, width: "100%", justifyContent: "space-between" }}>
        <Typography.Title heading={3}>节点</Typography.Title>
        <Button icon={<IconPlus />} theme="solid" onClick={() => setCreating(true)}>
          新建节点
        </Button>
      </Space>
      <Card>
        <Table
          loading={nodes === undefined}
          dataSource={nodes}
          rowKey="id"
          pagination={false}
          columns={[
            {
              title: "节点",
              render: (_: unknown, node: NodeView) => (
                <Space vertical align="start" spacing={2}>
                  <Typography.Text strong>{node.id}</Typography.Text>
                  <Typography.Text type="secondary" size="small">
                    {node.name}
                  </Typography.Text>
                </Space>
              ),
            },
            { title: "状态", render: (_: unknown, node: NodeView) => <NodeStatus node={node} /> },
            {
              title: "地址",
              dataIndex: "url",
              render: (url: string) => (url ? <Typography.Text copyable>{url}</Typography.Text> : "—"),
            },
            { title: "在线设备", dataIndex: "connected_devices", render: (count: number | null) => count ?? "—" },
            { title: "账号", dataIndex: "accounts" },
            { title: "版本", dataIndex: "version", render: (version: string | null) => version ?? "—" },
            { title: "最后上报", dataIndex: "last_report_unix", render: (unix: number | null) => formatTime(unix) },
            {
              title: "",
              render: (_: unknown, node: NodeView) => (
                <Dropdown
                  trigger="click"
                  position="bottomRight"
                  render={
                    <Dropdown.Menu>
                      <Dropdown.Item onClick={() => setRegenerating(node)}>重新生成部署命令</Dropdown.Item>
                      <Dropdown.Item onClick={() => setEditing(node)}>编辑名称与地址</Dropdown.Item>
                      <Dropdown.Item onClick={() => setMoving(node)} disabled={node.accounts === 0}>
                        迁移账号到其他节点
                      </Dropdown.Item>
                      {node.status === "disabled" ? (
                        <Dropdown.Item onClick={() => setDisabled(node, false)}>启用</Dropdown.Item>
                      ) : (
                        <Dropdown.Item onClick={() => setDisabled(node, true)} disabled={node.status === "pending"}>
                          停用
                        </Dropdown.Item>
                      )}
                      <Dropdown.Item type="danger" onClick={() => remove(node)} disabled={node.accounts > 0}>
                        删除
                      </Dropdown.Item>
                    </Dropdown.Menu>
                  }
                >
                  <Button icon={<IconMore />} theme="borderless" />
                </Dropdown>
              ),
            },
          ]}
        />
      </Card>

      <Modal title="新建节点" visible={creating} onCancel={() => setCreating(false)} footer={null}>
        <Form onSubmit={create} labelPosition="top">
          <Form.Input
            field="id"
            label="节点 ID"
            placeholder="tokyo-1"
            rules={[
              { required: true, message: "请输入节点 ID" },
              { pattern: /^[a-z0-9-]{1,32}$/, message: "小写字母、数字与连字符，至多 32 个" },
            ]}
          />
          <Form.Input field="name" label="显示名" placeholder="东京 1 号" />
          <Form.Input
            field="domain"
            label="域名（可选）"
            placeholder="relay.example.com"
            extraText="填写后自动申请 Let's Encrypt 证书并走 HTTPS，打洞成功率更高；不填则用服务器公网 IP 的 http 地址。"
          />
          <Button htmlType="submit" theme="solid" block>
            生成部署命令
          </Button>
        </Form>
      </Modal>

      <Modal
        title={`重新生成 ${regenerating?.id ?? ""} 的部署命令`}
        visible={regenerating !== undefined}
        onCancel={() => setRegenerating(undefined)}
        footer={null}
      >
        <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
          之前未用的部署命令立即作废。正在运行的节点不受影响，直到新命令在某台机器上完成登记，届时节点令牌随之轮换。
        </Typography.Paragraph>
        <Form onSubmit={regenerate} labelPosition="top">
          <Form.Input field="domain" label="域名（可选）" placeholder="relay.example.com" />
          <Button htmlType="submit" theme="solid" block>
            生成
          </Button>
        </Form>
      </Modal>

      <Modal
        title={`编辑节点 ${editing?.id ?? ""}`}
        visible={editing !== undefined}
        onCancel={() => setEditing(undefined)}
        footer={null}
      >
        {editing && (
          <Form onSubmit={edit} labelPosition="top" initValues={{ name: editing.name, url: editing.url }}>
            <Form.Input field="name" label="显示名" />
            <Form.Input
              field="url"
              label="地址"
              extraText="改地址后，设备要到下次续签 ticket 才换到新地址，Host 也需要重新开启 P2P。尽量保持不变，推荐使用域名。"
            />
            <Button htmlType="submit" theme="solid" block>
              保存
            </Button>
          </Form>
        )}
      </Modal>

      <Modal
        title={`迁移 ${moving?.id ?? ""} 上的 ${moving?.accounts ?? 0} 个账号`}
        visible={moving !== undefined}
        onCancel={() => setMoving(undefined)}
        onOk={() => void move()}
        okButtonProps={{ disabled: !target }}
      >
        <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
          账号的设备在续签 ticket 后改用新节点；在此之前它们仍连在旧节点上。
        </Typography.Paragraph>
        <Select
          placeholder="目标节点"
          style={{ width: "100%" }}
          value={target}
          onChange={(value) => setTarget(String(value))}
          optionList={(nodes ?? [])
            .filter((node) => node.id !== moving?.id && (node.status === "online" || node.status === "offline"))
            .map((node) => ({ value: node.id, label: `${node.id}（${node.name}）` }))}
        />
      </Modal>

      <CommandModal deploy={deploy} onClose={() => setDeploy(undefined)} />
    </>
  );
}
