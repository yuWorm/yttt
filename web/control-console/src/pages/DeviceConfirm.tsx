import { useState } from "react";
import { Banner, Button, Descriptions, Form, Typography } from "@douyinfe/semi-ui";
import { IconTickCircle } from "@douyinfe/semi-icons";
import { ApiError, errorText, formatTime, post } from "../api";
import { useSession } from "../session";
import { AuthLayout } from "./Login";

interface DeviceRequest {
  label: string;
  requester_ip: string;
  created_unix: number;
}

/** A headless device, e.g. `yttt-server p2p login`, shows a short code; the
 * person confirms it here. A code in the link is only filled in: whoever
 * confirms still reviews what asked before allowing. */
export function DeviceConfirm() {
  const { me, signOut } = useSession();
  const [code, setCode] = useState(() => new URLSearchParams(window.location.search).get("code") ?? "");
  const [found, setFound] = useState<DeviceRequest>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState<"look" | "allow" | "deny">();
  const [done, setDone] = useState<"allowed" | "denied">();

  const call = async (step: "look" | "allow" | "deny") => {
    setBusy(step);
    setError(undefined);
    try {
      const body = step === "look" ? { user_code: code } : { user_code: code, allow: step === "allow" };
      const answer = await post<DeviceRequest>("/v1/oauth/device/confirm", body);
      if (step === "look") {
        setFound(answer);
      } else {
        setDone(step === "allow" ? "allowed" : "denied");
      }
    } catch (failure) {
      setError(
        failure instanceof ApiError && failure.status === 400
          ? "短码不正确或已过期，请核对设备上显示的短码"
          : failure instanceof ApiError && failure.status === 429
            ? "尝试次数过多，请稍后再试"
            : errorText(failure),
      );
      if (step !== "look") {
        setFound(undefined);
      }
    } finally {
      setBusy(undefined);
    }
  };

  if (done) {
    return (
      <AuthLayout title={done === "allowed" ? "已允许" : "已拒绝"} subtitle="可以关闭此页。">
        <div style={{ display: "flex", gap: 10, alignItems: "center", color: "var(--semi-color-success)" }}>
          <IconTickCircle size="large" />
          <span>{done === "allowed" ? "设备会在几秒内完成登录。" : "设备的登录已取消。"}</span>
        </div>
      </AuthLayout>
    );
  }
  return (
    <AuthLayout title="登录一台设备" subtitle={`输入设备上显示的短码，以 ${me?.email ?? ""} 的身份登录它。`}>
      {error && <Banner type="danger" description={error} closeIcon={null} style={{ marginBottom: 16 }} />}
      {!found ? (
        <Form labelPosition="top" onSubmit={() => void call("look")}>
          <Form.Input
            field="code"
            label="短码"
            size="large"
            placeholder="ABCD-EFGH"
            initValue={code}
            onChange={(value: string) => setCode(value)}
            style={{ fontFamily: "var(--semi-font-family-mono, monospace)", letterSpacing: 2, textTransform: "uppercase" }}
            rules={[{ required: true, message: "请输入短码" }]}
          />
          <Button htmlType="submit" theme="solid" size="large" block loading={busy === "look"} style={{ marginTop: 8 }}>
            继续
          </Button>
        </Form>
      ) : (
        <>
          <Descriptions
            align="left"
            data={[
              { key: "短码", value: <code>{code.toUpperCase()}</code> },
              { key: "设备", value: <span>{found.label} <Typography.Text type="tertiary" size="small">（由设备自报）</Typography.Text></span> },
              { key: "发起地址", value: found.requester_ip },
              { key: "发起时间", value: formatTime(found.created_unix) },
            ]}
          />
          <Banner
            type="warning"
            closeIcon={null}
            style={{ margin: "16px 0" }}
            description="核对短码与设备上显示的一致。只有当这台设备就在你身边、并且是你刚刚让它登录时才允许；如果短码或链接是别人发给你的，请拒绝。"
          />
          <Typography.Paragraph type="tertiary" size="small" style={{ marginBottom: 20 }}>
            允许后，这台设备可以登记设备密钥、使用账号的 relay 流量；不能修改账号、密码或登录方式，也不能管理控制台。
          </Typography.Paragraph>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
            <Button size="large" onClick={() => void call("deny")} loading={busy === "deny"} disabled={busy !== undefined}>
              拒绝
            </Button>
            <Button size="large" theme="solid" onClick={() => void call("allow")} loading={busy === "allow"} disabled={busy !== undefined}>
              允许
            </Button>
          </div>
        </>
      )}
      <Typography.Text type="tertiary" size="small" style={{ display: "block", marginTop: 16, textAlign: "center" }}>
        不是这个账号？<Typography.Text link size="small" onClick={() => void signOut()}>换一个账号登录</Typography.Text>
      </Typography.Text>
    </AuthLayout>
  );
}
