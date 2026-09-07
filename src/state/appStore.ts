import { create } from "zustand";

export type View = "customers" | "systems" | "journal";

interface AppState {
  view: View;
  selectedCustomerId: number | null;
  selectedSystemId: number | null;
  formOpen: boolean;
  editorTarget: "new" | number | null;
  goToCustomers: () => void;
  goToSystems: (customerId?: number) => void;
  goToJournal: () => void;
  selectCustomer: (id: number | null) => void;
  selectSystem: (id: number | null) => void;
  openForm: () => void;
  closeForm: () => void;
  openEntryEditor: (target: "new" | number) => void;
  closeEntryEditor: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  view: "customers",
  selectedCustomerId: null,
  selectedSystemId: null,
  formOpen: false,
  editorTarget: null,
  goToCustomers: () => set({ view: "customers" }),
  goToSystems: (customerId) =>
    set((state) => ({
      view: "systems",
      selectedCustomerId: customerId ?? state.selectedCustomerId,
    })),
  goToJournal: () => set({ view: "journal" }),
  selectCustomer: (id) => set({ selectedCustomerId: id }),
  selectSystem: (id) => set({ selectedSystemId: id }),
  openForm: () => set({ formOpen: true }),
  closeForm: () => set({ formOpen: false }),
  openEntryEditor: (target) => set({ editorTarget: target }),
  closeEntryEditor: () => set({ editorTarget: null }),
}));
