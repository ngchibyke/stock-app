import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api";
import { useDebounced, useAction } from "../hooks";
import { Button } from "../ui/Button";
import { Drawer } from "../ui/Drawer";
import { ConfirmDialog, Modal } from "../ui/Modal";
import { DataTable, LoadMore, SearchBox } from "../ui/Table";
import { Badge, Card, EmptyState, ErrorNote, Segmented, StatusBadge, Tabs } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonDetail, SkeletonTable } from "../ui/Skeleton";

interface UserRow {
  id: string;
  display_name: string;
  role: "user" | "support" | "owner";
  is_seller: boolean;
  banned: boolean;
  muted: boolean;
  deleted: boolean;
  created_at: string;
}
interface UserPage {
  items: UserRow[];
  next_cursor: string | null;
}
interface UserDetail extends Omit<UserRow, "muted"> {
  phone: string;
  banned_at: string | null;
  muted_until: string | null;
  strike_count: number;
  store_ids: string[];
  orders_as_buyer: number;
}
interface StaffRow {
  id: string;
  display_name: string;
  role: string;
  created_at: string;
}

const day = (iso: string) => new Date(iso).toLocaleDateString("en-NG", { day: "numeric", month: "short", year: "numeric" });

export function Users() {
  const [tab, setTab] = useState<"people" | "staff">("people");
  return (
    <>
      <PageHeader title="Users" subtitle="Find a person, see their standing, and act on their account." />
      <div style={{ marginTop: 14 }}>
        <Tabs id="users" value={tab} onChange={setTab} tabs={[{ value: "people", label: "People" }, { value: "staff", label: "Staff" }]} />
      </div>
      {tab === "people" ? <People /> : <Staff />}
    </>
  );
}

function People() {
  const [text, setText] = useState("");
  const q = useDebounced(text);
  const [state, setState] = useState<"" | "active" | "banned" | "muted">("");
  const [role, setRole] = useState<"" | "user" | "support" | "owner">("");
  const [open, setOpen] = useState<string | null>(null);

  const list = useInfiniteQuery({
    queryKey: ["users", q, state, role],
    initialPageParam: "",
    queryFn: ({ pageParam }) => api.get<UserPage>("/admin/users", { q, state, role, before: pageParam, limit: 25 }),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const rows = list.data?.pages.flatMap((p) => p.items) ?? [];

  return (
    <>
      <div className="toolbar">
        <SearchBox value={text} onChange={setText} placeholder="Search a phone number or a name" />
        <Segmented id="ustate" label="State" value={state} onChange={setState} options={[{ value: "", label: "All" }, { value: "active", label: "Active" }, { value: "muted", label: "Muted" }, { value: "banned", label: "Banned" }]} />
        <Segmented id="urole" label="Role" value={role} onChange={setRole} options={[{ value: "", label: "Any role" }, { value: "user", label: "People" }, { value: "support", label: "Support" }, { value: "owner", label: "Owner" }]} />
      </div>
      <Card>
        {list.isPending && <SkeletonTable rows={6} cols={5} label="Loading users" />}
        {list.isError && <ErrorNote error={list.error} onRetry={() => void list.refetch()} />}
        {list.data && rows.length === 0 && <EmptyState icon="users" title="Nobody matches" hint="Try a different name, number or filter." />}
        {rows.length > 0 && (
          <DataTable
            label="Users"
            rows={rows}
            rowKey={(u) => u.id}
            onRowClick={(u) => setOpen(u.id)}
            columns={[
              { key: "name", header: "Name", cell: (u) => <strong>{u.display_name}</strong> },
              { key: "role", header: "Role", cell: (u) => (u.role === "user" ? (u.is_seller ? <Badge tone="info">seller</Badge> : <span className="muted">shopper</span>) : <Badge tone="info">{u.role}</Badge>) },
              { key: "state", header: "State", cell: (u) => (u.deleted ? <StatusBadge status="deleted" /> : u.banned ? <StatusBadge status="banned" /> : u.muted ? <StatusBadge status="muted" /> : <StatusBadge status="active" />) },
              { key: "since", header: "Joined", cell: (u) => day(u.created_at) },
            ]}
          />
        )}
        <LoadMore hasMore={!!list.hasNextPage} loading={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()} />
      </Card>
      <UserDrawer id={open} onClose={() => setOpen(null)} />
    </>
  );
}

function UserDrawer({ id, onClose }: { id: string | null; onClose: () => void }) {
  const detail = useQuery({ queryKey: ["user", id], queryFn: () => api.get<UserDetail>(`/admin/users/${id}`), enabled: id !== null });
  const [ban, setBan] = useState(false);
  const refresh = [["user", id], ["users"]];
  const unban = useAction({ run: () => api.post(`/admin/moderation/users/${id}/unban`), success: "Ban lifted.", invalidate: refresh });
  const unmute = useAction({ run: () => api.post(`/admin/moderation/users/${id}/unmute`), success: "Unmuted.", invalidate: refresh });
  const banNow = useAction({ run: (reason: string) => api.post(`/admin/moderation/users/${id}/ban`, { reason }), success: "User banned.", invalidate: refresh });
  const u = detail.data;
  const muted = !!u?.muted_until && Date.parse(u.muted_until) > Date.now();

  return (
    <>
      <Drawer open={id !== null} title={u?.display_name ?? "User"} onClose={onClose}>
        {detail.isPending && <SkeletonDetail rows={6} label="Loading the user" />}
        {detail.isError && <ErrorNote error={detail.error} />}
        {u && (
          <>
            <dl className="kv">
              <dt>Phone</dt>
              <dd className="mono">{u.phone}</dd>
              <dt>Role</dt>
              <dd style={{ textTransform: "capitalize" }}>{u.role}</dd>
              <dt>Joined</dt>
              <dd>{day(u.created_at)}</dd>
              <dt>Orders as buyer</dt>
              <dd>{u.orders_as_buyer}</dd>
              <dt>Strikes</dt>
              <dd>{u.strike_count}</dd>
              <dt>Stores</dt>
              <dd>{u.store_ids.length === 0 ? "None" : u.store_ids.length}</dd>
              <dt>Standing</dt>
              <dd className="row">
                {u.banned && <StatusBadge status="banned" />}
                {muted && <StatusBadge status="muted" />}
                {!u.banned && !muted && <StatusBadge status="active" />}
              </dd>
            </dl>
            <div className="row">
              {muted && (
                <Button icon="undo" loading={unmute.busy} done={unmute.done} onClick={() => void unmute.run(undefined).catch(() => {})}>
                  Unmute
                </Button>
              )}
              {u.banned ? (
                <Button icon="undo" loading={unban.busy} done={unban.done} onClick={() => void unban.run(undefined).catch(() => {})}>
                  Lift ban
                </Button>
              ) : (
                u.role === "user" && (
                  <Button variant="danger" icon="ban" onClick={() => setBan(true)}>
                    Ban
                  </Button>
                )
              )}
            </div>
            <p className="hint">Opening this record is written to the audit log.</p>
          </>
        )}
      </Drawer>
      <ConfirmDialog
        open={ban}
        title="Ban this user?"
        summary={<>They are signed out of chat and cannot post. You can lift the ban later.</>}
        confirmLabel="Ban"
        danger
        noteLabel="Reason (the user sees this)"
        onClose={() => setBan(false)}
        onConfirm={async (reason) => {
          await banNow.run(reason);
        }}
      />
    </>
  );
}

function Staff() {
  const q = useQuery({ queryKey: ["staff"], queryFn: () => api.get<{ items: StaffRow[] }>("/admin/staff") });
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<StaffRow | null>(null);
  const remove = useAction({ run: (id: string) => api.post(`/admin/staff/${id}/remove`), success: "Removed. They are signed out everywhere.", invalidate: [["staff"]] });

  return (
    <>
      <div className="toolbar" style={{ justifyContent: "space-between" }}>
        <span className="muted">Support sees disputes and moderation only.</span>
        <Button variant="primary" icon="plus" onClick={() => setAdding(true)}>
          Add Support
        </Button>
      </div>
      <Card>
        {q.isPending && <SkeletonTable rows={2} cols={3} label="Loading staff" />}
        {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
        {q.data && (
          <DataTable
            label="Staff"
            rows={q.data.items}
            rowKey={(s) => s.id}
            columns={[
              { key: "name", header: "Name", cell: (s) => <strong>{s.display_name}</strong> },
              { key: "role", header: "Role", cell: (s) => <Badge tone="info">{s.role}</Badge> },
              { key: "since", header: "Added", cell: (s) => day(s.created_at) },
              { key: "act", header: "", className: "actions", cell: (s) => <Button variant="ghost" icon="ban" onClick={() => setRemoving(s)}>Remove</Button> },
            ]}
          />
        )}
      </Card>
      <AddStaff open={adding} onClose={() => setAdding(false)} />
      <ConfirmDialog
        open={removing !== null}
        title="Remove this staff member?"
        summary={<><strong>{removing?.display_name}</strong> becomes an ordinary user and is signed out everywhere.</>}
        confirmLabel="Remove"
        danger
        noteLabel={false}
        onClose={() => setRemoving(null)}
        onConfirm={async () => {
          if (removing) await remove.run(removing.id);
        }}
      />
    </>
  );
}

function AddStaff({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [phone, setPhone] = useState("");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const add = useAction({ run: () => api.post("/admin/staff", { phone, name, password }), success: "Support account created. Pass the password on yourself.", invalidate: [["staff"]] });
  return (
    <Modal
      open={open}
      title="Add a Support account"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            icon="plus"
            loading={add.busy}
            disabled={!phone || !name || password.length < 8}
            onClick={() =>
              void add
                .run(undefined)
                .then(() => {
                  setPhone("");
                  setName("");
                  setPassword("");
                  onClose();
                })
                .catch(() => {})
            }
          >
            Create
          </Button>
        </>
      }
    >
      <label className="field"><span>Name</span><input value={name} onChange={(e) => setName(e.target.value)} maxLength={80} /></label>
      <label className="field"><span>Phone number</span><input value={phone} onChange={(e) => setPhone(e.target.value)} inputMode="tel" /></label>
      <label className="field"><span>First password (at least 8 characters)</span><input type="text" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="off" /></label>
      <p className="hint">They sign in with this. Nothing forces a change yet, so choose something they will replace.</p>
    </Modal>
  );
}
