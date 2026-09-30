import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CallRecord, Catalog, MonitorStatus, Snapshot, Update } from "./types";

interface Data {
  revision: number;
  calls: Map<string, CallRecord>;
  catalog: Catalog;
  status: MonitorStatus | null;
}
const empty = (): Data => ({
  revision: -1,
  calls: new Map(),
  catalog: { projects: [], conversations: [] },
  status: null,
});
function snapshotData(snapshot: Snapshot): Data {
  const calls = new Map(snapshot.calls.map((call) => [call.id, call]));
  if (
    snapshot.revision !== snapshot.status.revision ||
    calls.size !== snapshot.status.records ||
    calls.size > snapshot.status.retentionLimit
  ) {
    throw new Error("监控快照的统计范围不一致");
  }
  return { revision: snapshot.revision, calls, catalog: snapshot.catalog, status: snapshot.status };
}
function apply(current: Data, update: Update): Data | null {
  if (update.revision <= current.revision) return current;
  if (update.revision !== current.revision + 1 || update.status.revision !== update.revision)
    return null;
  const calls =
    update.calls.length || update.removedIds.length ? new Map(current.calls) : current.calls;
  for (const id of update.removedIds) calls.delete(id);
  for (const call of update.calls) calls.set(call.id, call);
  if (calls.size !== update.status.records || calls.size > update.status.retentionLimit)
    return null;
  return {
    revision: update.revision,
    calls,
    catalog: update.catalog ?? current.catalog,
    status: update.status,
  };
}

/** Ordered batches keep the table's retained window identical to the backend. */
export function useMonitor() {
  const [data, setData] = useState<Data>(empty);
  const current = useRef(data);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(true);
  const [generation, setGeneration] = useState(0);
  const [clock, setClock] = useState(() => performance.now());
  const heartbeat = useRef(performance.now());
  const retry = useCallback(() => setGeneration((n) => n + 1), []);
  useEffect(() => {
    const timer = window.setInterval(() => setClock(performance.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    let active = true,
      failed = false,
      running = false;
    let off: (() => void) | null = null;
    let cancelWait: (() => void) | null = null;
    let pending: Update[] = [],
      pendingItems = 0,
      overflow = false;
    const release = () => {
      const callback = off;
      off = null;
      if (callback) {
        try {
          void Promise.resolve(callback()).catch(console.error);
        } catch (reason) {
          console.error(reason);
        }
      }
    };
    const fail = (reason: unknown) => {
      failed = true;
      release();
      pending = [];
      pendingItems = 0;
      if (active) {
        console.error("Monitor synchronization failed", reason);
        setError("监控初始化失败或同步中断，请重试。");
        setSyncing(false);
      }
    };
    const receivePending = (update: Update) => {
      if (overflow) return;
      pendingItems += update.calls.length + update.removedIds.length;
      if (pending.length >= 8 || pendingItems > 25_000) {
        overflow = true;
        pending = [];
      } else pending.push(update);
    };
    const getSnapshot = () =>
      new Promise<Snapshot>((resolve, reject) => {
        const cancel = () => {
          cleanup();
          reject(new Error("cancelled"));
        };
        const timer = window.setTimeout(() => {
          cleanup();
          reject(new Error("监控快照超时"));
        }, 15_000);
        function cleanup() {
          window.clearTimeout(timer);
          if (cancelWait === cancel) cancelWait = null;
        }
        cancelWait = cancel;
        void invoke<Snapshot>("get_snapshot").then(
          (snapshot) => {
            cleanup();
            resolve(snapshot);
          },
          (reason) => {
            cleanup();
            reject(reason);
          },
        );
      });
    const synchronize = async () => {
      if (running || !active || failed) return;
      running = true;
      setSyncing(true);
      setError(null);
      try {
        for (let attempt = 0; attempt < 3; attempt++) {
          pending = [];
          pendingItems = 0;
          overflow = false;
          const snapshot = await getSnapshot();
          if (!active || failed) return;
          if (overflow) continue;
          let next: Data | null = snapshotData(snapshot);
          for (const update of pending) {
            if (!next) break;
            next = apply(next, update);
          }
          if (!next) continue;
          current.current = next;
          setData(next);
          heartbeat.current = performance.now();
          pending = [];
          pendingItems = 0;
          setSyncing(false);
          return;
        }
        throw new Error("更新过快或批次缺失，需要重新同步");
      } catch (reason) {
        fail(reason);
      } finally {
        running = false;
      }
    };
    const receive = (update: Update) => {
      if (!active || failed) return;
      heartbeat.current = performance.now();
      if (running || current.current.revision < 0) {
        receivePending(update);
        return;
      }
      const next = apply(current.current, update);
      if (!next) {
        void synchronize();
        return;
      }
      current.current = next;
      setData(next);
    };
    setSyncing(true);
    setError(null);
    void (async () => {
      const unsubscribe = await listen<Update>("monitor-update", ({ payload }) => receive(payload));
      if (!active || failed) {
        void Promise.resolve(unsubscribe()).catch(console.error);
        return;
      }
      off = unsubscribe;
      await synchronize();
    })().catch(fail);
    return () => {
      active = false;
      cancelWait?.();
      cancelWait = null;
      pending = [];
      release();
    };
  }, [generation]);
  return {
    ...data,
    error,
    syncing,
    stale: !syncing && !error && clock - heartbeat.current > 5000,
    retry,
  };
}
