import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, Decoration, type DecorationSet, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import { EditorState, Compartment, Facet, type Extension, RangeSetBuilder } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { languages } from "@codemirror/language-data";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { oneDark } from "@codemirror/theme-one-dark";
import { syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import { autocompletion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";

export interface EditorOpts {
  doc: string;
  path: string;
  onDirty: (dirty: boolean, view: EditorView) => void;
  onSave: (view: EditorView) => void;
  onOpenLink: (path: string, anchor?: string) => void;
  /** returns true when target resolves (used for unresolved-link styling is render-time; runtime click fallback) */
  docsProvider: () => { path: string; title: string }[];
  /** paste handler for images; return true when handled */
  onPasteFile?: (file: File, view: EditorView) => boolean;
}

// ---- line-level semantic decorations (live preview) ----

function headingDeco(level: number) {
  return Decoration.line({
    class: `mv-h-line mv-h${level}`,
    attributes: { "data-level": String(level) },
  });
}

const headingLinePlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = this.build(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged) this.decorations = this.build(u.view);
    }
    build(view: EditorView): DecorationSet {
      const b = new RangeSetBuilder<Decoration>();
      for (const { from, to } of view.visibleRanges) {
        for (let pos = from; pos <= to; ) {
          const line = view.state.doc.lineAt(pos);
          const m = /^(#{1,6})\s+/.exec(line.text);
          if (m) {
            b.add(line.from, line.from, headingDeco(m[1].length));
          }
          pos = line.to + 1;
          if (pos <= from) break;
        }
      }
      return b.finish();
    }
  }
  , { decorations: (v) => v.decorations }
);

// ---- ==highlight== inline decoration ----

const highlightMark = Decoration.mark({ class: "mv-highlight" });
const highlightPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = this.build(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged) this.decorations = this.build(u.view);
    }
    build(view: EditorView): DecorationSet {
      const b = new RangeSetBuilder<Decoration>();
      const re = /==([^=\n]+)==/g;
      for (const { from, to } of view.visibleRanges) {
        for (let pos = from; pos <= to; ) {
          const line = view.state.doc.lineAt(pos);
          let m: RegExpExecArray | null;
          re.lastIndex = 0;
          while ((m = re.exec(line.text))) {
            b.add(line.from + m.index, line.from + m.index + m[0].length, highlightMark);
          }
          pos = line.to + 1;
          if (pos <= from) break;
        }
      }
      return b.finish();
    }
  }
  , { decorations: (v) => v.decorations }
);

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
}, { decorations: (v) => v.decorations });

// ---- wikilinks: resolved = accent, unresolved = dashed; brackets hidden ----

const wikiResolved = Decoration.mark({
  class: "mv-wikilink",
  attributes: { style: "color:var(--accent);cursor:pointer;" },
});
const wikiUnresolved = Decoration.mark({
  class: "mv-wikilink mv-wikilink-unresolved",
  attributes: { style: "color:var(--text-dim);cursor:pointer;" },
});

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
          const resolved = resolveTarget(target, view.state.facet(docsFacet));
          const deco = resolved ? wikiResolved : wikiUnresolved;
          // additions must be sorted by from and non-overlapping
          b.add(start, start + 2, Decoration.replace({}));
          b.add(start + 2, end - 2, deco);
          b.add(end - 2, end, Decoration.replace({}));
        }
        pos = line.to + 1;
        if (pos <= from) break;
      }
    }
    return b.finish();
  }
}, { decorations: (v) => v.decorations });

function resolveTarget(target: string, docs: { path: string; title: string }[]): boolean {
  const t = target.replace(/^\[\[|\]\]$/g, "").trim();
  const withMd = t.endsWith(".md") ? t : t + ".md";
  return docs.some(
    (d) =>
      d.path === withMd ||
      d.path === t ||
      d.title === t ||
      d.path.split("/").pop()?.replace(/\.md$/, "") === t
  );
}

// doc index facet: updated per transaction by the tab manager
export const docsFacet = Facet.define<{ path: string; title: string }[], { path: string; title: string }[]>({
  combine: (values) => (values.length ? values[values.length - 1] : []),
});

// Ctrl/Cmd+click (or plain click when no selection) a wikilink -> open
function wikiClickHandler(ext: { onOpenLink: (p: string, anchor?: string) => void }): Extension {
  return EditorView.domEventHandlers({
    mousedown(ev, view) {
      const t = ev.target as HTMLElement;
      const wiki = t.closest(".mv-wikilink") as HTMLElement | null;
      if (wiki && (ev.ctrlKey || ev.metaKey || !window.getSelection()?.toString())) {
        // recover full link text under cursor (decoration hides brackets)
        const pos = view.posAtCoords({ x: ev.clientX, y: ev.clientY });
        if (pos == null) return false;
        const line = view.state.doc.lineAt(pos);
        const re = /\[\[([^\]\|#]+)(#[^\]\|]*)?(\|[^\]]*)?\]\]/g;
        let m: RegExpExecArray | null;
        while ((m = re.exec(line.text))) {
          const s = line.from + m.index;
          const e = s + m[0].length;
          if (pos >= s && pos <= e) {
            ext.onOpenLink(m[1].trim(), m[2]?.replace(/^#/, "") || undefined);
            return true;
          }
        }
      }
      return false;
    },
  });
}

// ---- image paste widget ----

class ImgWidget extends WidgetType {
  constructor(readonly src: string) {
    super();
  }
  eq(other: ImgWidget) {
    return other.src === this.src;
  }
  toDOM() {
    const wrap = document.createElement("span");
    const img = document.createElement("img");
    img.src = this.src;
    img.className = "mv-inline-img";
    wrap.appendChild(img);
    return wrap;
  }
}

const inlineImgPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = this.build(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged) this.decorations = this.build(u.view);
    }
    build(view: EditorView): DecorationSet {
      const b = new RangeSetBuilder<Decoration>();
      const re = /!\[[^\]]*\]\(([^)\s]+)\)/g;
      const docPath = view.state.facet(docPathFacet);
      for (const { from, to } of view.visibleRanges) {
        for (let pos = from; pos <= to; ) {
          const line = view.state.doc.lineAt(pos);
          let m: RegExpExecArray | null;
          re.lastIndex = 0;
          // whole-line image only (paragraph-ish); avoids nesting inside sentences
          if (/^!\[[^\]]*\]\(([^)\s]+)\)\s*$/.test(line.text.trim())) {
            m = re.exec(line.text.trim());
            if (m) {
              const src = m[1];
              const abs = src.startsWith("http") || src.startsWith("data:")
                ? src
                : "/vault/" + (src.startsWith("/")
                    ? src.slice(1)
                    : docPath.split("/").slice(0, -1).join("/") + "/" + src);
              const lineStart = line.from + line.text.indexOf("![");
              b.add(
                lineStart,
                line.to,
                Decoration.replace({ widget: new ImgWidget(abs) })
              );
            }
          }
          pos = line.to + 1;
          if (pos <= from) break;
        }
      }
      return b.finish();
    }
  }
  , { decorations: (v) => v.decorations }
);

// ---- wikilink autocomplete: [[query -> doc list ----

function wikilinkCompletions(): Extension {
  return autocompletion({
    override: [wikiSource],
    activateOnTyping: true,
    icons: false,
  });
}

function wikiSource(ctx: CompletionContext): CompletionResult | null {
  const before = ctx.matchBefore(/\[\[([^\]\n]*)$/);
  if (!before) return null;
  const docs = ctx.state.facet(docsFacet);
  const typed = before.text.slice(2);
  const q = typed.toLowerCase();
  const options = docs
    .filter((d) => {
      const title = d.title.toLowerCase();
      const base = (d.path.split("/").pop()?.replace(/\.md$/, "") ?? "").toLowerCase();
      return !q || title.includes(q) || base.includes(q) || d.path.toLowerCase().includes(q);
    })
    .slice(0, 12)
    .map((d) => {
      const base = d.path.split("/").pop()?.replace(/\.md$/, "") ?? d.title;
      const label = d.title || base;
      return {
        label,
        detail: d.path,
        type: "doc",
        apply: label + "]]",
      };
  });
  if (options.length === 0) return null;
  return {
    from: before.from + 2,
    options,
    validFor: /^[^\]\n]*$/,
  };
}

export const docPathFacet = Facet.define<string, string>({
  combine: (values) => values[values.length - 1] ?? "",
});

export interface EditorHandle {
  view: EditorView;
  docsComp: Compartment;
  pathComp: Compartment;
}

export function createEditor(host: HTMLElement, opts: EditorOpts): EditorHandle {
  const langComp = new Compartment();
  const themeComp = new Compartment();
  const docsComp = new Compartment();
  const pathComp = new Compartment();

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
    docsComp.of(docsFacet.of(opts.docsProvider())),
    pathComp.of(docPathFacet.of(opts.path)),
    keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
    wikilinkCompletions(),
    onChange,
    headingLinePlugin,
    highlightPlugin,
    blockIdPlugin,
    wikiDecoPlugin,
    inlineImgPlugin,
    wikiClickHandler(opts),
    EditorView.lineWrapping,
    EditorView.domEventHandlers({
      paste(ev, view) {
        const files = Array.from(ev.clipboardData?.files ?? []);
        if (files.length && opts.onPasteFile) {
          for (const f of files) {
            if (opts.onPasteFile(f, view)) ev.preventDefault();
          }
          return files.length > 0;
        }
        return false;
      },
    }),
  ];

  const view = new EditorView({
    state: EditorState.create({ doc: opts.doc, extensions: exts }),
    parent: host,
  });
  return { view, docsComp, pathComp };
}

/** Push updated doc index / path into a live editor. */
export function updateEditorContext(
  view: EditorView,
  docsComp: Compartment,
  docs: { path: string; title: string }[],
  pathComp: Compartment | null,
  path?: string
) {
  const effects: any[] = [docsComp.reconfigure(docsFacet.of(docs))];
  if (pathComp) {
    effects.push(
      pathComp.reconfigure(docPathFacet.of(path !== undefined ? path : view.state.facet(docPathFacet)))
    );
  }
  view.dispatch({ effects });
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
