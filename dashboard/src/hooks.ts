import { type QueryKey, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useToasts } from "./ui/Toasts";

/** Whether a CSS media query matches now, and keeps following it. */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => (typeof window !== "undefined" && window.matchMedia ? window.matchMedia(query).matches : false));
  useEffect(() => {
    if (!window.matchMedia) return;
    const mq = window.matchMedia(query);
    const on = () => setMatches(mq.matches);
    on();
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, [query]);
  return matches;
}

/** A value that follows `value` after `ms` of quiet (search boxes). */
export function useDebounced<T>(value: T, ms = 250): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

interface ActionOptions<A, R> {
  /** The call. Its rejection is shown as a toast. */
  run: (arg: A) => Promise<R>;
  success?: string | ((r: R) => string);
  /** Queries to refetch after success. */
  invalidate?: QueryKey[];
}

/**
 * A server action with a busy flag, a short-lived `done` flag for the button's
 * check mark, a toast on success and on failure, and cache invalidation.
 * `run` resolves with the result and rejects with the error (so a dialog can
 * stay open), after the toast has been shown.
 */
export function useAction<A, R = unknown>({ run, success, invalidate = [] }: ActionOptions<A, R>) {
  const qc = useQueryClient();
  const toasts = useToasts();
  const [done, setDone] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const m = useMutation({
    mutationFn: run,
    onSuccess: async (r) => {
      await Promise.all(invalidate.map((queryKey) => qc.invalidateQueries({ queryKey })));
      if (success) toasts.success(typeof success === "function" ? success(r) : success);
      setDone(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setDone(false), 1400);
    },
    onError: (e) => toasts.error(e),
  });
  return { run: m.mutateAsync, busy: m.isPending, done };
}
