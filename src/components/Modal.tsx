import { useEffect, useRef } from "react";

interface ModalProps {
  onClose: () => void;
  children: React.ReactNode;
}

export default function Modal({ onClose, children }: ModalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const previouslyFocusedRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    previouslyFocusedRef.current = document.activeElement as HTMLElement | null;
    const firstField = containerRef.current?.querySelector<HTMLElement>("input, textarea, select, button");
    firstField?.focus();
    return () => {
      previouslyFocusedRef.current?.focus();
    };
  }, []);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key !== "Tab" || !containerRef.current) return;
      const focusable = containerRef.current.querySelectorAll<HTMLElement>(
        "input, textarea, select, button, [tabindex]:not([tabindex='-1'])",
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, []);

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(8, 9, 12, 0.55)",
        backdropFilter: "blur(2px)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 100,
      }}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={containerRef}
        style={{
          background: "var(--bg-elevated)",
          color: "var(--text-primary)",
          padding: "1.25rem",
          borderRadius: "var(--radius-lg)",
          border: "1px solid var(--border)",
          boxShadow: "var(--shadow-modal)",
        }}
      >
        {children}
      </div>
    </div>
  );
}
