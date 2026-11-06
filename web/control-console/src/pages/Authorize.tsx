import { useEffect, useMemo, useState } from "react";
import { Banner, Button, Descriptions, Spin, Typography } from "@douyinfe/semi-ui";
import { IconTickCircle } from "@douyinfe/semi-icons";
import { errorText, post } from "../api";
import { useSession } from "../session";
import { AuthLayout } from "./Login";

interface Answer {
  redirect?: string;
  label: string;
}

/** The yttt app asks to sign in: it opened this page from its "sign in with
 * the browser" button and waits on its own machine for the answer. */
export function Authorize() {
  const { me, signOut } = useSession();
  const request = useMemo(() => {
    const params = new URLSearchParams(window.location.search);
    return {
      redirect_uri: params.get("redirect_uri") ?? "",
      state: params.get("state") ?? "",
      code_challenge: params.get("code_challenge") ?? "",
      code_challenge_method: params.get("code_challenge_method") ?? "",
      label: params.get("label") ?? "",
    };
  }, []);
  const [label, setLabel] = useState<string>();
  const [invalid, setInvalid] = useState<string>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState<"allow" | "deny">();
  const [left, setLeft] = useState<"allowed" | "denied">();

  useEffect(() => {
    // An invalid request is shown, never followed.
    post<Answer>("/v1/oauth/authorize", request)
      .then((answer) => setLabel(answer.label))
      .catch((failure) => setInvalid(errorText(failure)));
  }, [request]);

  const decide = async (allow: boolean) => {
    setBusy(allow ? "allow" : "deny");
    setError(undefined);
    try {
      const answer = await post<Answer>("/v1/oauth/authorize", { ...request, allow });
      setLeft(allow ? "allowed" : "denied");
      // A top-level navigation back to the app on this machine.
      window.location.assign(answer.redirect!);
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setBusy(undefined);
    }
  };

  if (left) {
    return (
      <AuthLayout title={left === "allowed" ? "已允许" : "已拒绝"} subtitle="可以关闭此页，回到 yttt 应用。">
        <div style={{ display: "flex", gap: 10, alignItems: "center", color: "var(--semi-color-success)" }}>
          <IconTickCircle size="large" />
          <span>{left === "allowed" ? "应用会在几秒内完成登录。" : "应用的登录已取消。"}</span>
        </div>
      </AuthLayout>
    );
  }
  return (
    <AuthLayout title="允许 yttt 应用登录？" subtitle="一台设备上的 yttt 应用请求使用你的 relay 账号。">
      {invalid ? (
        <Banner type="danger" closeIcon={null} title="这个登录请求无效" description={`${invalid}。请回到 yttt 应用重新点“在浏览器中登录”。`} />
      ) : label === undefined ? (
        <div style={{ display: "grid", placeItems: "center", minHeight: 160 }}>
          <Spin />
        </div>
      ) : (
        <>
          <Descriptions
            align="left"
            data={[
              { key: "设备", value: <span>{label} <Typography.Text type="tertiary" size="small">（由设备自报）</Typography.Text></span> },
              { key: "账号", value: me?.email },
            ]}
          />
          <Banner
            type="warning"
            closeIcon={null}
            style={{ margin: "16px 0" }}
            description="只有当你刚刚在自己的设备上点了“在浏览器中登录”时才允许。如果这个页面来自别人发给你的链接，请拒绝。"
          />
          <Typography.Paragraph type="tertiary" size="small" style={{ marginBottom: 20 }}>
            允许后，这台设备可以登记设备密钥、使用账号的 relay 流量；不能修改账号、密码或登录方式，也不能管理控制台。
          </Typography.Paragraph>
          {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
            <Button size="large" onClick={() => void decide(false)} loading={busy === "deny"} disabled={busy !== undefined}>
              拒绝
            </Button>
            <Button size="large" theme="solid" onClick={() => void decide(true)} loading={busy === "allow"} disabled={busy !== undefined}>
              允许
            </Button>
          </div>
          <Typography.Text type="tertiary" size="small" style={{ display: "block", marginTop: 16, textAlign: "center" }}>
            不是这个账号？<Typography.Text link size="small" onClick={() => void signOut()}>换一个账号登录</Typography.Text>
          </Typography.Text>
        </>
      )}
    </AuthLayout>
  );
}
