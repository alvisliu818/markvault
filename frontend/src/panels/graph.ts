import { api } from "../api";
import { el, toast } from "../ui";

/**
 * Global link graph panel (M11): force-directed layout on a canvas.
 * Hand-rolled simulation — repulsion (O(n²), fine for note-sized vaults),
 * edge springs, weak centering, alpha decay. Click a node to open the doc;
 * drag pins a node; wheel zooms; empty canvas drag pans.
 */

interface GraphNode {
  id: string;
  title: string;
  notebook: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  degree: number;
}
interface GraphEdge {
  a: number; // node index
  b: number;
}

const REPULSION = 4200;
const DAMPING = 0.86;
const CENTER_PULL = 0.008;

export class GraphPanel {
  private canvas: HTMLCanvasElement;
  private statEl: HTMLElement;
  private nodes: GraphNode[] = [];
  private edges: GraphEdge[] = [];
  private alpha = 0;
  private scale = 1;
  private panX = 0;
  private panY = 0;
  private dragIdx = -1;
  private panDrag = false;
  private lastMouse = { x: 0, y: 0 };
  private hoverIdx = -1;
  private openDoc: (path: string, title: string) => void;
  private raf = 0;
  private cw = 0;
  private ch = 0;

  constructor(container: HTMLElement, openDoc: (path: string, title: string) => void) {
    this.openDoc = openDoc;
    const head = el("div", "panel-head");
    const refreshBtn = el("button", "btn", "刷新图谱");
    refreshBtn.onclick = () => void this.load();
    this.statEl = el("span", "graph-stat", "");
    head.append(refreshBtn, this.statEl);
    this.canvas = el("canvas", "graph-canvas") as HTMLCanvasElement;
    container.append(head, this.canvas);

    this.canvas.addEventListener("mousedown", (e) => this.onDown(e));
    window.addEventListener("mousemove", this.onMove);
    window.addEventListener("mouseup", this.onUp);
    this.canvas.addEventListener("wheel", (e) => {
      e.preventDefault();
      const f = e.deltaY < 0 ? 1.15 : 1 / 1.15;
      this.scale = Math.min(4, Math.max(0.2, this.scale * f));
      this.wake();
    });
    this.canvas.addEventListener("dblclick", () => {
      this.scale = 1;
      this.panX = 0;
      this.panY = 0;
      this.wake();
    });
    // The panel mounts inside a hidden dock page (canvas rect = 0): load and
    // simulate only when it first becomes visible, and re-wake on re-entry.
    let loadedOnce = false;
    const io = new IntersectionObserver((entries) => {
      if (entries.some((en) => en.isIntersecting)) {
        if (!loadedOnce) {
          loadedOnce = true;
          void this.load();
        } else {
          this.alpha = Math.max(this.alpha, 0.3);
          this.wake();
        }
      }
    });
    io.observe(this.canvas);
  }

  private toWorld(mx: number, my: number): { x: number; y: number } {
    const rect = this.canvas.getBoundingClientRect();
    return {
      x: (mx - rect.left - this.panX) / this.scale,
      y: (my - rect.top - this.panY) / this.scale,
    };
  }

  private hitTest(mx: number, my: number): number {
    const w = this.toWorld(mx, my);
    let best = -1;
    let bestD = Infinity;
    for (let i = 0; i < this.nodes.length; i++) {
      const n = this.nodes[i];
      const d = (n.x - w.x) ** 2 + (n.y - w.y) ** 2;
      const r = 6 + 2 * Math.min(3, Math.sqrt(n.degree));
      if (d < r * r && d < bestD) {
        best = i;
        bestD = d;
      }
    }
    return best;
  }

  private onDown(e: MouseEvent) {
    const idx = this.hitTest(e.clientX, e.clientY);
    if (idx >= 0) {
      this.dragIdx = idx;
    } else {
      this.panDrag = true;
    }
    this.lastMouse = { x: e.clientX, y: e.clientY };
  }

  private onMove = (e: MouseEvent) => {
    if (this.dragIdx >= 0) {
      const w = this.toWorld(e.clientX, e.clientY);
      const n = this.nodes[this.dragIdx];
      n.x = w.x;
      n.y = w.y;
      n.vx = 0;
      n.vy = 0;
      this.alpha = Math.max(this.alpha, 0.15);
      this.wake();
    } else if (this.panDrag) {
      this.panX += e.clientX - this.lastMouse.x;
      this.panY += e.clientY - this.lastMouse.y;
      this.lastMouse = { x: e.clientX, y: e.clientY };
      this.wake();
    } else {
      const idx = this.hitTest(e.clientX, e.clientY);
      if (idx !== this.hoverIdx) {
        this.hoverIdx = idx;
        this.canvas.style.cursor = idx >= 0 ? "pointer" : "default";
        this.wake();
      }
    }
  };

  private onUp = (e: MouseEvent) => {
    if (this.dragIdx >= 0) {
      const moved = Math.abs(e.clientX - this.lastMouse.x) + Math.abs(e.clientY - this.lastMouse.y);
      if (moved < 3) {
        const n = this.nodes[this.dragIdx];
        this.openDoc(n.id, n.title);
      }
      this.dragIdx = -1;
    }
    this.panDrag = false;
  };

  private css(name: string, fallback: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
  }

  private wake() {
    if (!this.raf) this.raf = requestAnimationFrame(() => this.tick());
  }

  private tick() {
    this.raf = 0;
    this.step();
    this.draw();
    if (this.alpha > 0.02) this.wake();
  }

  private step() {
    const n = this.nodes;
    const N = n.length;
    // spring length adapts to the panel so small docks stay readable
    const spring = Math.max(50, Math.min(130, Math.min(this.canvas.width, this.canvas.height) / 4));
    if (N > 1) {
      for (let i = 0; i < N; i++) {
        const a = n[i];
        for (let j = i + 1; j < N; j++) {
          const b = n[j];
          let dx = a.x - b.x;
          let dy = a.y - b.y;
          let d2 = dx * dx + dy * dy;
          if (d2 < 1) {
            dx = (Math.random() - 0.5) * 2;
            dy = (Math.random() - 0.5) * 2;
            d2 = 4;
          }
          const f = (REPULSION * this.alpha) / d2;
          const d = Math.sqrt(d2);
          const fx = (dx / d) * f;
          const fy = (dy / d) * f;
          a.vx += fx;
          a.vy += fy;
          b.vx -= fx;
          b.vy -= fy;
        }
      }
    }
    for (const e of this.edges) {
      const a = n[e.a];
      const b = n[e.b];
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const d = Math.max(1, Math.hypot(dx, dy));
      const f = (d - spring) * 0.03 * Math.max(this.alpha, 0.25);
      const fx = (dx / d) * f;
      const fy = (dy / d) * f;
      a.vx += fx;
      a.vy += fy;
      b.vx -= fx;
      b.vy -= fy;
    }
    for (const v of n) {
      // pull toward canvas center (0,0 is the top-left corner, not the middle)
      if (this.cw > 0) {
        v.vx += (this.cw / 2 - v.x) * CENTER_PULL;
        v.vy += (this.ch / 2 - v.y) * CENTER_PULL;
      }
      v.vx *= DAMPING;
      v.vy *= DAMPING;
      v.x += Math.max(-12, Math.min(12, v.vx));
      v.y += Math.max(-12, Math.min(12, v.vy));
      // soft walls: keep nodes inside the visible canvas
      const m = 14;
      if (this.cw > 0) {
        v.x = Math.max(m, Math.min(this.cw - m, v.x));
        v.y = Math.max(m, Math.min(this.ch - m, v.y));
      }
    }
    this.alpha *= 0.985;
  }

  private fitCanvas() {
    const rect = this.canvas.getBoundingClientRect();
    const w = Math.max(50, Math.floor(rect.width));
    const h = Math.max(50, Math.floor(rect.height));
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    this.cw = w;
    this.ch = h;
    return { w, h };
  }

  private draw() {
    const { w, h } = this.fitCanvas();
    const ctx = this.canvas.getContext("2d");
    if (!ctx) return;
    const edge = this.css("--text-dim", "#888") + "55";
    const accent = this.css("--accent", "#7aa2f7");
    const text = this.css("--text", "#dcdfe4");
    ctx.clearRect(0, 0, w, h);
    ctx.save();
    ctx.translate(this.panX, this.panY);
    ctx.scale(this.scale, this.scale);

    // edges
    ctx.strokeStyle = edge;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (const e of this.edges) {
      const a = this.nodes[e.a];
      const b = this.nodes[e.b];
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
    }
    ctx.stroke();

    // highlight hover node edges
    if (this.hoverIdx >= 0) {
      ctx.strokeStyle = accent;
      ctx.beginPath();
      for (const e of this.edges) {
        const a = this.nodes[e.a];
        const b = this.nodes[e.b];
        if (e.a === this.hoverIdx) {
          ctx.moveTo(a.x, a.y);
          ctx.lineTo(b.x, b.y);
        } else if (e.b === this.hoverIdx) {
          ctx.moveTo(b.x, b.y);
          ctx.lineTo(a.x, a.y);
        }
      }
      ctx.stroke();
    }

    // nodes + labels
    ctx.textBaseline = "middle";
    for (const v of this.nodes) {
      const r = 4 + Math.min(4, Math.sqrt(v.degree));
      ctx.fillStyle = v.degree > 0 ? accent : this.css("--text-dim", "#888");
      ctx.beginPath();
      ctx.arc(v.x, v.y, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = text;
      ctx.font = `${this.hoverIdx >= 0 && this.nodes[this.hoverIdx] === v ? "600 " : ""}10px system-ui`;
      ctx.fillText(v.title, v.x + r + 3, v.y);
    }
    ctx.restore();
  }

  private async load() {
    try {
      const g = await api.getGraph();
      const byId = new Map<string, number>();
      // fit canvas first so the initial layout fits the visible area
      const { w, h } = this.fitCanvas();
      const spread = Math.max(40, Math.min(w, h) / 3);
      this.nodes = g.nodes.map((n, i) => {
        byId.set(n.id, i);
        const ang = (i / Math.max(1, g.nodes.length)) * Math.PI * 2;
        const rad = spread * (0.4 + ((i * 7) % 10) / 10);
        return {
          ...n,
          x: w / 2 + Math.cos(ang) * rad,
          y: h / 2 + Math.sin(ang) * rad,
          vx: 0,
          vy: 0,
          degree: 0,
        };
      });
      this.edges = [];
      for (const l of g.links) {
        const a = byId.get(l.source);
        const b = byId.get(l.target);
        if (a === undefined || b === undefined) continue;
        this.edges.push({ a, b });
        this.nodes[a].degree++;
        this.nodes[b].degree++;
      }
      this.statEl.textContent = `${this.nodes.length} 节点 · ${this.edges.length} 链接`;
      this.scale = 1;
      this.panX = 0;
      this.panY = 0;
      this.alpha = 1;
      this.wake();
    } catch (e) {
      this.statEl.textContent = "";
      toast("图谱加载失败: " + (e as Error).message);
    }
  }
}
