import type { ReactNode } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { Avatar, Breadcrumb, Button, Dropdown, Layout, Nav, Tag, Tooltip, Typography } from "@douyinfe/semi-ui";
import {
  IconCreditCard,
  IconExit,
  IconHistogram,
  IconHome,
  IconList,
  IconMoon,
  IconMonitorStroked,
  IconServer,
  IconSetting,
  IconSun,
  IconUser,
  IconUserGroup,
  IconPieChartStroked,
} from "@douyinfe/semi-icons";
import { useSession } from "../session";
import { useTheme } from "../theme";
import { BrandMark } from "./Brand";

const USER_ITEMS = [
  { itemKey: "/", text: "概览", icon: <IconHome /> },
  { itemKey: "/devices", text: "设备", icon: <IconMonitorStroked /> },
  { itemKey: "/usage", text: "用量", icon: <IconHistogram /> },
  { itemKey: "/account", text: "账号", icon: <IconUser /> },
];

const ADMIN_ITEMS = [
  { itemKey: "/admin", text: "运营总览", icon: <IconPieChartStroked /> },
  { itemKey: "/admin/accounts", text: "账号管理", icon: <IconUserGroup /> },
  { itemKey: "/admin/nodes", text: "节点", icon: <IconServer /> },
  { itemKey: "/admin/plans", text: "套餐", icon: <IconCreditCard /> },
  { itemKey: "/admin/settings", text: "设置", icon: <IconSetting /> },
  { itemKey: "/admin/audit", text: "审计日志", icon: <IconList /> },
];

const ALL_ITEMS = [...USER_ITEMS, ...ADMIN_ITEMS];

function selectedKey(path: string): string {
  return (
    ALL_ITEMS.map((item) => item.itemKey)
      .filter((key) => key === path || (key !== "/" && path.startsWith(`${key}/`)))
      .sort((a, b) => b.length - a.length)[0] ?? "/"
  );
}

export function Shell({ children }: { children: ReactNode }) {
  const { me, signOut } = useSession();
  const navigate = useNavigate();
  const location = useLocation();
  const [mode, toggleMode] = useTheme();
  const admin = me?.role === "admin";
  const selected = selectedKey(location.pathname);
  const current = ALL_ITEMS.find((item) => item.itemKey === selected);
  const inAdmin = selected.startsWith("/admin");
  const items = admin
    ? [
        ...USER_ITEMS,
        { itemKey: "admin-group", text: "管理", icon: <IconSetting />, items: ADMIN_ITEMS },
      ]
    : USER_ITEMS;
  const initial = (me?.email ?? "?").charAt(0).toUpperCase();
  return (
    <Layout className="app-layout">
      <Layout.Sider className="app-sider">
        <Nav
          style={{ height: "100%", background: "transparent", border: "none" }}
          items={items}
          selectedKeys={[selected]}
          defaultOpenKeys={admin ? ["admin-group"] : []}
          onSelect={({ itemKey }) => {
            if (itemKey !== "admin-group") {
              navigate(String(itemKey));
            }
          }}
          header={{ logo: <BrandMark />, text: "yttt relay" }}
          footer={{ collapseButton: true }}
        />
      </Layout.Sider>
      <Layout>
        <Layout.Header className="app-header">
          <Breadcrumb>
            <Breadcrumb.Item onClick={() => navigate(inAdmin ? "/admin" : "/")}>
              {inAdmin ? "管理" : "我的"}
            </Breadcrumb.Item>
            <Breadcrumb.Item>{current?.text}</Breadcrumb.Item>
            {location.pathname !== selected && <Breadcrumb.Item>详情</Breadcrumb.Item>}
          </Breadcrumb>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <Tooltip content={mode === "dark" ? "切换到亮色" : "切换到暗色"}>
              <Button
                theme="borderless"
                type="tertiary"
                icon={mode === "dark" ? <IconSun /> : <IconMoon />}
                aria-label="切换主题"
                onClick={toggleMode}
              />
            </Tooltip>
            <Dropdown
              position="bottomRight"
              render={
                <Dropdown.Menu>
                  <Dropdown.Title>{me?.email}</Dropdown.Title>
                  <Dropdown.Item icon={<IconUser />} onClick={() => navigate("/account")}>
                    账号与密码
                  </Dropdown.Item>
                  <Dropdown.Divider />
                  <Dropdown.Item icon={<IconExit />} type="danger" onClick={() => void signOut()}>
                    退出登录
                  </Dropdown.Item>
                </Dropdown.Menu>
              }
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer", padding: "4px 6px" }}>
                <Avatar size="small" color="light-blue">
                  {initial}
                </Avatar>
                <Typography.Text style={{ maxWidth: 200 }} ellipsis={{ showTooltip: true }}>
                  {me?.email}
                </Typography.Text>
                {admin && (
                  <Tag color="violet" size="small">
                    管理员
                  </Tag>
                )}
              </div>
            </Dropdown>
          </div>
        </Layout.Header>
        <Layout.Content className="app-content">
          <div className="app-page">{children}</div>
        </Layout.Content>
      </Layout>
    </Layout>
  );
}
