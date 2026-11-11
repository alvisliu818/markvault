// Kernel API client — SiYuan-style {code, msg, data} envelopes over POST JSON.

export interface ApiResp<T = any> {
  code: number;
  msg: string;
  data: T;
}

export async function post<T = any>(path: string, body?: unknown): Promise<T> {
  const r = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  const j: ApiResp<T> = await r.json();
  if (j.code !== 0) throw new Error(j.msg || `api error ${j.code}`);
  return j.data;
}

export async function getRaw(path: string, body?: unknown): Promise<Response> {
  return fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
}

export async function readText(path: string): Promise<string> {
  const r = await getRaw("/api/file/getFile", { path });
  if (!r.ok) throw new Error(`getFile ${path}: ${r.status}`);
  return r.text();
}

/** POST JSON (non-envelope endpoints like export download). Returns raw Response. */
export async function postRaw(path: string, body: unknown): Promise<Response> {
  return fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
}

export async function readBinary(path: string): Promise<ArrayBuffer> {
  const r = await getRaw("/api/file/getFile", { path });
  if (!r.ok) throw new Error(`getFile ${path}: ${r.status}`);
  return r.arrayBuffer();
}

// ---- domains ----

export interface Notebook {
  id: string;
  name: string;
  docCount: number;
}

export interface FileEntry {
  path: string;
  name: string;
  title?: string;
  type: "folder" | "doc";
  subFileCount?: number;
  size?: number;
  updated?: string;
}

export interface SearchRow {
  path: string;
  title: string;
  hpath: string;
  notebook: string;
  snippet: string;
}

export interface BacklinkLink {
  path: string;
  raw: string;
  anchor: string | null;
}

export interface BacklinksData {
  path: string;
  links: BacklinkLink[];
  unresolved: { raw: string; anchor: string | null }[];
  backlinks: { path: string }[];
  blockBacklinks?: { path: string; anchor: string | null }[];
  mentions: { path: string }[];
}

export interface OutlineItem {
  blockId: string | null;
  type: string;
  content: string;
  line: number;
}

export interface VtNode {
  id: string;
  path: string;
  name: string;
  weight: number;
  isRoot: boolean;
  children: VtNode[];
}

export interface MountItem {
  uid: string;
  kind: "file" | "doc" | "notebook";
  name: string;
  path: string;
  children: MountItem[];
}

export interface Bookmark {
  url: string;
  title: string;
  created: string;
  tags: string[];
}

export const api = {
  // bookmark
  bookmarkList: () => post<Bookmark[]>("/api/bookmark/list"),
  bookmarkAdd: (url: string, title: string) => post<Bookmark[]>("/api/bookmark/add", { url, title }),
  bookmarkRemove: (url: string) => post<Bookmark[]>("/api/bookmark/remove", { url }),
  // clip
  clipUrl: (url: string, notebook: string, tags: string[] = []) =>
    post<{ path: string; title: string }>("/api/clip/clipUrl", { url, notebook, tags }),

  // system
  version: () => post("/api/system/version"),
  getConf: () => post("/api/system/getConf"),
  setConf: (patch: Record<string, unknown>) => post("/api/system/setConf", { patch }),
  // notebook
  lsNotebooks: () => post<{ notebooks: Notebook[] }>("/api/notebook/lsNotebooks"),
  createNotebook: (name: string) => post("/api/notebook/createNotebook", { name }),
  renameNotebook: (id: string, name: string) => post("/api/notebook/renameNotebook", { id, name }),
  removeNotebook: (id: string) => post("/api/notebook/removeNotebook", { id: id }),
  // filetree
  listDocsByPath: (notebook: string, path: string) =>
    post<{ files: FileEntry[] }>("/api/filetree/listDocsByPath", { notebook, path }),
  listDocTree: () => post<{ docs: { path: string; title: string; notebook: string; hpath: string }[] }>(
    "/api/filetree/listDocTree"
  ),
  createDocWithMd: (notebook: string, path: string, markdown: string) =>
    post<{ path: string }>("/api/filetree/createDocWithMd", { notebook, path, markdown }),
  renameDoc: (path: string, title: string) => post("/api/filetree/renameDoc", { path, title }),
  removeDoc: (path: string) => post("/api/filetree/removeDoc", { path }),
  moveDocs: (fromPaths: string[], toPath: string) =>
    post("/api/filetree/moveDocs", { fromPaths, toPath }),
  searchDocs: (k: string) => post<{ docs: { path: string; title: string; hpath: string; notebook: string }[] }>(
    "/api/filetree/searchDocs", { k }
  ),
  // file
  putFile: (path: string, data: string) => post("/api/file/putFile", { path, data }),
  putFileBinary: (path: string, dataBase64: string) => post("/api/file/putFile", { path, dataBase64 }),
  removeFile: (path: string) => post("/api/file/removeFile", { path }),
  renameFile: (path: string, newPath: string) => post("/api/file/renameFile", { path, newPath }),
  readDir: (path: string) => post<{ entries: { name: string; isDir: boolean; size: number; mtime: string }[] }>(
    "/api/file/readDir", { path }
  ),
  // block / attr / outline
  getDocInfo: (path: string) => post("/api/block/getDocInfo", { path }),
  getAttrs: (path: string) => post<Record<string, unknown>>("/api/attr/getBlockAttrs", { path }),
  setAttrs: (path: string, attrs: Record<string, unknown>) =>
    post("/api/attr/setBlockAttrs", { path, attrs }),
  getOutline: (path: string) => post<{ outline: OutlineItem[] }>("/api/outline/getDocOutline", { path }),
  // search
  fullTextSearch: (query: string) => post<SearchRow[]>("/api/search/fullTextSearchBlock", { query }),
  // refs
  getBacklinks: (path: string) => post<BacklinksData>("/api/ref/getBacklinks", { path }),
  // format
  renderMd: (md: string, path: string) =>
    post<{ content: string }>("/api/format/renderMd", { md, path, rewriteLinks: true }),
  // vt
  getRelationTree: () =>
    post<{ roots: string[]; tree: VtNode[]; settings: Record<string, unknown> }>("/api/vt/getRelationTree"),
  addRoot: (path: string) => post("/api/vt/addRoot", { path }),
  removeRoot: (path: string) => post("/api/vt/removeRoot", { path }),
  getMountTree: () => post<{ mountRoots: MountItem[] }>("/api/vt/getMountTree"),
  setMountTree: (mountRoots: MountItem[]) => post("/api/vt/setMountTree", { mountRoots }),
  setNodeOrder: (parentKey: string, orderedPaths: string[]) =>
    post("/api/vt/setNodeOrder", { parentKey, orderedPaths }),
  moveNode: (path: string, newParent: string) =>
    post("/api/vt/moveNode", { path, newParent }),
  getVtSettings: () =>
    post<{ settings: { sortMethod: string; caseSensitive: boolean; defaultExpandLevel: number; includePhysicalSubtree: boolean } }>(
      "/api/vt/getSettings"
    ),
  setVtSettings: (settings: Record<string, unknown>) => post("/api/vt/setSettings", { settings }),
  // storage
  getVal: <T>(key: string) => post<T>("/api/storage/getVal", { key }),
  setVal: (key: string, val: unknown) => post("/api/storage/setVal", { key, val }),
};

// ---- WebSocket push ----

type WsHandler = (cmd: string, data: unknown) => void;

const handlers = new Set<WsHandler>();
let ws: WebSocket | null = null;
let wsRetry = 0;

export function onWsPush(h: WsHandler): () => void {
  handlers.add(h);
  return () => handlers.delete(h);
}

export function connectWs() {
  if (ws) return;
  const proto = location.protocol === "https:" ? "wss" : "ws";
  ws = new WebSocket(`${proto}://${location.host}/ws`);
  ws.onmessage = (ev) => {
    try {
      const env = JSON.parse(ev.data);
      handlers.forEach((h) => h(env.cmd, env.data));
    } catch {
      /* ignore */
    }
  };
  ws.onclose = () => {
    ws = null;
    setTimeout(connectWs, Math.min(1000 * ++wsRetry, 10000));
  };
  ws.onopen = () => {
    wsRetry = 0;
  };
}
