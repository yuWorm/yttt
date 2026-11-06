import { useCallback, useEffect, useState } from "react";
import {
  Avatar,
  Banner,
  Button,
  Card,
  Col,
  Descriptions,
  Form,
  List,
  Popconfirm,
  Row,
  Tag,
  Toast,
  Typography,
} from "@douyinfe/semi-ui";
import { IconExit, IconKey } from "@douyinfe/semi-icons";
import { ApiError, type SignInMethods, del, errorText, formatTime, get, post } from "../api";
import { useSession } from "../session";
import { PageHeader } from "../components/ui";
import {
  PROVIDER_NAMES,
  type Provider,
  ProviderIcon,
  providerError,
  startProvider,
} from "../components/Providers";

export function Account() {
  const { me, signOut } = useSession();
  const [methods, setMethods] = useState<SignInMethods>();
  const [methodsError, setMethodsError] = useState<string | undefined>(providerError);
  const load = useCallback(() => {
    get<SignInMethods>("/v1/accounts/me/sign-in")
      .then(setMethods)
      .catch((failure) => setMethodsError(errorText(failure)));
  }, []);
  useEffect(load, [load]);

  if (!me) {
    return null;
  }
  return (
    <>
      <PageHeader title="账号" />
      <Row gutter={[16, 16]} type="flex">
        <Col xs={24} lg={10}>
          <Card style={{ height: "100%" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 14, marginBottom: 20 }}>
              <Avatar size="large" color="light-blue">
                {me.email.charAt(0).toUpperCase()}
              </Avatar>
              <div style={{ minWidth: 0 }}>
                <Typography.Title heading={5} ellipsis={{ showTooltip: true }} style={{ margin: 0 }}>
                  {me.email}
                </Typography.Title>
                <div style={{ marginTop: 6, display: "flex", gap: 6 }}>
                  {me.suspended ? <Tag color="red">已封停</Tag> : <Tag color="green">正常</Tag>}
                  {me.role === "admin" && <Tag color="violet">管理员</Tag>}
                </div>
              </div>
            </div>
            <Descriptions
              data={[
                { key: "账号 ID", value: me.account_id },
                { key: "套餐", value: me.plan_name || me.plan },
                { key: "relay 节点", value: me.node_name || me.node_id },
              ]}
            />
            <Button icon={<IconExit />} style={{ marginTop: 20 }} onClick={() => void signOut()}>
              退出登录
            </Button>
          </Card>
        </Col>
        <Col xs={24} lg={14}>
          <SignInMethodsCard methods={methods} error={methodsError} setError={setMethodsError} reload={load} />
        </Col>
        <Col xs={24}>{methods && <PasswordCard methods={methods} reload={load} />}</Col>
      </Row>
    </>
  );
}

function SignInMethodsCard({
  methods,
  error,
  setError,
  reload,
}: {
  methods: SignInMethods | undefined;
  error: string | undefined;
  setError: (error: string | undefined) => void;
  reload: () => void;
}) {
  const [busy, setBusy] = useState<Provider>();
  const link = async (provider: Provider) => {
    setBusy(provider);
    setError(undefined);
    try {
      const { redirect } = await post<{ redirect: string }>(`/v1/auth/${provider}/link`, { next: "/account" });
      window.location.assign(redirect);
    } catch (failure) {
      setError(errorText(failure));
      setBusy(undefined);
    }
  };
  const unlink = async (provider: Provider) => {
    setError(undefined);
    try {
      await del(`/v1/accounts/me/identities/${provider}`);
      Toast.success(`已解除关联 ${PROVIDER_NAMES[provider]}，其他浏览器与设备上的登录已退出`);
      reload();
    } catch (failure) {
      setError(
        failure instanceof ApiError && failure.status === 409
          ? "这是唯一的登录方式：先设置密码或关联另一个账号，再解除关联"
          : errorText(failure),
      );
    }
  };
  const linked = new Map(methods?.identities.map((identity) => [identity.provider, identity]));
  const rows: Provider[] = (["github", "google"] as Provider[]).filter(
    (provider) => linked.has(provider) || methods?.providers.includes(provider),
  );
  return (
    <Card title="登录方式" style={{ height: "100%" }}>
      {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
      <List
        loading={!methods}
        dataSource={[
          { kind: "password" as const },
          ...rows.map((provider) => ({ kind: "provider" as const, provider })),
        ]}
        renderItem={(item) => {
          if (item.kind === "password") {
            return (
              <List.Item
                header={<Avatar size="small" color="grey"><IconKey /></Avatar>}
                main={
                  <div>
                    <div style={{ fontWeight: 600 }}>邮箱与密码</div>
                    <Typography.Text type="tertiary" size="small">
                      {methods?.has_password ? "已设置密码" : "未设置密码"}
                    </Typography.Text>
                  </div>
                }
              />
            );
          }
          const identity = linked.get(item.provider);
          const name = PROVIDER_NAMES[item.provider];
          return (
            <List.Item
              header={<Avatar size="small" color="grey"><ProviderIcon provider={item.provider} /></Avatar>}
              main={
                <div>
                  <div style={{ fontWeight: 600 }}>{name}</div>
                  <Typography.Text type="tertiary" size="small">
                    {identity ? `${identity.email} · ${formatTime(identity.created_unix)} 关联` : "未关联"}
                  </Typography.Text>
                </div>
              }
              extra={
                identity ? (
                  <Popconfirm
                    title={`解除关联 ${name}？`}
                    content="之后不能再用它登录，其他浏览器与设备上的登录会退出。"
                    onConfirm={() => void unlink(item.provider)}
                  >
                    <Button type="danger" theme="borderless">解除关联</Button>
                  </Popconfirm>
                ) : methods?.providers.includes(item.provider) ? (
                  <Button loading={busy === item.provider} onClick={() => void link(item.provider)}>
                    关联
                  </Button>
                ) : null
              }
            />
          );
        }}
      />
    </Card>
  );
}

function PasswordCard({ methods, reload }: { methods: SignInMethods; reload: () => void }) {
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const setting = !methods.has_password;
  const submit = async (values: { current?: string; new: string; confirm: string }) => {
    if (values.new !== values.confirm) {
      setError("两次输入的新密码不一致");
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      await post("/v1/accounts/me/password", setting ? { new: values.new } : { current: values.current, new: values.new });
      Toast.success(setting ? "密码已设置，现在也可以用邮箱和密码登录" : "密码已修改，其他设备上的登录已退出");
      reload();
    } catch (failure) {
      setError(
        failure instanceof ApiError && failure.status === 403
          ? setting
            ? "为了安全，请重新登录后再设置密码"
            : "当前密码不正确"
          : errorText(failure),
      );
    } finally {
      setBusy(false);
    }
  };
  // A first password needs a fresh sign-in, so a stolen session cannot add one.
  const reauthenticate = methods.identities[0]?.provider as Provider | undefined;
  return (
    <Card title={setting ? "设置密码" : "修改密码"}>
      {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
      {setting && !methods.may_set_password ? (
        <div>
          <Typography.Paragraph type="tertiary" style={{ marginBottom: 12 }}>
            设置密码后也可以用邮箱和密码登录。为了安全，设置前需要重新登录一次。
          </Typography.Paragraph>
          {reauthenticate && (
            <Button icon={<ProviderIcon provider={reauthenticate} />} onClick={() => startProvider(reauthenticate, "/account")}>
              用 {PROVIDER_NAMES[reauthenticate]} 重新登录
            </Button>
          )}
        </div>
      ) : (
        <Form onSubmit={submit} labelPosition="top" style={{ maxWidth: 420 }}>
          {!setting && (
            <Form.Input
              field="current"
              label="当前密码"
              mode="password"
              autoComplete="current-password"
              rules={[{ required: true, message: "请输入当前密码" }]}
            />
          )}
          <Form.Input
            field="new"
            label="新密码"
            mode="password"
            autoComplete="new-password"
            rules={[{ required: true, min: 8, message: "至少 8 个字符" }]}
          />
          <Form.Input
            field="confirm"
            label="确认新密码"
            mode="password"
            autoComplete="new-password"
            rules={[{ required: true, message: "请再输入一次" }]}
          />
          <Button htmlType="submit" theme="solid" loading={busy}>
            {setting ? "设置密码" : "修改密码"}
          </Button>
          <Typography.Text type="tertiary" size="small" style={{ display: "block", marginTop: 12 }}>
            {setting ? "设置后，邮箱与密码可以作为另一种登录方式。" : "修改后，其他浏览器与设备上的登录会退出。"}
          </Typography.Text>
        </Form>
      )}
    </Card>
  );
}
