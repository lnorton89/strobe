"use client";

/*
 * The /play page's code editor: CodeMirror 6, themed from the site's tokens,
 * with the engine's errors drawn on their lines.
 *
 * GLSL is highlighted with the C++ grammar, which reads it well enough;
 * scene.json and script.js have their own. Switching files swaps the
 * document and keeps each file's undo history in its own state.
 */

import { useEffect, useRef } from "react";
import { basicSetup } from "codemirror";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { indentWithTab } from "@codemirror/commands";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { cpp } from "@codemirror/lang-cpp";
import { json } from "@codemirror/lang-json";
import { javascript } from "@codemirror/lang-javascript";
import { type Diagnostic, setDiagnostics } from "@codemirror/lint";
import { tags } from "@lezer/highlight";

export type LineError = { line: number; column?: number; message: string };

const theme = EditorView.theme(
  {
    "&": {
      height: "100%",
      color: "var(--text)",
      backgroundColor: "var(--ground)",
      fontSize: "13px",
    },
    ".cm-scroller": { fontFamily: "var(--font-jetbrains-mono), ui-monospace, monospace" },
    ".cm-content": { caretColor: "var(--accent)" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)" },
    "&.cm-focused": { outline: "none" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
      backgroundColor: "color-mix(in srgb, var(--accent) 22%, transparent)",
    },
    ".cm-gutters": {
      backgroundColor: "var(--panel)",
      color: "var(--label)",
      borderRight: "1px solid var(--hairline)",
    },
    ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--panel-raised) 55%, transparent)" },
    ".cm-activeLineGutter": { backgroundColor: "var(--panel-raised)", color: "var(--bright)" },
    ".cm-tooltip": {
      backgroundColor: "var(--panel-raised)",
      border: "1px solid var(--hairline-strong)",
      color: "var(--text)",
    },
    ".cm-diagnostic-error": { borderLeftColor: "var(--magenta)" },
    ".cm-lintRange-error": {
      backgroundImage: "none",
      textDecoration: "underline wavy var(--magenta)",
    },
    ".cm-panels": { backgroundColor: "var(--panel)", color: "var(--text)" },
    ".cm-searchMatch": { backgroundColor: "color-mix(in srgb, var(--violet) 35%, transparent)" },
  },
  { dark: true },
);

const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.controlKeyword, tags.modifier], color: "var(--violet)" },
  { tag: [tags.typeName, tags.standard(tags.typeName)], color: "var(--accent-text)" },
  { tag: [tags.number, tags.bool], color: "var(--magenta)" },
  { tag: [tags.string, tags.special(tags.string)], color: "#7ee0a1" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--mute)", fontStyle: "italic" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: "#8fb8ff" },
  { tag: [tags.processingInstruction, tags.macroName], color: "var(--secondary)" },
  { tag: [tags.propertyName], color: "var(--bright)" },
]);

function language(file: string): Extension {
  if (file.endsWith(".json")) return json();
  if (file.endsWith(".js")) return javascript();
  return cpp();
}

function makeState(file: string, text: string, onChange: (text: string) => void) {
  return EditorState.create({
    doc: text,
    extensions: [
      basicSetup,
      keymap.of([indentWithTab]),
      language(file),
      theme,
      syntaxHighlighting(highlight),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) onChange(update.state.doc.toString());
      }),
    ],
  });
}

export default function CodeEditor({
  file,
  text,
  errors,
  onChange,
}: {
  file: string;
  text: string;
  errors: LineError[];
  onChange: (file: string, text: string) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const states = useRef(new Map<string, EditorState>());
  const current = useRef(file);
  const started = useRef(false);
  const change = useRef(onChange);
  useEffect(() => {
    change.current = onChange;
  }, [onChange]);

  useEffect(() => {
    if (!host.current) return;
    const editor = new EditorView({ parent: host.current });
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
  }, []);

  // Swap in the file's own state (made on first visit), or follow a text
  // change made outside the editor (a scene opened, a file imported).
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const name = file;
    const fresh = () => makeState(name, text, (value) => change.current(name, value));
    if (current.current !== name || !started.current) {
      if (started.current) states.current.set(current.current, editor.state);
      current.current = name;
      started.current = true;
      const saved = states.current.get(name);
      editor.setState(saved && saved.doc.toString() === text ? saved : fresh());
      return;
    }
    // Typing round-trips through the parent unchanged; only a different
    // text means the scene changed under the editor.
    if (editor.state.doc.toString() !== text) editor.setState(fresh());
  }, [file, text]);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const doc = editor.state.doc;
    const diagnostics: Diagnostic[] = errors
      .filter((error) => error.line >= 1 && error.line <= doc.lines)
      .map((error) => {
        const line = doc.line(error.line);
        const from = Math.min(line.from + Math.max(0, (error.column ?? 1) - 1), line.to);
        return {
          from: error.column ? from : line.from,
          to: line.to,
          severity: "error",
          message: error.message,
        };
      });
    editor.dispatch(setDiagnostics(editor.state, diagnostics));
  }, [errors, file, text]);

  return <div ref={host} className="h-full min-h-0 overflow-hidden" />;
}
