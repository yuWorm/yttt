import { Tag, Tooltip } from "@douyinfe/semi-ui";
import type { NodeView } from "../../api";

const STATUS: Record<NodeView["status"], { color: "green" | "grey" | "orange" | "red"; text: string }> = {
  online: { color: "green", text: "在线" },
  offline: { color: "red", text: "离线" },
  pending: { color: "orange", text: "待登记" },
  disabled: { color: "grey", text: "已停用" },
};

export function NodeStatus({ node }: { node: NodeView }) {
  const status = STATUS[node.status];
  const expired =
    node.status === "pending" &&
    (node.enrollment_expires_unix === null || node.enrollment_expires_unix * 1000 < Date.now());
  return (
    <>
      <Tag color={status.color}>{expired ? "令牌已过期" : status.text}</Tag>
      {node.status === "online" && node.reachable === false && (
        <Tooltip content="节点在上报，但控制面访问不到它的地址；检查防火墙或节点地址">
          <Tag color="yellow" style={{ marginLeft: 4 }}>
            地址不可达
          </Tag>
        </Tooltip>
      )}
    </>
  );
}
