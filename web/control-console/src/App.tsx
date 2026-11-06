import { Navigate, Route, Routes } from "react-router-dom";
import { Spin } from "@douyinfe/semi-ui";
import { Shell } from "./components/Shell";
import { useSession } from "./session";
import { Login } from "./pages/Login";
import { Signup } from "./pages/Signup";
import { Overview } from "./pages/Overview";
import { Devices } from "./pages/Devices";
import { Usage } from "./pages/Usage";
import { Account } from "./pages/Account";
import { AdminOverview } from "./pages/admin/Overview";
import { AdminAccounts } from "./pages/admin/Accounts";
import { AdminAccountDetail } from "./pages/admin/AccountDetail";
import { AdminNodes } from "./pages/admin/Nodes";
import { AdminPlans } from "./pages/admin/Plans";
import { AdminAudit } from "./pages/admin/Audit";

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
            <Route path="/admin/audit" element={<AdminAudit />} />
          </>
        )}
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </Shell>
  );
}
