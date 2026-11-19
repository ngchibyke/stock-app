import { motion } from "motion/react";
import { type ReactNode } from "react";
import { duration, ease } from "../motion";
import { Button } from "./Button";
import { Icon } from "./Icon";

export interface Column<T> {
  key: string;
  header: string;
  cell: (row: T) => ReactNode;
  className?: string;
}

/** A table whose rows rise in one after another and light up when hovered. */
export function DataTable<T>({
  rows,
  columns,
  rowKey,
  onRowClick,
  label,
}: {
  rows: T[];
  columns: Column<T>[];
  rowKey: (row: T) => string;
  onRowClick?: (row: T) => void;
  label: string;
}) {
  return (
    <div style={{ overflowX: "auto" }}>
      <table className="table" aria-label={label}>
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={c.key} className={c.className}>
                {c.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <motion.tr
              key={rowKey(row)}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: duration.base, ease: ease.out, delay: Math.min(i * 0.025, 0.3) }}
              whileTap={onRowClick ? { scale: 0.995 } : undefined}
              onClick={onRowClick ? () => onRowClick(row) : undefined}
              style={onRowClick ? { cursor: "pointer" } : undefined}
              tabIndex={onRowClick ? 0 : undefined}
              onKeyDown={onRowClick ? (e) => (e.key === "Enter" ? onRowClick(row) : undefined) : undefined}
            >
              {columns.map((c) => (
                <td key={c.key} className={c.className}>
                  {c.cell(row)}
                </td>
              ))}
            </motion.tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function LoadMore({ hasMore, loading, onClick }: { hasMore: boolean; loading: boolean; onClick: () => void }) {
  if (!hasMore) return null;
  return (
    <div style={{ display: "flex", justifyContent: "center", padding: 14 }}>
      <Button icon="refresh" loading={loading} onClick={onClick}>
        Load more
      </Button>
    </div>
  );
}

/** A search box with a magnifier that tilts on hover. */
export function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <label className="search">
      <Icon name="search" size={18} />
      <input type="search" value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} aria-label={placeholder} />
    </label>
  );
}
