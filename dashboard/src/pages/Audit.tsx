import { useInfiniteQuery } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { api } from "../api";
import { dateTime, words } from "../format";
import { useDebounced } from "../hooks";
import { Icon } from "../ui/Icon";
import { LoadMore } from "../ui/Table";
import { Card, CopyChip, EmptyState, ErrorNote } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonTable } from "../ui/Skeleton";

interface Row {
  id: number;
  at: string;
  actor_id: string | null;
  action: string;
  subject_type: string;
  subject_id: string | null;
  detail: Record<string, unknown>;
}
interface Page {
  items: Row[];
  next_cursor: string | null;
}

export function Audit() {
  const [action, setAction] = useState("");
  const [actor, setActor] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const a = useDebounced(action.trim());
  const who = useDebounced(actor.trim());
  const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(who);
  const [open, setOpen] = useState<number | null>(null);

  const list = useInfiniteQuery({
    queryKey: ["audit", a, uuid ? who : "", from, to],
    initialPageParam: "",
    queryFn: ({ pageParam }) =>
      api.get<Page>("/admin/audit", {
        action: a,
        actor_id: uuid ? who : undefined,
        before: pageParam,
        limit: 50,
        from: from ? new Date(`${from}T00:00:00`).toISOString() : undefined,
        to: to ? new Date(`${to}T23:59:59`).toISOString() : undefined,
      }),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const rows = list.data?.pages.flatMap((p) => p.items) ?? [];

  return (
    <>
      <PageHeader title="Audit log" subtitle="Who did what, and when. Nobody can edit or delete these rows." />
      <div className="toolbar">
        <input style={{ maxWidth: 260 }} value={action} onChange={(e) => setAction(e.target.value)} placeholder="Exact action, e.g. admin.store_suspended" aria-label="Action" />
        <input style={{ maxWidth: 300 }} value={actor} onChange={(e) => setActor(e.target.value)} placeholder="Actor id (full)" aria-label="Actor id" />
        <label className="row muted">From <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} style={{ width: "auto" }} /></label>
        <label className="row muted">To <input type="date" value={to} onChange={(e) => setTo(e.target.value)} style={{ width: "auto" }} /></label>
      </div>
      <Card>
        {list.isPending && <SkeletonTable rows={8} cols={4} label="Loading the audit log" />}
        {list.isError && <ErrorNote error={list.error} onRetry={() => void list.refetch()} />}
        {list.data && rows.length === 0 && <EmptyState icon="audit" title="No entries match" />}
        {rows.length > 0 && (
          <div role="list" aria-label="Audit entries">
            {rows.map((r) => (
              <div key={r.id} role="listitem" className="audit-row">
                <button type="button" className="audit-head" aria-expanded={open === r.id} onClick={() => setOpen(open === r.id ? null : r.id)}>
                  <motion.span animate={{ rotate: open === r.id ? 90 : 0 }} style={{ display: "inline-flex" }}>
                    <Icon name="chevron" size={16} interactive={false} />
                  </motion.span>
                  <strong className="mono">{r.action}</strong>
                  <span className="muted">{words(r.subject_type)}</span>
                  <span className="muted" style={{ marginLeft: "auto" }}>{dateTime(r.at)}</span>
                </button>
                <AnimatePresence initial={false}>
                  {open === r.id && (
                    <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }} exit={{ height: 0, opacity: 0 }} style={{ overflow: "hidden" }}>
                      <dl className="kv" style={{ padding: "8px 12px 12px" }}>
                        <dt>Actor</dt>
                        <dd>{r.actor_id ? <CopyChip value={r.actor_id} label={r.actor_id.slice(0, 8)} /> : <span className="muted">the system</span>}</dd>
                        <dt>Subject</dt>
                        <dd>{r.subject_id ? <CopyChip value={r.subject_id} label={r.subject_id.slice(0, 8)} /> : "–"}</dd>
                        <dt>Detail</dt>
                        <dd><pre className="mono detail">{JSON.stringify(r.detail, null, 2)}</pre></dd>
                      </dl>
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            ))}
          </div>
        )}
        <LoadMore hasMore={!!list.hasNextPage} loading={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()} />
      </Card>
    </>
  );
}
