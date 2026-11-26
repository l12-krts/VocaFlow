import { useCallback, useEffect, useState } from 'react';
import { api, events, on } from './ipc';
import type { ModelsState, Settings } from './types';

/** Settings with optimistic updates: the UI moves first, the backend confirms. */
export function useSettings() {
  const [settings, setLocal] = useState<Settings | null>(null);
  useEffect(() => {
    api.getSettings().then(setLocal);
    return on<Settings>(events.settings, setLocal);
  }, []);

  const update = useCallback(async (patch: Partial<Settings>): Promise<string | null> => {
    let prev: Settings | null = null;
    setLocal((s) => {
      prev = s;
      return s ? { ...s, ...patch } : s;
    });
    try {
      setLocal(await api.setSettings(patch));
      return null;
    } catch (e) {
      setLocal(prev);
      return typeof e === 'string' ? e : 'Couldn’t save';
    }
  }, []);

  return [settings, update] as const;
}

export function useModels() {
  const [models, set] = useState<ModelsState | null>(null);
  useEffect(() => {
    api.models().then(set);
    return on<ModelsState>(events.models, set);
  }, []);
  return models;
}
