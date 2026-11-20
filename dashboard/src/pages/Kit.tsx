// A living page of every piece the dashboard is built from, so a change to one
// can be checked next to the others. Only served by the dev server (/kit).
import { type ReactNode, useState } from "react";
import { Button, IconButton } from "../ui/Button";
import { Avatar, Banner, Checkbox, Kbd, RadioCard } from "../ui/Controls";
import { Drawer } from "../ui/Drawer";
import { Glyphs, type IconName, Icon } from "../ui/Icon";
import { Menu } from "../ui/Menu";
import { ConfirmDialog, Modal } from "../ui/Modal";
import { PageHeader } from "../ui/PageHeader";
import { Bone, Skeleton, SkeletonCards, SkeletonDetail, SkeletonForm, SkeletonStats, SkeletonTable } from "../ui/Skeleton";
import { DataTable } from "../ui/Table";
import { Tooltip } from "../ui/Tooltip";
import { useToasts } from "../ui/Toasts";
import { Badge, Card, Counter, EmptyState, ErrorNote, Money, Segmented, StatusBadge, Tabs, Toggle } from "../ui/kit";

function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <Card>
      <h2>{title}</h2>
      {hint && <p className="muted">{hint}</p>}
      <div className="kit-demo">{children}</div>
    </Card>
  );
}

export function Kit() {
  const toast = useToasts();
  const [tab, setTab] = useState("one");
  const [seg, setSeg] = useState("a");
  const [on, setOn] = useState(true);
  const [check, setCheck] = useState(true);
  const [radio, setRadio] = useState("full");
  const [modal, setModal] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const run = () => {
    setBusy(true);
    setTimeout(() => {
      setBusy(false);
      setDone(true);
      setTimeout(() => setDone(false), 1400);
    }, 900);
  };

  return (
    <>
      <PageHeader title="Kit" subtitle="Every piece the dashboard is built from. Hover, press and tab through them." actions={<Kbd>Ctrl K</Kbd>} />
      <div className="stack">
        <Section title="Buttons" hint="Loading swaps the icon for a spinner; done shows a tick for a moment.">
          <Button variant="primary" icon="check">Primary</Button>
          <Button icon="refresh">Secondary</Button>
          <Button variant="ghost" icon="eye">Ghost</Button>
          <Button variant="danger" icon="trash">Danger</Button>
          <Button variant="primary" icon="bolt" loading={busy} done={done} onClick={run}>Press me</Button>
          <Button disabled>Disabled</Button>
          <IconButton icon="edit" label="Edit" />
          <IconButton icon="copy" label="Copy" />
          <IconButton icon="download" label="Download" />
        </Section>

        <Section title="Icons" hint="Each one has its own motion on hover, press and when its button is selected.">
          <div className="icon-grid">
            {(Object.keys(Glyphs) as IconName[]).map((n) => (
              <Tooltip key={n} label={n}>
                <button type="button" className="icon-cell" aria-label={n}>
                  <Icon name={n} size={22} />
                </button>
              </Tooltip>
            ))}
          </div>
        </Section>

        <Section title="Status and labels">
          <StatusBadge status="completed" />
          <StatusBadge status="awaiting_decision" />
          <StatusBadge status="refunded" />
          <StatusBadge status="draft" />
          <Badge tone="info">Info</Badge>
          <Avatar name="Ada Okafor" />
          <Avatar name="Bola Adeyemi" />
          <Avatar name="Chidi Eze" />
          <Money kobo={2_300_000} />
          <Money kobo={-150_000} signed />
          <span style={{ fontSize: 28, fontWeight: 700 }}><Counter to={1280} /></span>
        </Section>

        <Section title="Choosing">
          <Tabs id="kit" value={tab} onChange={setTab} tabs={[{ value: "one", label: "Payouts", count: 3 }, { value: "two", label: "Refunds" }, { value: "three", label: "Statements" }]} />
          <Segmented id="kit-seg" label="Demo" value={seg} onChange={setSeg} options={[{ value: "a", label: "Open" }, { value: "b", label: "Acknowledged" }]} />
          <Toggle checked={on} onChange={setOn} label="Demo switch" />
          <Checkbox checked={check} onChange={setCheck} label="Attest the provider was checked" hint="Kept with the note in the audit log." />
          <div className="stack" role="radiogroup" aria-label="Decision" style={{ minWidth: 260 }}>
            <RadioCard name="kit-decision" value="full" checked={radio === "full"} onChange={setRadio} title="Full refund">Items and delivery go back to the buyer.</RadioCard>
            <RadioCard name="kit-decision" value="release" checked={radio === "release"} onChange={setRadio} title="Release to the seller">The payout goes out.</RadioCard>
          </div>
        </Section>

        <Section title="Messages" hint="An inline banner for the page, a toast for what just happened.">
          <div className="stack" style={{ width: "100%" }}>
            <Banner tone="info" title="Payment methods on">mono_directpay · Payouts via mono_disburse</Banner>
            <Banner tone="good" title="All checks clear">Nothing is waiting on you.</Banner>
            <Banner tone="warn" title="Someone else changed these" action={<button type="button" className="link">Reload</button>}>Your changes are not saved.</Banner>
            <Banner tone="bad" title="Could not reach the provider" onDismiss={() => undefined}>Try again in a minute.</Banner>
            <ErrorNote error={new Error("Something went wrong.")} onRetry={() => undefined} />
          </div>
          <Button onClick={() => toast.success("Marked as paid by hand.")}>Success toast</Button>
          <Button onClick={() => toast.info("Asked the provider.")}>Info toast</Button>
          <Button onClick={() => toast.error(new Error("The provider said no."))}>Error toast</Button>
        </Section>

        <Section title="Floating" hint="Dialog, confirmation with a note, drawer, menu and tooltip. Esc closes them and focus goes back to where it was.">
          <Button onClick={() => setModal(true)}>Dialog</Button>
          <Button variant="danger" onClick={() => setConfirm(true)}>Confirmation</Button>
          <Button onClick={() => setDrawer(true)}>Drawer</Button>
          <Menu
            label="More actions"
            items={[
              { label: "Open details", icon: "eye", onSelect: () => toast.info("Open details") },
              { label: "Copy ID", icon: "copy", hint: "C", onSelect: () => toast.info("Copied") },
              "separator",
              { label: "Suspend", icon: "ban", danger: true, onSelect: () => toast.info("Suspended") },
              { label: "Not available", icon: "lock", disabled: true, onSelect: () => undefined },
            ]}
          />
          <Tooltip label="Tooltips open after a short pause, or at once from the keyboard"><Button variant="ghost" icon="info">Hover me</Button></Tooltip>
        </Section>

        <Section title="Table">
          <div style={{ width: "100%" }}>
            <DataTable
              label="Demo table"
              rows={[{ id: "1", who: "Ada Okafor", s: "completed", k: 2_300_000 }, { id: "2", who: "Bola Adeyemi", s: "awaiting_decision", k: 1_150_000 }]}
              rowKey={(r) => r.id}
              onRowClick={() => toast.info("Row opened")}
              columns={[
                { key: "who", header: "Name", cell: (r) => <span className="row"><Avatar name={r.who} size={28} /><strong>{r.who}</strong></span> },
                { key: "s", header: "Status", cell: (r) => <StatusBadge status={r.s} /> },
                { key: "k", header: "Amount", className: "num", cell: (r) => <Money kobo={r.k} /> },
              ]}
            />
          </div>
        </Section>

        <Section title="Empty" hint="Shown when there is nothing to list.">
          <div style={{ width: "100%" }}><EmptyState icon="inbox" title="Nothing here yet" hint="New items show up as they arrive." /></div>
        </Section>

        <Section title="Loading skeletons" hint="Shaped like the screen they stand in for, so nothing jumps when the data arrives.">
          <div className="stack" style={{ width: "100%" }}>
            <Skeleton lines={3} />
            <SkeletonTable rows={3} cols={5} />
            <SkeletonStats count={4} />
            <SkeletonCards count={2} />
            <div className="two"><SkeletonDetail rows={3} /><SkeletonForm fields={2} /></div>
            <div className="row"><Bone w={140} h={16} /><Bone w={60} h={16} r={999} /><Bone w={36} h={36} r={999} /></div>
          </div>
        </Section>
      </div>

      <Modal open={modal} title="A dialog" description="One thing to decide, a way out, and a way forward." onClose={() => setModal(false)} footer={<><Button variant="ghost" onClick={() => setModal(false)}>Cancel</Button><Button variant="primary" icon="check" onClick={() => setModal(false)}>Save</Button></>}>
        <label className="field"><span>Label</span><input placeholder="Wuse market traders" /></label>
      </Modal>
      <ConfirmDialog open={confirm} title="Refund the buyer in full?" summary={<>The buyer gets <strong>₦22,300.00</strong> back. This cannot be undone.</>} confirmLabel="Refund now" danger onClose={() => setConfirm(false)} onConfirm={() => new Promise<void>((r) => setTimeout(r, 700))} />
      <Drawer open={drawer} title="A drawer" onClose={() => setDrawer(false)}>
        <SkeletonDetail rows={4} />
      </Drawer>
    </>
  );
}
