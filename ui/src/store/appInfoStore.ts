import { create } from "zustand";
import { api, type AppInfo } from "../lib/ipc";

interface AppInfoState {
  info: AppInfo | null;
  load: () => Promise<void>;
  setInfo: (info: AppInfo) => void;
}

export const useAppInfoStore = create<AppInfoState>((set) => ({
  info: null,
  load: async () => set({ info: await api.getAppInfo() }),
  setInfo: (info) => set({ info }),
}));
