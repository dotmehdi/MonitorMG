// Aperçu navigateur de l'UI avec un faux backend Tauri, sans toucher aux écrans.
// Usage : node dev/serve.js ui 4517  puis  http://127.0.0.1:4517/?s=dock  (ou ?s=desk, &empty=1, &lang=en)
const http = require("http");
const fs = require("fs");
const path = require("path");
const root = path.resolve(process.argv[2] || "ui");
const port = Number(process.argv[3] || 4517);
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml" };

http
  .createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    if (url.pathname === "/__mock.js") {
      res.writeHead(200, { "content-type": "text/javascript" });
      return res.end(fs.readFileSync(path.join(__dirname, "mock.js")));
    }
    const file = path.join(root, url.pathname === "/" ? "index.html" : url.pathname);
    if (!file.startsWith(root) || !fs.existsSync(file)) {
      res.writeHead(404);
      return res.end();
    }
    let body = fs.readFileSync(file);
    if (file.endsWith(".html")) body = body.toString().replace("<script src=", '<script src="/__mock.js"></script><script src=');
    res.writeHead(200, { "content-type": types[path.extname(file)] || "text/plain" });
    res.end(body);
  })
  .listen(port, "127.0.0.1", () => console.log(`http://127.0.0.1:${port}`));
