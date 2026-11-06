import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Card, Input, Table, Tag, Typography } from "@douyinfe/semi-ui";
import { IconSearch } from "@douyinfe/semi-icons";
import { type AccountPage, type AccountRow, formatBytes, formatTime, get } from "../../api";

const PAGE = 20;

export function AdminAccounts() {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(1);
  const [result, setResult] = useState<AccountPage>();
  const load = useCallback(async () => {
    const params = new URLSearchParams({ q: query, offset: String((page - 1) * PAGE), limit: String(PAGE) });
    setResult(await get<AccountPage>(`/v1/admin/accounts?${params}`));
  }, [query, page]);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <>
      <Typography.Title heading={3} style={{ marginBottom: 16 }}>
        账号管理
      </Typography.Title>
      <Card>
        <Input
          prefix={<IconSearch />}
          placeholder="按邮箱搜索"
          showClear
          style={{ maxWidth: 320, marginBottom: 16 }}
          onChange={(value) => {
            setPage(1);
            setQuery(value);
          }}
        />
        <Table
          loading={result === undefined}
          dataSource={result?.accounts}
          rowKey="account_id"
          onRow={(row?: AccountRow) => ({
            onClick: () => row && navigate(`/admin/accounts/${row.account_id}`),
            style: { cursor: "pointer" },
          })}
          pagination={{
            currentPage: page,
            pageSize: PAGE,
            total: result?.total ?? 0,
            onPageChange: setPage,
          }}
          columns={[
            { title: "ID", dataIndex: "account_id", width: 80 },
            { title: "邮箱", dataIndex: "email" },
            {
              title: "状态",
              render: (_: unknown, row: AccountRow) => (
                <>
                  {row.suspended ? <Tag color="red">已封停</Tag> : <Tag color="green">正常</Tag>}
                  {row.role === "admin" && (
                    <Tag color="blue" style={{ marginLeft: 4 }}>
                      管理员
                    </Tag>
                  )}
                </>
              ),
            },
            { title: "套餐", dataIndex: "plan" },
            { title: "节点", dataIndex: "node_id" },
            { title: "设备", dataIndex: "devices" },
            { title: "本月", dataIndex: "month_upstream_bytes", render: (bytes: number) => formatBytes(bytes) },
            { title: "注册时间", dataIndex: "created_unix", render: (unix: number) => formatTime(unix) },
          ]}
        />
      </Card>
    </>
  );
}
