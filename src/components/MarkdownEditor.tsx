import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, placeholder as placeholderExtension } from "@codemirror/view";
import { defaultKeymap, historyKeymap, history } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { basicSetup } from "codemirror";

export interface MarkdownEditorHandle {
  insertAtCursor: (text: string) => void;
  focus: () => void;
}

interface MarkdownEditorProps {
  value: string;
  onChange: (value: string) => void;
  onPaste?: (event: ClipboardEvent) => void;
  placeholder?: string;
}

// Dark/dense/monospace theme matching the rest of the app's overlay components
// (Modal.tsx, CommandPalette.tsx use #1e1e1e background / #e2e8f0 text). CodeMirror
// does not inherit page styles, so this is set explicitly. The default focus ring is
// intentionally left alone (not overridden) — the spec requires focus to always be
// visible.
const editorTheme = EditorView.theme({
  "&": {
    color: "#e2e8f0",
    backgroundColor: "#1e1e1e",
    fontSize: "0.9rem",
  },
  ".cm-content": {
    fontFamily: "monospace",
    caretColor: "#e2e8f0",
  },
  ".cm-gutters": {
    backgroundColor: "#181818",
    color: "#6b7280",
    border: "none",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
    backgroundColor: "#2a2a2a",
  },
  ".cm-activeLine": {
    backgroundColor: "#232323",
  },
  ".cm-placeholder": {
    color: "#6b7280",
  },
});

const MarkdownEditor = forwardRef<MarkdownEditorHandle, MarkdownEditorProps>(function MarkdownEditor(
  { value, onChange, onPaste, placeholder },
  ref,
) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  // onChange/onPaste captured in refs so the CodeMirror extensions (set up once)
  // always call the latest callback without needing to recreate the editor.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const onPasteRef = useRef(onPaste);
  onPasteRef.current = onPaste;

  useEffect(() => {
    if (!containerRef.current) return;
    const state = EditorState.create({
      doc: value,
      extensions: [
        basicSetup,
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        markdown(),
        EditorView.lineWrapping,
        editorTheme,
        ...(placeholder ? [placeholderExtension(placeholder)] : []),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onChangeRef.current(update.state.doc.toString());
          }
        }),
        EditorView.domEventHandlers({
          paste: (event) => {
            onPasteRef.current?.(event);
          },
        }),
      ],
    });
    const view = new EditorView({ state, parent: containerRef.current });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Intentionally empty deps: the editor is created once; external `value`
    // updates (e.g. resetting a draft, or the Strg+Alt+S event path that sets
    // bodyMd directly without going through this component) are synced below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Keep the editor's document in sync when `value` changes from OUTSIDE
  // (i.e. not from the user typing, which already updated `value` via onChange
  // and would otherwise cause a redundant/cursor-resetting re-sync here).
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== value) {
      view.dispatch({ changes: { from: 0, to: current.length, insert: value } });
    }
  }, [value]);

  useImperativeHandle(ref, () => ({
    insertAtCursor: (text: string) => {
      const view = viewRef.current;
      if (!view) return;
      const { from, to } = view.state.selection.main;
      view.dispatch({
        changes: { from, to, insert: text },
        selection: { anchor: from + text.length },
      });
      view.focus();
    },
    focus: () => viewRef.current?.focus(),
  }));

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1,
        minHeight: "10rem",
        border: "1px solid #333",
        borderRadius: "4px",
        overflow: "auto",
      }}
    />
  );
});

export default MarkdownEditor;
