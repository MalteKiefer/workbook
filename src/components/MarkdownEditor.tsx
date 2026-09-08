import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { EditorSelection, EditorState, Prec } from "@codemirror/state";
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
  minHeight?: string;
}

// Dark/dense/monospace theme matching the rest of the app's overlay components
// (Modal.tsx, CommandPalette.tsx use #1e1e1e background / #e2e8f0 text). CodeMirror
// does not inherit page styles, so this is set explicitly. The default focus ring is
// intentionally left alone (not overridden) — the spec requires focus to always be
// visible.
const editorTheme = EditorView.theme({
  "&": {
    color: "var(--text-primary)",
    backgroundColor: "var(--bg-surface)",
    fontSize: "0.9rem",
  },
  ".cm-content": {
    fontFamily: "var(--font-mono)",
    caretColor: "var(--text-primary)",
  },
  // CodeMirror's `drawSelection()` extension (part of `basicSetup`) draws its
  // own synthetic cursor as a `.cm-cursor` element instead of relying on the
  // native caret — so the `caretColor` above never actually applies. Without
  // an explicit border color here it falls back to CodeMirror's own default
  // (a dark color), which is invisible against this app's dark editor
  // background. `.cm-cursor-primary` covers the common single-cursor case;
  // `.cm-cursor` covers secondary cursors from multi-selection.
  "&.cm-focused .cm-cursor, .cm-cursor-primary, .cm-cursor": {
    borderLeftColor: "var(--text-primary)",
    borderLeftWidth: "2px",
  },
  ".cm-gutters": {
    backgroundColor: "var(--bg-surface)",
    color: "var(--text-muted)",
    border: "none",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
    backgroundColor: "var(--bg-selected)",
  },
  ".cm-activeLine": {
    backgroundColor: "var(--bg-hover)",
  },
  ".cm-placeholder": {
    color: "var(--text-muted)",
  },
});

// Toggle a marker pair (e.g. "**" for bold, "_" for italic) around each
// selection range. If the selection is already immediately wrapped by the
// marker, the markers are removed (unwrap) instead of wrapping again. An
// empty selection just inserts the marker pair with the cursor left between
// them, ready to type.
function toggleWrap(marker: string) {
  return (view: EditorView): boolean => {
    const { state } = view;
    const changes = state.changeByRange((range) => {
      const { from, to } = range;
      const before = state.sliceDoc(Math.max(0, from - marker.length), from);
      const after = state.sliceDoc(to, Math.min(state.doc.length, to + marker.length));
      if (before === marker && after === marker) {
        return {
          changes: [
            { from: from - marker.length, to: from, insert: "" },
            { from: to, to: to + marker.length, insert: "" },
          ],
          range: EditorSelection.range(from - marker.length, to - marker.length),
        };
      }
      return {
        changes: [
          { from, insert: marker },
          { from: to, insert: marker },
        ],
        range: EditorSelection.range(from + marker.length, to + marker.length),
      };
    });
    view.dispatch(state.update(changes, { scrollIntoView: true, userEvent: "input" }));
    return true;
  };
}

const MarkdownEditor = forwardRef<MarkdownEditorHandle, MarkdownEditorProps>(function MarkdownEditor(
  { value, onChange, onPaste, placeholder, minHeight = "10rem" },
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
        // Prec.highest is required here: basicSetup bundles its own copy of
        // defaultKeymap (which already binds Mod-i to selectParentSyntax) in
        // an earlier-positioned keymap.of extension, which would otherwise
        // shadow our Mod-b/Mod-i bindings by array-order precedence alone —
        // confirmed by testing that Mod-i silently ran selectParentSyntax
        // instead of toggling italics before this was added.
        Prec.highest(
          keymap.of([
            { key: "Mod-b", run: toggleWrap("**") },
            { key: "Mod-i", run: toggleWrap("_") },
            ...defaultKeymap,
            ...historyKeymap,
          ]),
        ),
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
        minHeight,
        border: "1px solid var(--border, #333)",
        borderRadius: "var(--radius-sm, 4px)",
        overflow: "auto",
      }}
    />
  );
});

export default MarkdownEditor;
