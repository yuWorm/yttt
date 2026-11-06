import { useCallback, useEffect, useState } from "react";
import { Banner, Button, Card, Form, Popconfirm, Spin, Tag, Toast, Typography } from "@douyinfe/semi-ui";
import { type NodeView, type PlanView, type Settings, del, errorText, get, patch, put } from "../../api";
import { PageHeader } from "../../components/ui";
import { PROVIDER_NAMES, type Provider, ProviderIcon } from "../../components/Providers";

interface ProviderSettings {
  provider: Provider;
  callback_url: string | null;
  client_id: string;
  has_secret: boolean;
  enabled: boolean;
  from_file: boolean;
}

interface OAuthSettings {
  available: boolean;
  public_url: string | null;
  providers: ProviderSettings[];
}

/** GitHub and Google sign-in: client ID and a write-only secret. */
function SignInProviders() {
  const [settings, setSettings] = useState<OAuthSettings>();
  const [busy, setBusy] = useState<Provider>();
  const load = useCallback(() => {
    get<OAuthSettings>("/v1/admin/oauth").then(setSettings, (failure) => Toast.error(errorText(failure)));
  }, []);
  useEffect(load, [load]);
  const save = async (provider: Provider, values: { client_id: string; client_secret?: string; enabled: boolean }) => {
    setBusy(provider);
    try {
      await put(`/v1/admin/oauth/${provider}`, values);
      Toast.success("已保存");
      load();
    } catch (failure) {
      Toast.error(errorText(failure));
    } finally {
      setBusy(undefined);
    }
  };
  const remove = async (provider: Provider) => {
    try {
      await del(`/v1/admin/oauth/${provider}`);
      load();
    } catch (failure) {
      Toast.error(errorText(failure));
    }
  };
  if (!settings) {
    return null;
  }
  return (
    <Card title="第三方登录" style={{ maxWidth: 640, marginTop: 16 }}>
      {!settings.available && (
        <Banner
          type="warning"
          closeIcon={null}
          fullMode={false}
          style={{ marginBottom: 16 }}
          description="control.toml 里没有 secrets_key，控制台无法加密保存 client secret。用安装脚本重新配置，或手动加上 secrets_key（32 字节 hex）后重启控制面。"
        />
      )}
      {!settings.public_url && (
        <Banner
          type="warning"
          closeIcon={null}
          fullMode={false}
          style={{ marginBottom: 16 }}
          description="未设置 public_url，第三方登录无法回调到控制台。"
        />
      )}
      {settings.providers.map((row) => (
        <div key={row.provider} style={{ paddingBottom: 20, marginBottom: 20, borderBottom: "1px solid var(--semi-color-border)" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
            <ProviderIcon provider={row.provider} />
            <Typography.Text strong>{PROVIDER_NAMES[row.provider]}</Typography.Text>
            {row.enabled ? <Tag color="green">已启用</Tag> : <Tag>未启用</Tag>}
            {row.from_file && <Tag color="grey">来自 control.toml</Tag>}
          </div>
          {row.callback_url && (
            <Typography.Paragraph type="tertiary" size="small" style={{ marginBottom: 8 }}>
              回调地址：
              <Typography.Text copyable size="small" className="mono">
                {row.callback_url}
              </Typography.Text>
            </Typography.Paragraph>
          )}
          <Form
            labelPosition="left"
            labelWidth={120}
            initValues={{ client_id: row.client_id, enabled: row.enabled }}
            onSubmit={(values) => void save(row.provider, values)}
            disabled={!settings.available}
          >
            <Form.Input field="client_id" label="Client ID" style={{ width: 360 }} rules={[{ required: true, message: "请输入 Client ID" }]} />
            <Form.Input
              field="client_secret"
              label="Client secret"
              mode="password"
              style={{ width: 360 }}
              placeholder={row.has_secret ? "已保存，留空则不修改" : ""}
              autoComplete="new-password"
            />
            <Form.Switch field="enabled" label="启用" />
            <div style={{ marginLeft: 120, display: "flex", gap: 8 }}>
              <Button htmlType="submit" theme="solid" loading={busy === row.provider}>
                保存
              </Button>
              {row.has_secret && !row.from_file && (
                <Popconfirm title={`移除 ${PROVIDER_NAMES[row.provider]} 登录？`} content="已关联的账号保留关联，重新设置后可继续使用。" onConfirm={() => void remove(row.provider)}>
                  <Button type="danger" theme="borderless">
                    移除
                  </Button>
                </Popconfirm>
              )}
            </div>
          </Form>
        </div>
      ))}
      <Typography.Text type="tertiary" size="small">
        GitHub 用 OAuth App（不是 GitHub App）；Google 需要 https 的 public_url。secret 加密后保存，保存后不再显示。
      </Typography.Text>
    </Card>
  );
}

export function AdminSettings() {
  const [settings, setSettings] = useState<Settings>();
  const [plans, setPlans] = useState<PlanView[]>([]);
  const [nodes, setNodes] = useState<NodeView[]>([]);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void get<Settings>("/v1/admin/settings").then(setSettings);
    void get<PlanView[]>("/v1/admin/plans").then(setPlans);
    void get<NodeView[]>("/v1/admin/nodes").then(setNodes);
  }, []);
  const submit = async (values: Settings) => {
    setBusy(true);
    try {
      await patch("/v1/admin/settings", values);
      setSettings(values);
      Toast.success("已保存");
    } catch (failure) {
      Toast.error(errorText(failure));
    } finally {
      setBusy(false);
    }
  };
  if (!settings) {
    return <Spin size="large" />;
  }
  const node = nodes.find((candidate) => candidate.id === settings.signup_node);
  return (
    <>
      <PageHeader title="设置" description="这里的设置保存在数据库，立即生效，不需要重启控制面。" />
      <Card title="注册" style={{ maxWidth: 640 }}>
        {settings.signup_enabled && node && node.status !== "online" && (
          <Banner
            type="warning"
            fullMode={false}
            closeIcon={null}
            description={`注册节点 ${node.id} 当前${node.status === "pending" ? "尚未部署" : "不在线"}，新用户暂时无法注册。`}
            style={{ marginBottom: 16 }}
          />
        )}
        <Form<Settings> onSubmit={submit} labelPosition="left" labelWidth={120} initValues={settings}>
          <Form.Switch field="signup_enabled" label="开放注册" />
          <Form.Select
            field="signup_plan"
            label="新账号套餐"
            style={{ width: 280 }}
            optionList={plans
              .filter((plan) => !plan.archived)
              .map((plan) => ({ value: plan.id, label: plan.name || plan.id }))}
          />
          <Form.Select
            field="signup_node"
            label="新账号节点"
            style={{ width: 280 }}
            optionList={nodes
              .filter((candidate) => candidate.status !== "disabled")
              .map((candidate) => ({ value: candidate.id, label: `${candidate.name}（${candidate.id}）` }))}
          />
          <Typography.Text type="tertiary" size="small" style={{ display: "block", margin: "4px 0 16px 120px" }}>
            关闭注册后，仍可在“账号管理”里为用户开户。
          </Typography.Text>
          <Button htmlType="submit" theme="solid" loading={busy} style={{ marginLeft: 120 }}>
            保存
          </Button>
        </Form>
      </Card>
      <SignInProviders />
    </>
  );
}
