import { create } from "zustand";

export interface ChoiceOption {
  id: string;
  label: string;
  description?: string;
  tag?: string;
  icon?: string;
}

export interface ChoiceData {
  title: string;
  subtitle?: string;
  options: ChoiceOption[];
  onSelect: (optionId: string) => void;
  onCancel?: () => void;
}

interface InteractiveChoiceStore {
  activeChoice: ChoiceData | null;
  setChoice: (choice: ChoiceData | null) => void;
  resolveChoice: (optionId: string) => void;
  cancelChoice: () => void;
}

export const useInteractiveChoice = create<InteractiveChoiceStore>((set, get) => ({
  activeChoice: null,
  setChoice: (activeChoice) => set({ activeChoice }),
  resolveChoice: (optionId) => {
    const choice = get().activeChoice;
    if (choice) {
      choice.onSelect(optionId);
      set({ activeChoice: null });
    }
  },
  cancelChoice: () => {
    const choice = get().activeChoice;
    if (choice) {
      choice.onCancel?.();
      set({ activeChoice: null });
    }
  },
}));
