import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api";
import { useAuth } from "../auth";
import { ago, words } from "../format";
import { useAction } from "../hooks";
import { Button } from "../ui/Button";
import { Drawer } from "../ui/Drawer";
import { ConfirmDialog } from "../ui/Modal";
import { DataTable, LoadMore } from "../ui/Table";
import { Badge, Card, CopyChip, EmptyState, ErrorNote, Segmented, Skeleton, StatusBadge, Tabs } from "../ui/kit";

interface Flag {
  id: string;
  user_id: string;
  surface: string;
  severity: string;
  category: string;
  source: string;
  status: string;
  has_strike: boolean;
  created_at: string;
}
interface FlagPage {
  items: Flag[];
  next_cursor: string | null;
}
interface ReviewRow {
  user_id: string;
  owner_review_at: string;
  muted_until: string | null;
  active_strikes: number;
}

const sevTone = (s: string) => (s === "serious" ? "bad" : s === "unreviewable" ? "warn" : "neutral");

export function Moderation() {
  const [tab, setTab] = useState<"flags" | "bans" | "reviews">("flags");
  const { user } = useAuth();
  const owner = user?.role === "owner";
  return (
    <>
      <div className="page-head">
        <div>
          <h1>Moderation</h1>
          <p>Messages the filters caught, people waiting on a ban decision, and reviews to hide.</p>
        </div>
      </div>
      <div style={{ marginTop: 14 }}>
        <Tabs id="mod" value={tab} onChange={setTab} tabs={[{ value: "flags", label: "Flagged messages" }, { value: "bans", label: "Ban review" }, { value: "reviews", label: "Reviews" }]} />
      </div>
      {tab === "flags" && <Flags owner={owner} />}
      {tab === "bans" && <BanReview owner={owner} />}
      {tab === "reviews" && <HideReview />}
    </>
  );
}

function Flags({ owner }: { owner: boolean }) {
  const [status, setStatus] = useState<"open" | "upheld" | "dismissed">("open");
  const [severity, setSeverity] = useState<"" | "mild" | "serious" | "unreviewable">("");
  const [open, setOpen] = useState<Flag | null>(null);
  const list = useInfiniteQuery({
    queryKey: ["flags", status, severity],
    initialPageParam: "",
    queryFn: ({ pageParam }) => api.get<FlagPage>("/admin/moderation/flags", { status, severity, before: pageParam, limit: 25 }),
    getNextPageParam: (l) => l.next_cursor ?? undefined,
  });
  const rows = list.data?.pages.flatMap((p) => p.items) ?? [];
  return (
    <>
      <div className="toolbar">
        <Segmented id="fstatus" label="Status" value={status} onChange={setStatus} options={[{ value: "open", label: "Open" }, { value: "upheld", label: "Upheld" }, { value: "dismissed", label: "Dismissed" }]} />
        <Segmented id="fsev" label="Severity" value={severity} onChange={setSeverity} options={[{ value: "", label: "Any" }, { value: "mild", label: "Mild" }, { value: "serious", label: "Serious" }, { value: "unreviewable", label: "Unreviewable" }]} />
      </div>
      <Card>
        {list.isPending && <Skeleton lines={5} />}
        {list.isError && <ErrorNote error={list.error} onRetry={() => void list.refetch()} />}
        {list.data && rows.length === 0 && <EmptyState icon="flag" title="Nothing flagged" hint="No messages in this state." />}
        {rows.length > 0 && (
          <DataTable
            label="Flagged messages"
            rows={rows}
            rowKey={(f) => f.id}
            onRowClick={setOpen}
            columns={[
              { key: "sev", header: "Severity", cell: (f) => <Badge tone={sevTone(f.severity)}>{f.severity}</Badge> },
              { key: "cat", header: "Category", cell: (f) => <strong style={{ textTransform: "capitalize" }}>{words(f.category)}</strong> },
              { key: "surf", header: "Where", cell: (f) => words(f.surface) },
              { key: "src", header: "Caught by", cell: (f) => <span className="muted">{words(f.source)}</span> },
              { key: "st", header: "Status", cell: (f) => <span className="row"><StatusBadge status={f.status} />{f.has_strike && <Badge tone="warn">strike</Badge>}</span> },
              { key: "t", header: "When", cell: (f) => ago(f.created_at) },
            ]}
          />
        )}
        <LoadMore hasMore={!!list.hasNextPage} loading={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()} />
      </Card>
      <FlagDrawer flag={open} owner={owner} onClose={() => setOpen(null)} />
    </>
  );
}

function FlagDrawer({ flag, owner, onClose }: { flag: Flag | null; owner: boolean; onClose: () => void }) {
  const text = useQuery({
    queryKey: ["flag", flag?.id],
    queryFn: () => api.get<{ original: string; suggestion: string | null }>(`/admin/moderation/flags/${flag?.id}`),
    enabled: flag !== null && owner,
  });
  const [pick, setPick] = useState<"uphold" | "dismiss" | null>(null);
  const resolve = useAction({
    run: (action: "uphold" | "dismiss") => api.post(`/admin/moderation/flags/${flag?.id}/resolve`, { action }),
    success: "Resolved.",
    invalidate: [["flags"]],
  });
  return (
    <>
      <Drawer open={flag !== null} title="Flagged message" onClose={onClose}>
        {flag && (
          <>
            <div className="row">
              <Badge tone={sevTone(flag.severity)}>{flag.severity}</Badge>
              <StatusBadge status={flag.status} />
              {flag.has_strike && <Badge tone="warn">gave a strike</Badge>}
            </div>
            <dl className="kv">
              <dt>Category</dt><dd style={{ textTransform: "capitalize" }}>{words(flag.category)}</dd>
              <dt>Where</dt><dd>{words(flag.surface)}</dd>
              <dt>Caught by</dt><dd>{words(flag.source)}</dd>
              <dt>Person</dt><dd><CopyChip value={flag.user_id} label={flag.user_id.slice(0, 8)} /></dd>
            </dl>
            {owner ? (
              <Card>
                <h2>What was written</h2>
                {text.isPending && <Skeleton lines={2} />}
                {text.isError && <ErrorNote error={text.error} />}
                {text.data && (
                  <>
                    <p className="flagged" style={{ marginTop: 8 }}>{text.data.original}</p>
                    {text.data.suggestion && <p className="hint">We offered them: “{text.data.suggestion}”</p>}
                  </>
                )}
                <p className="hint">Reading the original is written to the audit log.</p>
              </Card>
            ) : (
              <p className="hint">Only the Owner can read the original text.</p>
            )}
            {flag.status === "open" && (
              <div className="row">
                <Button variant="danger" icon="check" onClick={() => setPick("uphold")}>Uphold</Button>
                <Button icon="x" onClick={() => setPick("dismiss")}>Dismiss</Button>
              </div>
            )}
            <p className="hint">Uphold: the filter was right, the strike stands. Dismiss: it was a mistake, the strike is voided.</p>
          </>
        )}
      </Drawer>
      <ConfirmDialog
        open={pick !== null}
        title={pick === "uphold" ? "Uphold this flag?" : "Dismiss this flag?"}
        summary={pick === "uphold" ? <>The strike stays against this person.</> : <>The strike is voided and their mute is recounted.</>}
        confirmLabel={pick === "uphold" ? "Uphold" : "Dismiss"}
        danger={pick === "uphold"}
        noteLabel={false}
        onClose={() => setPick(null)}
        onConfirm={async () => {
          if (pick) await resolve.run(pick);
          onClose();
        }}
      />
    </>
  );
}

function BanReview({ owner }: { owner: boolean }) {
  const q = useQuery({ queryKey: ["ban-review"], queryFn: () => api.get<{ items: ReviewRow[] }>("/admin/moderation/review-queue") });
  const [target, setTarget] = useState<{ kind: "ban" | "unmute"; row: ReviewRow } | null>(null);
  const inv = [["ban-review"], ["users"]];
  const ban = useAction({ run: ({ id, reason }: { id: string; reason: string }) => api.post(`/admin/moderation/users/${id}/ban`, { reason }), success: "User banned.", invalidate: inv });
  const unmute = useAction({ run: (id: string) => api.post(`/admin/moderation/users/${id}/unmute`), success: "Unmuted.", invalidate: inv });
  return (
    <Card>
      {q.isPending && <Skeleton lines={4} />}
      {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
      {q.data && q.data.items.length === 0 && <EmptyState icon="check" title="Nobody is waiting" hint="People who pass the strike threshold appear here for your decision." />}
      {q.data && q.data.items.length > 0 && (
        <DataTable
          label="Ban review"
          rows={q.data.items}
          rowKey={(r) => r.user_id}
          columns={[
            { key: "u", header: "Person", cell: (r) => <CopyChip value={r.user_id} label={r.user_id.slice(0, 8)} /> },
            { key: "s", header: "Active strikes", cell: (r) => <Badge tone="bad">{r.active_strikes}</Badge> },
            { key: "w", header: "Waiting", cell: (r) => ago(r.owner_review_at) },
            { key: "m", header: "Muted", cell: (r) => (r.muted_until && Date.parse(r.muted_until) > Date.now() ? <Badge tone="warn">muted</Badge> : <span className="muted">no</span>) },
            {
              key: "x",
              header: "",
              className: "actions",
              cell: (r) =>
                owner ? (
                  <span className="row" style={{ justifyContent: "flex-end" }}>
                    <Button icon="undo" onClick={() => setTarget({ kind: "unmute", row: r })}>Unmute</Button>
                    <Button variant="danger" icon="ban" onClick={() => setTarget({ kind: "ban", row: r })}>Ban</Button>
                  </span>
                ) : null,
            },
          ]}
        />
      )}
      <ConfirmDialog
        open={target?.kind === "ban"}
        title="Ban this person?"
        summary={<>They cannot post in chat or reviews. You can lift it later from Users.</>}
        confirmLabel="Ban"
        danger
        noteLabel="Reason (the person sees this)"
        onClose={() => setTarget(null)}
        onConfirm={async (reason) => {
          if (target) await ban.run({ id: target.row.user_id, reason });
        }}
      />
      <ConfirmDialog
        open={target?.kind === "unmute"}
        title="Unmute this person?"
        summary={<>They can post again straight away. Their strikes stay on record.</>}
        confirmLabel="Unmute"
        noteLabel={false}
        onClose={() => setTarget(null)}
        onConfirm={async () => {
          if (target) await unmute.run(target.row.user_id);
        }}
      />
    </Card>
  );
}

function HideReview() {
  const [id, setId] = useState("");
  const valid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id.trim());
  const hide = useAction({ run: () => api.post(`/admin/reviews/${id.trim()}/hide`), success: "Review hidden." });
  const unhide = useAction({ run: () => api.post(`/admin/reviews/${id.trim()}/unhide`), success: "Review is visible again." });
  return (
    <Card>
      <h2>Hide or restore a review</h2>
      <p className="muted">Paste the review's id. A hidden review disappears from the product and the store's rating.</p>
      <label className="field">
        <span>Review id</span>
        <input value={id} onChange={(e) => setId(e.target.value)} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" aria-invalid={id !== "" && !valid} />
      </label>
      <div className="row" style={{ marginTop: 14 }}>
        <Button variant="danger" icon="eye" disabled={!valid} loading={hide.busy} done={hide.done} onClick={() => void hide.run(undefined).catch(() => {})}>Hide</Button>
        <Button icon="undo" disabled={!valid} loading={unhide.busy} done={unhide.done} onClick={() => void unhide.run(undefined).catch(() => {})}>Restore</Button>
      </div>
    </Card>
  );
}
