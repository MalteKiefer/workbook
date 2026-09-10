import { create } from "zustand";

export type View = "customers" | "systems" | "journal" | "settings";
export type SettingsTab = "general" | "backup" | "plugins" | "keymap";

interface AppState {
  view: View;
  settingsTab: SettingsTab;
  selectedCustomerId: number | null;
  selectedSystemId: number | null;
  formOpen: boolean;
  editorTarget: "new" | number | null;
  viewingEntryId: number | null;
  exportDialogOpen: boolean;
  customerEditorTarget: "new" | number | null;
  systemEditorTarget: "new" | number | null;
  systemEditorCustomerId: number | null;
  shortcutOverviewOpen: boolean;
  goToCustomers: () => void;
  goToSystems: (customerId?: number) => void;
  goToJournal: () => void;
  goToSettings: (tab?: SettingsTab) => void;
  setSettingsTab: (tab: SettingsTab) => void;
  selectCustomer: (id: number | null) => void;
  selectSystem: (id: number | null) => void;
  openForm: () => void;
  closeForm: () => void;
  openEntryEditor: (target: "new" | number) => void;
  closeEntryEditor: () => void;
  openEntryDetail: (id: number) => void;
  closeEntryDetail: () => void;
  openExportDialog: () => void;
  closeExportDialog: () => void;
  openCustomerEditor: (target: "new" | number) => void;
  closeCustomerEditor: () => void;
  openSystemEditor: (target: "new" | number, customerId: number) => void;
  closeSystemEditor: () => void;
  openShortcutOverview: () => void;
  closeShortcutOverview: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  view: "customers",
  settingsTab: "general",
  selectedCustomerId: null,
  selectedSystemId: null,
  formOpen: false,
  editorTarget: null,
  viewingEntryId: null,
  exportDialogOpen: false,
  customerEditorTarget: null,
  systemEditorTarget: null,
  systemEditorCustomerId: null,
  shortcutOverviewOpen: false,
  goToCustomers: () => set({ view: "customers" }),
  goToSystems: (customerId) =>
    set((state) => ({
      view: "systems",
      selectedCustomerId: customerId ?? state.selectedCustomerId,
    })),
  goToJournal: () => set({ view: "journal" }),
  goToSettings: (tab) => set((state) => ({ view: "settings", settingsTab: tab ?? state.settingsTab })),
  setSettingsTab: (tab) => set({ settingsTab: tab }),
  selectCustomer: (id) => set({ selectedCustomerId: id }),
  selectSystem: (id) => set({ selectedSystemId: id }),
  openForm: () => set({ formOpen: true }),
  closeForm: () => set({ formOpen: false }),
  openEntryEditor: (target) => set({ editorTarget: target }),
  closeEntryEditor: () => set({ editorTarget: null }),
  openEntryDetail: (id) => set({ viewingEntryId: id }),
  closeEntryDetail: () => set({ viewingEntryId: null }),
  openExportDialog: () => set({ exportDialogOpen: true }),
  closeExportDialog: () => set({ exportDialogOpen: false }),
  openCustomerEditor: (target) => set({ customerEditorTarget: target }),
  closeCustomerEditor: () => set({ customerEditorTarget: null }),
  openSystemEditor: (target, customerId) => set({ systemEditorTarget: target, systemEditorCustomerId: customerId }),
  closeSystemEditor: () => set({ systemEditorTarget: null, systemEditorCustomerId: null }),
  openShortcutOverview: () => set({ shortcutOverviewOpen: true }),
  closeShortcutOverview: () => set({ shortcutOverviewOpen: false }),
}));
