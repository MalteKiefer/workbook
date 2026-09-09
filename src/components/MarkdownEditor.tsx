import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { EditorSelection, EditorState, Prec, RangeSetBuilder } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, keymap, placeholder as placeholderExtension } from "@codemirror/view";
import type { DecorationSet, ViewUpdate } from "@codemirror/view";
import { defaultKeymap, historyKeymap, history } from "@codemirror/commands";
import { HighlightStyle, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { tags } from "@lezer/highlight";
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
  // Applied by codeBlockBackground below to every line inside a fenced code
  // block (including its ``` fence lines), so a code block reads as a
  // distinct box rather than plain text on the same background as prose —
  // this is what was missing before (basicSetup's default highlight style
  // only colors tokens, it never gives a block its own background).
  ".cm-code-block-line": {
    backgroundColor: "var(--bg-hover)",
  },
});

// basicSetup ships its own default syntax highlighting, tuned for a generic
// light-ish background — several markdown token colors (the ```fence marks,
// the language tag, inline `code`) ended up low-contrast/hard to read against
// this app's dark editor background. Re-themed against this app's own design
// tokens so it stays legible (and correctly re-themes itself) in both the
// dark and light palettes in theme.css. Tag-to-node mapping confirmed against
// @lezer/markdown's own styleTags config: CodeMark = the ``` /# /- /> /**
// marker characters themselves, monospace = InlineCode/CodeText (covers both
// `inline code` and fenced code block content), labelName = the CodeInfo
// language tag (e.g. "bash" in ```bash).
const markdownHighlightStyle = HighlightStyle.define([
  { tag: [tags.heading1, tags.heading2, tags.heading3, tags.heading4, tags.heading5, tags.heading6], color: "var(--text-primary)", fontWeight: "700" },
  { tag: tags.strong, color: "var(--text-primary)", fontWeight: "700" },
  { tag: tags.emphasis, color: "var(--text-primary)", fontStyle: "italic" },
  { tag: tags.strikethrough, color: "var(--text-muted)", textDecoration: "line-through" },
  { tag: tags.link, color: "var(--accent)", textDecoration: "underline" },
  { tag: tags.url, color: "var(--accent)" },
  { tag: tags.quote, color: "var(--text-secondary)", fontStyle: "italic" },
  { tag: tags.monospace, color: "var(--text-primary)" },
  { tag: tags.labelName, color: "var(--text-secondary)", fontStyle: "italic" },
  { tag: tags.processingInstruction, color: "var(--text-muted)" },
]);

// Gives every line inside a fenced code block (```...```) the `.cm-code-block-line`
// background class from editorTheme above, so the block reads as one visually
// distinct box (the ``` fence lines included) instead of blending into
// surrounding prose. basicSetup's markdown highlighting only colors
// individual tokens -- it has no concept of "this whole block gets a
// background" -- so this has to be a line decoration built by hand from the
// syntax tree, recomputed whenever the document or viewport changes (the
// same pattern CodeMirror's own docs use for line-based decorations).
function computeCodeBlockLineDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const decoratedLines = new Set<number>();
  syntaxTree(view.state).iterate({
    enter: (node) => {
      if (node.name !== "FencedCode") return;
      const startLine = view.state.doc.lineAt(node.from).number;
      const endLine = view.state.doc.lineAt(node.to).number;
      for (let lineNumber = startLine; lineNumber <= endLine; lineNumber++) {
        decoratedLines.add(lineNumber);
      }
    },
  });
  for (const lineNumber of [...decoratedLines].sort((a, b) => a - b)) {
    const line = view.state.doc.line(lineNumber);
    builder.add(line.from, line.from, Decoration.line({ class: "cm-code-block-line" }));
  }
  return builder.finish();
}

const codeBlockBackground = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = computeCodeBlockLineDecorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = computeCodeBlockLineDecorations(update.view);
      }
    }
  },
  { decorations: (instance) => instance.decorations },
);

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
        syntaxHighlighting(markdownHighlightStyle),
        codeBlockBackground,
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
