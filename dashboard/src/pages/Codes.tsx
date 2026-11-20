import { useQuery } from "@tanstack/react-query";
import { motion } from "motion/react";
import { useState } from "react";
import { api } from "../api";
import { useAction } from "../hooks";
import { spring } from "../motion";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { DataTable } from "../ui/Table";
import { Card, CopyChip, EmptyState, ErrorNote, StatusBadge } from "../ui/kit";
import { ConfirmDialog } from "../ui/Modal";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonTable } from "../ui/Skeleton";

export interface AccessCode {
  id: string;
  label: string | null;
  last4: string;
  max_uses: number | null;
  uses: number;
  expires_at: string | null;
  revoked_at: string | null;
}

export function codeStatus(c: AccessCode, now = Date.now()): "revoked" | "expired" | "used_up" | "active" {
  if (c.revoked_at) return "revoked";
  if (c.expires_at && Date.parse(c.expires_at) <= now) return "expired";
  if (c.max_uses !== null && c.uses >= c.max_uses) return "used_up";
  return "active";
}

const when = (iso: string | null) => (iso ? new Date(iso).toLocaleDateString("en-NG", { day: "numeric", month: "short", year: "numeric" }) : "Never");

export function Codes() {
  const q = useQuery({ queryKey: ["codes"], queryFn: () => api.get<AccessCode[]>("/admin/access-codes") });
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<AccessCode | null>(null);
  const [fresh, setFresh] = useState<string | null>(null);

  const revoke = useAction({
    run: (id: string) => api.post(`/admin/access-codes/${id}/revoke`),
    success: "Code revoked. Stores already made are unaffected.",
    invalidate: [["codes"]],
  });

  return (
    <>
      <PageHeader
        title="Access codes"
        subtitle="Sellers need a code to open a store. Share it once; it is never shown again."
        actions={
          <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
            New code
          </Button>
        }
      />
      <Card className="" >
        {q.isPending && <SkeletonTable rows={4} cols={6} label="Loading access codes" />}
        {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
        {q.data && q.data.length === 0 && <EmptyState icon="key" title="No codes yet" hint="Create one to invite a seller." />}
        {q.data && q.data.length > 0 && (
          <DataTable
            label="Access codes"
            rows={q.data}
            rowKey={(c) => c.id}
            columns={[
              { key: "label", header: "Label", cell: (c) => c.label ?? <span className="muted">No label</span> },
              { key: "code", header: "Code", cell: (c) => <span className="mono">•••••-•{c.last4}</span> },
              {
                key: "uses",
                header: "Uses",
                cell: (c) => (
                  <div style={{ minWidth: 110 }}>
                    <span className="mono">
                      {c.uses}
                      {c.max_uses !== null ? ` / ${c.max_uses}` : " / ∞"}
                    </span>
                    {c.max_uses !== null && (
                      <div className="progress" style={{ marginTop: 4 }}>
                        <motion.span initial={{ scaleX: 0 }} animate={{ scaleX: Math.min(1, c.uses / c.max_uses) }} transition={spring.gentle} />
                      </div>
                    )}
                  </div>
                ),
              },
              { key: "exp", header: "Expires", cell: (c) => when(c.expires_at) },
              { key: "status", header: "Status", cell: (c) => <StatusBadge status={codeStatus(c)} /> },
              {
                key: "act",
                header: "",
                className: "actions",
                cell: (c) =>
                  codeStatus(c) === "revoked" ? null : (
                    <Button variant="ghost" icon="ban" onClick={() => setRevoking(c)}>
                      Revoke
                    </Button>
                  ),
              },
            ]}
          />
        )}
      </Card>

      <CreateCode open={creating} onClose={() => setCreating(false)} onCreated={(code) => { setCreating(false); setFresh(code); }} />

      <Modal open={fresh !== null} title="Code created" onClose={() => setFresh(null)} footer={<Button variant="primary" onClick={() => setFresh(null)}>Done</Button>}>
        <p className="muted">Copy it now. Only the last four characters are kept.</p>
        <motion.div className="code-reveal" initial={{ scale: 0.7, opacity: 0, rotate: -3 }} animate={{ scale: 1, opacity: 1, rotate: 0 }} transition={spring.bouncy} style={{ margin: "14px 0" }}>
          {fresh}
        </motion.div>
        <div className="row" style={{ justifyContent: "center" }}>{fresh && <CopyChip value={fresh} label="Copy code" />}</div>
      </Modal>

      <ConfirmDialog
        open={revoking !== null}
        title="Revoke this code?"
        summary={<>Nobody can redeem <strong>{revoking?.label ?? "this code"}</strong> any more. Stores already created with it stay.</>}
        confirmLabel="Revoke"
        danger
        noteLabel={false}
        onClose={() => setRevoking(null)}
        onConfirm={async () => {
          if (revoking) await revoke.run(revoking.id);
        }}
      />
    </>
  );
}

function CreateCode({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (code: string) => void }) {
  const [label, setLabel] = useState("");
  const [uses, setUses] = useState("");
  const [expires, setExpires] = useState("");
  const create = useAction({
    run: () =>
      api.post<{ id: string; code: string }>("/admin/access-codes", {
        label: label.trim() || null,
        max_uses: uses ? Number(uses) : null,
        expires_at: expires ? new Date(`${expires}T23:59:59`).toISOString() : null,
      }),
    invalidate: [["codes"]],
  });
  const submit = async () => {
    const r = await create.run(undefined);
    setLabel("");
    setUses("");
    setExpires("");
    onCreated(r.code);
  };
  return (
    <Modal
      open={open}
      title="New access code"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" icon="key" loading={create.busy} onClick={() => void submit().catch(() => {})}>
            Create
          </Button>
        </>
      }
    >
      <label className="field">
        <span>Label (who is it for?)</span>
        <input value={label} onChange={(e) => setLabel(e.target.value)} maxLength={80} placeholder="Wuse market traders" />
      </label>
      <label className="field">
        <span>Most uses (empty for unlimited)</span>
        <input type="number" min={1} value={uses} onChange={(e) => setUses(e.target.value)} placeholder="Unlimited" />
      </label>
      <label className="field">
        <span>Expires on (optional)</span>
        <input type="date" value={expires} onChange={(e) => setExpires(e.target.value)} />
      </label>
    </Modal>
  );
}
