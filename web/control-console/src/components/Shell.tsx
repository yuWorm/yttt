import type { ReactNode } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { Button, Layout, Nav, Tag, Typography } from "@douyinfe/semi-ui";
import {
  IconHistogram,
  IconHome,
  IconKey,
  IconList,
  IconMonitorStroked,
  IconServer,
  IconUser,
  IconUserGroup,
  IconCreditCard,
} from "@douyinfe/semi-icons";
import { useSession } from "../session";

const USER_ITEMS = [
  { itemKey: "/", text: "概览", icon: <IconHome /> },
  { itemKey: "/devices", text: "设备", icon: <IconMonitorStroked /> },
  { itemKey: "/usage", text: "用量", icon: <IconHistogram /> },
  { itemKey: "/account", text: "账号", icon: <IconUser /> },
];

const ADMIN_ITEMS = [
  { itemKey: "/admin", text: "总览", icon: <IconHistogram /> },
  { itemKey: "/admin/accounts", text: "账号管理", icon: <IconUserGroup /> },
  { itemKey: "/admin/nodes", text: "节点", icon: <IconServer /> },
  { itemKey: "/admin/plans", text: "套餐", icon: <IconCreditCard /> },
  { itemKey: "/admin/audit", text: "审计日志", icon: <IconList /> },
];

export function Shell({ children }: { children: ReactNode }) {
  const { me, signOut } = useSession();
  const navigate = useNavigate();
  const location = useLocation();
  const admin = me?.role === "admin";
  const items = admin
    ? [
        ...USER_ITEMS,
        { itemKey: "admin-group", text: "管理", icon: <IconKey />, items: ADMIN_ITEMS },
      ]
    : USER_ITEMS;
  const selected =
    [...ADMIN_ITEMS, ...USER_ITEMS]
      .map((item) => item.itemKey)
      .filter((key) => key === location.pathname || (key !== "/" && location.pathname.startsWith(`${key}/`)))
      .sort((a, b) => b.length - a.length)[0] ?? "/";
  return (
    <Layout style={{ minHeight: "100vh" }}>
      <Layout.Sider>
        <Nav
          style={{ height: "100%" }}
          items={items}
          selectedKeys={[selected]}
          defaultOpenKeys={admin ? ["admin-group"] : []}
          onSelect={({ itemKey }) => navigate(String(itemKey))}
          header={{ text: "yttt relay" }}
          footer={{ collapseButton: true }}
        />
      </Layout.Sider>
      <Layout>
        <Layout.Header
          style={{
            display: "flex",
            justifyContent: "flex-end",
            alignItems: "center",
            gap: 12,
            padding: "12px 24px",
            borderBottom: "1px solid var(--semi-color-border)",
          }}
        >
          {admin && <Tag color="blue">管理员</Tag>}
          <Typography.Text type="secondary">{me?.email}</Typography.Text>
          <Button theme="borderless" onClick={() => void signOut()}>
            退出登录
          </Button>
        </Layout.Header>
        <Layout.Content style={{ padding: 24, maxWidth: 1200 }}>{children}</Layout.Content>
      </Layout>
    </Layout>
  );
}
