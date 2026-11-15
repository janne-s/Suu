// Local static server for the Suu page (web/): `bun run web` (builds first), then open the printed URL.
const root = import.meta.dir + "/../web";
const types: Record<string, string> = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".gz": "application/octet-stream" };
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: Number(process.env.PORT ?? 5318),
  async fetch(req) {
    let path = new URL(req.url).pathname;
    if (path.endsWith("/")) path += "index.html";
    if (path.includes("..")) return new Response("no", { status: 400 });
    const file = Bun.file(root + path);
    if (!(await file.exists())) return new Response("not found", { status: 404 });
    const ext = path.slice(path.lastIndexOf("."));
    return new Response(file, { headers: { "content-type": types[ext] ?? "application/octet-stream", "cache-control": "no-store" } });
  },
});
console.log(`Suu: http://${server.hostname}:${server.port}/`);
