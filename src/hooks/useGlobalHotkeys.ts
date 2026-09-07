import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../state/appStore";

export function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || (el as HTMLElement).isContentEditable;
}

export function useGlobalHotkeys() {
  const pendingPrefixRef = useRef<string | null>(null);
  const pendingTimeoutRef = useRef<number | null>(null);

  const goToCustomers = useAppStore((s) => s.goToCustomers);
  const goToSystems = useAppStore((s) => s.goToSystems);
  const goToJournal = useAppStore((s) => s.goToJournal);
  const formOpen = useAppStore((s) => s.formOpen);
  const closeForm = useAppStore((s) => s.closeForm);
  const selectedCustomerId = useAppStore((s) => s.selectedCustomerId);
  const selectedSystemId = useAppStore((s) => s.selectedSystemId);
  const view = useAppStore((s) => s.view);

  useEffect(() => {
    function clearPrefix() {
      pendingPrefixRef.current = null;
      if (pendingTimeoutRef.current !== null) {
        window.clearTimeout(pendingTimeoutRef.current);
        pendingTimeoutRef.current = null;
      }
    }

    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (formOpen) {
          e.preventDefault();
          closeForm();
        } else if (view === "systems") {
          goToCustomers();
        }
        clearPrefix();
        return;
      }

      if (e.ctrlKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        void invoke("open_quick_capture_with_context", {
          customerId: selectedCustomerId,
          systemId: selectedSystemId,
        });
        return;
      }

      if (isTypingTarget(document.activeElement) || e.ctrlKey || e.metaKey || e.altKey) {
        return;
      }

      if (pendingPrefixRef.current === "g") {
        clearPrefix();
        if (e.key === "c") {
          e.preventDefault();
          goToCustomers();
        } else if (e.key === "s" && selectedCustomerId !== null) {
          e.preventDefault();
          goToSystems();
        } else if (e.key === "j") {
          e.preventDefault();
          goToJournal();
        }
        return;
      }

      if (e.key === "g") {
        pendingPrefixRef.current = "g";
        pendingTimeoutRef.current = window.setTimeout(clearPrefix, 800);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      clearPrefix();
    };
  }, [formOpen, view, selectedCustomerId, selectedSystemId, goToCustomers, goToSystems, goToJournal, closeForm]);
}
