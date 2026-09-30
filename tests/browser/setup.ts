// Test-only entry: use the real App and Tauri's official IPC/event mocks.
// This file is not imported by the production entry and contains synthetic data only.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { batch, type HarnessOptions } from "./fixtures";

declare global {
  interface Window {
    __wtmOptions: HarnessOptions;
    __wtm: {
      commands: string[];
      activeListeners: Map<number, string>;
      emit: typeof emit;
      copied: string | null;
      exports: Array<{ format: string; content: string }>;
      snapshots: number;
    };
  }
}

const options = window.__wtmOptions;
for (const [key, value] of Object.entries(options.saved ?? {})) localStorage.setItem(key, value);
window.__wtm = {
  commands: [],
  activeListeners: new Map(),
  emit,
  copied: null,
  exports: [],
  snapshots: 0,
};
mockWindows("main");
mockIPC(
  async (command, args) => {
    if (command === "get_snapshot") {
      window.__wtm.snapshots++;
      if (options.delaySnapshotMs)
        await new Promise((resolve) => setTimeout(resolve, options.delaySnapshotMs));
      if (options.failSnapshot) throw new Error("Synthetic snapshot failure");
      if (options.injectBeforeSnapshot)
        await emit("monitor-update", batch([options.injectBeforeSnapshot], 1));
      if (options.injectUpdate) await emit("monitor-update", options.injectUpdate);
      if (window.__wtm.snapshots > 1 && options.resyncSnapshot) return options.resyncSnapshot;
      return options.snapshot;
    }
    if (command === "save_export") {
      if (options.failExport) throw new Error("Synthetic export failure");
      window.__wtm.exports.push(args as { format: string; content: string });
      return true;
    }
    if (command === "plugin:clipboard-manager|write_text") {
      window.__wtm.copied = (args as { text: string }).text;
      return;
    }
    throw new Error(`Unexpected command: ${command}`);
  },
  { shouldMockEvents: true },
);

const internal = window as unknown as {
  __TAURI_INTERNALS__: {
    invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
};
const invoke = internal.__TAURI_INTERNALS__.invoke;
internal.__TAURI_INTERNALS__.invoke = async (command, args) => {
  window.__wtm.commands.push(command);
  if (command === "plugin:event|listen" && args?.event === options.failListen) {
    throw new Error("Synthetic listener registration failure");
  }
  // api/mocks currently names the unlisten ID differently from the public event API.
  const mockArgs = command === "plugin:event|unlisten" ? { ...args, id: args?.eventId } : args;
  const result = await invoke(command, mockArgs);
  if (command === "plugin:event|listen")
    window.__wtm.activeListeners.set(result as number, String(args?.event));
  if (command === "plugin:event|unlisten")
    window.__wtm.activeListeners.delete(args?.eventId as number);
  return result;
};

await import("../../src/main");
