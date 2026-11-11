import "./style.css";
import { connectWs } from "./api";
import { App } from "./layout/app";

async function boot() {
  connectWs();
  const app = new App();
  await app.mount(document.getElementById("app")!);

  // hash router: #/doc/<path> opens a doc (renderMd wikilinks + bookmarks)
  const route = () => {
    const h = location.hash;
    if (h.startsWith("#/doc/")) {
      const p = decodeURIComponent(h.slice("#/doc/".length));
      if (p) void app.resolveLink(p);
    }
  };
  window.addEventListener("hashchange", route);
  route();
}

void boot();
