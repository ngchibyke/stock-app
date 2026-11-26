import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api";
import { useAction, useDebounced } from "../hooks";
import { Button } from "../ui/Button";
import { Drawer } from "../ui/Drawer";
import { ConfirmDialog } from "../ui/Modal";
import { DataTable, LoadMore, SearchBox } from "../ui/Table";
import { Badge, Card, CopyChip, EmptyState, ErrorNote, Segmented, StatusBadge } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonDetail, SkeletonTable } from "../ui/Skeleton";

interface StoreRow {
  id: string;
  name: string;
  slug: string;
  owner_id: string;
  status: "pending" | "active" | "suspended";
  created_at: string;
}
interface StorePage {
  items: StoreRow[];
  next_cursor: string | null;
}
interface StoreDetail extends StoreRow {
  suspended: boolean;
  suspended_reason: string | null;
  suspended_at: string | null;
  payout_account_status: string | null;
  product_count: number;
  open_order_count: number;
  branches: BranchRow[];
  team: MemberRow[];
}
interface BranchRow {
  id: string;
  name: string;
  area_council: string | null;
  closed_at: string | null;
  collect_enabled: boolean;
}
interface MemberRow {
  id: string;
  user_id: string;
  display_name: string;
  role: "manager" | "staff";
  branch_id: string | null;
  removed_at: string | null;
}

const day = (iso: string) => new Date(iso).toLocaleDateString("en-NG", { day: "numeric", month: "short", year: "numeric" });

export function Stores() {
  const [text, setText] = useState("");
  const q = useDebounced(text);
  const [status, setStatus] = useState<"" | "pending" | "active" | "suspended">("");
  const [open, setOpen] = useState<string | null>(null);
  const list = useInfiniteQuery({
    queryKey: ["stores", q, status],
    initialPageParam: "",
    queryFn: ({ pageParam }) => api.get<StorePage>("/admin/stores", { q, status, before: pageParam, limit: 25 }),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const rows = list.data?.pages.flatMap((p) => p.items) ?? [];

  return (
    <>
      <PageHeader title="Stores" subtitle="Open a store to see its standing, or suspend it." />
      <div className="toolbar">
        <SearchBox value={text} onChange={setText} placeholder="Search a store name" />
        <Segmented id="sstatus" label="Status" value={status} onChange={setStatus} options={[{ value: "", label: "All" }, { value: "active", label: "Active" }, { value: "pending", label: "Pending" }, { value: "suspended", label: "Suspended" }]} />
      </div>
      <Card>
        {list.isPending && <SkeletonTable rows={6} cols={4} label="Loading stores" />}
        {list.isError && <ErrorNote error={list.error} onRetry={() => void list.refetch()} />}
        {list.data && rows.length === 0 && ((text || status) ? <EmptyState icon="store" title="No stores match" action={<Button icon="refresh" onClick={() => { setText(""); setStatus(""); }}>Clear filters</Button>} /> : <EmptyState icon="store" title="No stores match" />)}
        {rows.length > 0 && (
          <DataTable
            label="Stores"
            rows={rows}
            rowKey={(s) => s.id}
            onRowClick={(s) => setOpen(s.id)}
            columns={[
              { key: "name", header: "Store", cell: (s) => <strong>{s.name}</strong> },
              { key: "slug", header: "Link", cell: (s) => <span className="mono muted">{s.slug}</span> },
              { key: "status", header: "Status", cell: (s) => <StatusBadge status={s.status} /> },
              { key: "since", header: "Opened", cell: (s) => day(s.created_at) },
            ]}
          />
        )}
        <LoadMore hasMore={!!list.hasNextPage} loading={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()} />
      </Card>
      <StoreDrawer id={open} onClose={() => setOpen(null)} />
    </>
  );
}

function StoreDrawer({ id, onClose }: { id: string | null; onClose: () => void }) {
  const detail = useQuery({ queryKey: ["store", id], queryFn: () => api.get<StoreDetail>(`/admin/stores/${id}`), enabled: id !== null });
  const [suspending, setSuspending] = useState(false);
  const refresh = [["store", id], ["stores"]];
  const suspend = useAction({ run: (reason: string) => api.post(`/admin/stores/${id}/suspend`, { reason }), success: "Store suspended.", invalidate: refresh });
  const unsuspend = useAction({ run: () => api.post(`/admin/stores/${id}/unsuspend`), success: "Store is back.", invalidate: refresh });
  const s = detail.data;

  return (
    <>
      <Drawer open={id !== null} title={s?.name ?? "Store"} onClose={onClose}>
        {detail.isPending && <SkeletonDetail rows={5} label="Loading the store" />}
        {detail.isError && <ErrorNote error={detail.error} />}
        {s && (
          <>
            <dl className="kv">
              <dt>Status</dt>
              <dd><StatusBadge status={s.status} /></dd>
              <dt>Owner</dt>
              <dd><CopyChip value={s.owner_id} label={s.owner_id.slice(0, 8)} /></dd>
              <dt>Payout account</dt>
              <dd>{s.payout_account_status ? <StatusBadge status={s.payout_account_status} /> : <span className="muted">None</span>}</dd>
              <dt>Products</dt>
              <dd>{s.product_count}</dd>
              <dt>Orders in progress</dt>
              <dd>{s.open_order_count}</dd>
              {s.suspended && (
                <>
                  <dt>Suspended</dt>
                  <dd>{s.suspended_at ? day(s.suspended_at) : ""}: {s.suspended_reason}</dd>
                </>
              )}
            </dl>
            <h3 style={{ margin: "16px 0 8px" }}>Branches</h3>
            {s.branches.length === 0 ? (
              <EmptyState icon="store" title="No branches" />
            ) : (
              <DataTable
                label="Branches"
                rows={s.branches}
                rowKey={(b) => b.id}
                columns={[
                  { key: "name", header: "Branch", cell: (b) => <strong>{b.name}</strong> },
                  { key: "area", header: "Area", cell: (b) => b.area_council ?? <span className="muted">None</span> },
                  { key: "collect", header: "Collect", cell: (b) => (b.collect_enabled ? "On" : <span className="muted">Off</span>) },
                  { key: "state", header: "Status", cell: (b) => (b.closed_at ? <Badge tone="neutral">Closed {day(b.closed_at)}</Badge> : <Badge tone="good">Open</Badge>) },
                ]}
              />
            )}
            <h3 style={{ margin: "16px 0 8px" }}>Team</h3>
            {s.team.length === 0 ? (
              <EmptyState icon="users" title="No team members" />
            ) : (
              <DataTable
                label="Team"
                rows={s.team}
                rowKey={(m) => m.id}
                columns={[
                  { key: "name", header: "Name", cell: (m) => <strong>{m.display_name}</strong> },
                  { key: "role", header: "Role", cell: (m) => <Badge tone="info">{m.role}</Badge> },
                  { key: "branch", header: "Branch", cell: (m) => (m.branch_id ? (s.branches.find((b) => b.id === m.branch_id)?.name ?? "Unknown") : <span className="muted">All</span>) },
                  { key: "state", header: "Status", cell: (m) => (m.removed_at ? <Badge tone="neutral">Removed</Badge> : <Badge tone="good">Active</Badge>) },
                ]}
              />
            )}
            {s.suspended ? (
              <Button icon="undo" variant="primary" loading={unsuspend.busy} done={unsuspend.done} onClick={() => void unsuspend.run(undefined).catch(() => {})}>
                Unsuspend
              </Button>
            ) : (
              <Button variant="danger" icon="ban" onClick={() => setSuspending(true)}>
                Suspend store
              </Button>
            )}
            <p className="hint">
              A suspended store disappears from browse and takes no new orders. Orders already paid keep going, and payouts are not frozen.
            </p>
          </>
        )}
      </Drawer>
      <ConfirmDialog
        open={suspending}
        title="Suspend this store?"
        summary={<>It is hidden from buyers and cannot take new orders until you unsuspend it. {s && s.open_order_count > 0 ? `${s.open_order_count} order(s) in progress carry on.` : ""}</>}
        confirmLabel="Suspend"
        danger
        noteLabel="Reason"
        onClose={() => setSuspending(false)}
        onConfirm={async (reason) => {
          await suspend.run(reason);
        }}
      />
    </>
  );
}
