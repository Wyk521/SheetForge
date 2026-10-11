import { listen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";

/** Owns native subscriptions, including registrations finishing after disposal. */
export function createEventScope() {
  let generation = 0;
  let ready: Promise<void> | undefined;
  const unlisteners: UnlistenFn[] = [];

  function dispose() {
    generation += 1;
    ready = undefined;
    for (const unlisten of unlisteners.splice(0)) unlisten();
  }

  function init(register: (on: <T>(name: string, callback: EventCallback<T>) => Promise<void>) => Promise<void>) {
    if (ready) return ready;
    const current = generation;
    ready = (async () => {
      await register(async <T>(name: string, callback: EventCallback<T>) => {
        if (current !== generation) return;
        const unlisten = await listen<T>(name, callback);
        if (current !== generation) unlisten();
        else unlisteners.push(unlisten);
      });
    })().catch((error: unknown) => {
      if (current === generation) dispose();
      throw error;
    });
    return ready;
  }

  return { init, dispose };
}
