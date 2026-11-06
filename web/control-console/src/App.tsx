import { lazy, Suspense } from "react";
import { Navigate, Route, Routes } from "react-router-dom";
import { Spin } from "@douyinfe/semi-ui";
import { Shell } from "./components/Shell";
import { useSession } from "./session";
import { Login } from "./pages/Login";
import { Signup } from "./pages/Signup";

// Pages load on first visit; account holders never download the admin ones.
const page = <T extends Record<string, React.ComponentType>>(load: () => Promise<T>, name: keyof T) =>
  lazy(() => load().then((module) => ({ default: module[name] })));
const Overview = page(() => import("./pages/Overview"), "Overview");
const Devices = page(() => import("./pages/Devices"), "Devices");
const Usage = page(() => import("./pages/Usage"), "Usage");
const Account = page(() => import("./pages/Account"), "Account");
const AdminOverview = page(() => import("./pages/admin/Overview"), "AdminOverview");
const AdminAccounts = page(() => import("./pages/admin/Accounts"), "AdminAccounts");
const AdminAccountDetail = page(() => import("./pages/admin/AccountDetail"), "AdminAccountDetail");
const AdminNodes = page(() => import("./pages/admin/Nodes"), "AdminNodes");
const AdminPlans = page(() => import("./pages/admin/Plans"), "AdminPlans");
const AdminAudit = page(() => import("./pages/admin/Audit"), "AdminAudit");
const AdminSettings = page(() => import("./pages/admin/Settings"), "AdminSettings");

function Loading() {
  return (
    <div style={{ display: "grid", placeItems: "center", minHeight: 240 }}>
      <Spin size="large" />
    </div>
  );
}

export function App() {
  const { me } = useSession();
  if (me === undefined) {
    return (
      <div style={{ display: "grid", placeItems: "center", height: "100vh" }}>
        <Spin size="large" />
      </div>
    );
  }
  if (me === null) {
    return (
      <Routes>
        <Route path="/signup" element={<Signup />} />
        <Route path="*" element={<Login />} />
      </Routes>
    );
  }
  // The server checks the role on every admin request; this only hides pages.
  const admin = me.role === "admin";
  return (
    <Shell>
      <Suspense fallback={<Loading />}>
        <Routes>
        <Route path="/" element={<Overview />} />
        <Route path="/devices" element={<Devices />} />
        <Route path="/usage" element={<Usage />} />
        <Route path="/account" element={<Account />} />
        {admin && (
          <>
            <Route path="/admin" element={<AdminOverview />} />
            <Route path="/admin/accounts" element={<AdminAccounts />} />
            <Route path="/admin/accounts/:id" element={<AdminAccountDetail />} />
            <Route path="/admin/nodes" element={<AdminNodes />} />
            <Route path="/admin/plans" element={<AdminPlans />} />
            <Route path="/admin/settings" element={<AdminSettings />} />
            <Route path="/admin/audit" element={<AdminAudit />} />
          </>
        )}
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
      </Suspense>
    </Shell>
  );
}
