// Skeletons shaped like what they stand in for, so a page does not jump when its
// data arrives: a table's rows, a grid of stat cards, a detail list, a form.
// One shimmer (`.bone`) drives all of them; people who ask for less motion get a
// still, slightly pulsing block instead (see styles.css).
import { type CSSProperties } from "react";

/** One grey block. Sizes are CSS lengths or pixel numbers. */
export function Bone({ w = "100%", h = 14, r = 7, style, className = "" }: { w?: number | string; h?: number | string; r?: number | string; style?: CSSProperties; className?: string }) {
  return <span aria-hidden="true" className={`bone ${className}`} style={{ width: w, height: h, borderRadius: r, ...style }} />;
}

/** The wrapper every skeleton shares: announces itself once, hides its blocks. */
function Busy({ label, className = "", children }: { label: string; className?: string; children: React.ReactNode }) {
  return (
    <div className={`skel ${className}`} role="status" aria-live="polite" aria-busy="true">
      <span className="sr-only">{label}</span>
      {children}
    </div>
  );
}

/** A few lines of text, each shorter than the one above. */
export function Skeleton({ lines = 3, label = "Loading" }: { lines?: number; label?: string }) {
  return (
    <Busy label={label} className="skel-lines">
      {Array.from({ length: lines }, (_, i) => (
        <Bone key={i} w={`${Math.max(30, 92 - i * 13)}%`} />
      ))}
    </Busy>
  );
}

/** A table: a header strip and rows whose cells vary in width, like real data. */
export function SkeletonTable({ rows = 6, cols = 5, label = "Loading rows" }: { rows?: number; cols?: number; label?: string }) {
  // Deterministic widths so the pattern is stable between renders.
  const width = (r: number, c: number) => 38 + ((r * 7 + c * 19) % 5) * 11;
  return (
    <Busy label={label} className="skel-table">
      <div className="skel-row skel-head" style={{ gridTemplateColumns: `repeat(${cols}, 1fr)` }}>
        {Array.from({ length: cols }, (_, c) => (
          <Bone key={c} w={`${48 + (c % 3) * 12}%`} h={10} />
        ))}
      </div>
      {Array.from({ length: rows }, (_, r) => (
        <div key={r} className="skel-row" style={{ gridTemplateColumns: `repeat(${cols}, 1fr)`, ["--i" as string]: r }}>
          {Array.from({ length: cols }, (_, c) => (
            <Bone key={c} w={c === 1 ? 68 : `${width(r, c)}%`} h={c === 1 ? 22 : 13} r={c === 1 ? 999 : 7} />
          ))}
        </div>
      ))}
    </Busy>
  );
}

/** A grid of stat cards: a label, then a big number. */
export function SkeletonStats({ count = 8, label = "Loading figures" }: { count?: number; label?: string }) {
  return (
    <Busy label={label} className="grid skel-stats">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="card skel-stat" style={{ ["--i" as string]: i }}>
          <Bone w="62%" h={12} />
          <Bone w={54} h={30} r={9} />
        </div>
      ))}
    </Busy>
  );
}

/** Label and value pairs, as in an order or dispute's detail panel. */
export function SkeletonDetail({ rows = 6, label = "Loading details" }: { rows?: number; label?: string }) {
  return (
    <Busy label={label} className="skel-detail">
      <div className="skel-title">
        <Bone w="46%" h={20} r={8} />
        <Bone w={72} h={22} r={999} />
      </div>
      <div className="skel-kv">
        {Array.from({ length: rows }, (_, i) => (
          <div key={i} className="skel-kv-row">
            <Bone w="70%" h={11} />
            <Bone w={`${52 + ((i * 13) % 5) * 9}%`} h={13} />
          </div>
        ))}
      </div>
    </Busy>
  );
}

/** A form: a label over an input, repeated. */
export function SkeletonForm({ fields = 6, label = "Loading settings" }: { fields?: number; label?: string }) {
  return (
    <Busy label={label} className="form-grid skel-form">
      {Array.from({ length: fields }, (_, i) => (
        <div key={i} className="skel-field">
          <Bone w="44%" h={11} />
          <Bone w="100%" h={38} r={10} />
        </div>
      ))}
    </Busy>
  );
}

/** A list of cards, each an icon beside two lines (alerts, flags). */
export function SkeletonCards({ count = 3, label = "Loading" }: { count?: number; label?: string }) {
  return (
    <Busy label={label} className="skel-cards">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="card skel-card" style={{ ["--i" as string]: i }}>
          <Bone w={42} h={42} r={13} />
          <div className="skel-card-text">
            <Bone w="52%" h={14} />
            <Bone w="78%" h={11} />
          </div>
        </div>
      ))}
    </Busy>
  );
}

/** The whole app frame, shown while the first sign-in check is still out. */
export function SkeletonShell() {
  return (
    <div className="shell" role="status" aria-live="polite" aria-busy="true">
      <span className="sr-only">Loading the dashboard</span>
      <aside className="side skel-side" aria-hidden="true">
        <div className="brand">
          <Bone w={30} h={30} r={9} />
          <Bone w={64} h={14} />
        </div>
        {Array.from({ length: 9 }, (_, i) => (
          <div key={i} className="skel-nav" style={{ ["--i" as string]: i }}>
            <Bone w={20} h={20} r={6} />
            <Bone w={`${46 + ((i * 11) % 4) * 10}%`} h={12} />
          </div>
        ))}
      </aside>
      <main className="main" aria-hidden="true">
        <Bone w={220} h={26} r={8} />
        <Bone w={320} h={13} style={{ marginTop: 10 }} />
        <div style={{ marginTop: 24 }}>
          <SkeletonStats count={4} label="" />
        </div>
      </main>
    </div>
  );
}
