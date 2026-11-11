import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, Decoration, type DecorationSet } from "@codemirror/view";
import { EditorState, Compartment, type Extension, RangeSetBuilder } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { languages } from "@codemirror/language-data";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { oneDark } from "@codemirror/theme-one-dark";
import { syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import { ViewPlugin, type ViewUpdate } from "@codemirror/view";

export interface EditorOpts {
  doc: string;
  path: string;
  onDirty: (dirty: boolean, view: EditorView) => void;
  onSave: (view: EditorView) => void;
  onOpenLink: (path: string) => void;
  docsProvider: () => { path: string; title: string }[];
}

// hide trailing block ids (Obsidian ^id)
const blockIdDeco = Decoration.replace({});
const blockIdPlugin = ViewPlugin.fromClass(class {
  decorations: DecorationSet;
  constructor(view: EditorView) {
    this.decorations = this.build(view);
  }
  update(u: ViewUpdate) {
    if (u.docChanged || u.viewportChanged) this.decorations = this.build(u.view);
  }
  build(view: EditorView): DecorationSet {
    const b = new RangeSetBuilder<Decoration>();
    const re = /\s\^[A-Za-z0-9][A-Za-z0-9-]*\s*$/;
    for (const { from, to } of view.visibleRanges) {
      for (let pos = from; pos <= to; ) {
        const line = view.state.doc.lineAt(pos);
        const m = re.exec(line.text);
        if (m && m.index !== undefined) {
          b.add(line.from + m.index, line.to, blockIdDeco);
        }
        pos = line.to + 1;
        if (pos <= from) break;
      }
    }
    return b.finish();
  }
});

// Style [[wikilinks]]: hide the brackets, color the target (non-interactive DOM;
// clicks fall through to editor link handling below)
const wikiDecoPlugin = ViewPlugin.fromClass(class {
  decorations: DecorationSet;
  constructor(view: EditorView) {
    this.decorations = this.build(view);
  }
  update(u: ViewUpdate) {
    if (u.docChanged || u.viewportChanged) this.decorations = this.build(u.view);
  }
  build(view: EditorView): DecorationSet {
    const b = new RangeSetBuilder<Decoration>();
    const re = /\[\[([^\]\|#]+)(#[^\]\|]*)?(\|[^\]]*)?\]\]/g;
    for (const { from, to } of view.visibleRanges) {
      for (let pos = from; pos <= to; ) {
        const line = view.state.doc.lineAt(pos);
        let m: RegExpExecArray | null;
        re.lastIndex = 0;
        while ((m = re.exec(line.text))) {
          const start = line.from + m.index;
          const end = start + m[0].length;
          const target = m[1].trim();
          // additions must be sorted by from and non-overlapping
          b.add(start, start + 2, Decoration.replace({}));
          b.add(start + 2, end - 2, Decoration.mark({
            class: "mv-wikilink",
            attributes: { "data-wikitarget": target, style: "color:var(--accent);cursor:pointer;" },
          }));
          b.add(end - 2, end, Decoration.replace({}));
        }
        pos = line.to + 1;
        if (pos <= from) break;
      }
    }
    return b.finish();
  }
});

// Ctrl/Cmd+click a wikilink-marked range -> open
function wikiClickHandler(ext: { onOpenLink: (p: string) => void }): Extension {
  return EditorView.domEventHandlers({
    mousedown(ev, view) {
      const t = ev.target as HTMLElement;
      const wiki = t.closest(".mv-wikilink") as HTMLElement | null;
      if (wiki && (ev.ctrlKey || ev.metaKey || !window.getSelection()?.toString())) {
        const target = wiki.getAttribute("data-wikitarget");
        if (target) {
          ext.onOpenLink(target);
          return true;
        }
      }
      void view;
      return false;
    },
  });
}

// wikilink completion is added in M3 (needs the docs index provider)

export function createEditor(host: HTMLElement, opts: EditorOpts): EditorView {
  const langComp = new Compartment();
  const themeComp = new Compartment();

  const onChange = EditorView.updateListener.of((u: ViewUpdate) => {
    if (u.docChanged) opts.onDirty(true, u.view);
  });

  const exts: Extension[] = [
    lineNumbers(),
    history(),
    drawSelection(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    oneDark,
    syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
    markdown({ base: markdownLanguage, codeLanguages: languages }),
    langComp.of([]),
    themeComp.of([]),
    keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
    onChange,
    blockIdPlugin,
    wikiDecoPlugin,
    wikiClickHandler(opts),
    EditorView.lineWrapping,
  ];

  const view = new EditorView({
    state: EditorState.create({ doc: opts.doc, extensions: exts }),
    parent: host,
  });
  return view;
}

export function editorTheme(): Extension {
  return EditorView.theme({
    "&": { fontSize: "14px", background: "var(--bg)" },
    ".cm-content": { padding: "12px 0", caretColor: "var(--accent)" },
    ".cm-gutters": { background: "var(--bg)", color: "var(--text-dim)", border: "none" },
    ".cm-activeLineGutter": { background: "var(--bg-hover)" },
    ".cm-activeLine": { background: "rgba(122,162,247,.05)" },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { background: "rgba(122,162,247,.25)!" },
    ".cm-scroller": { fontFamily: 'Consolas, "Microsoft YaHei", monospace', lineHeight: "1.6" },
  });
}
