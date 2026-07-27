// Runs the real WebAssembly module under Node, because `cargo check --target
// wasm32` only proves it compiles. It compiled happily while the parser tried to
// spawn a thread — unsupported on wasm — and aborted on the first manifest.
//
// Usage: wasm-bindgen … --target nodejs --out-dir pkg-node && node smoke.cjs
const { evaluate, highlight, format } = require("./pkg-node/aura_wasm.js");

let failures = 0;
const check = (name, cond, extra = "") => {
  console.log(`${cond ? "  OK  " : " FAIL "} ${name}${cond ? "" : " :: " + extra}`);
  if (!cond) failures++;
};

// 1. The schemas/enums example, exactly as the page ships it
const main = `enum Tier
  "frontend"
  "backend"
end

type Service
  name: String
  tier: Tier
  replicas: Int = 2
end

is_prod = env("APP_ENV", "dev") == "production"

api: new Service
  name:     "checkout-api"
  tier:     "backend"
  replicas: is_prod ? 6 : 2
end
`;
let r = JSON.parse(evaluate({ "main.aura": main }, "main.aura", "json", false, { APP_ENV: "production" }));
check("evaluates with env override", r.ok, JSON.stringify(r));
check("env() reached the browser-side map", r.ok && r.output.includes('"replicas": 6'), r.output);

// 2. YAML and TOML formats
for (const f of ["yaml", "toml"]) {
  const out = JSON.parse(evaluate({ "main.aura": main }, "main.aura", f, false, { APP_ENV: "dev" }));
  check(`format ${f}`, out.ok && out.output.length > 0, JSON.stringify(out).slice(0, 200));
}

// 3. Multi-file imports between buffers
const files = {
  "main.aura": 'import "lib.aura" as lib\nport: lib.default_port\n',
  "lib.aura": "default_port: 8080\n",
};
r = JSON.parse(evaluate(files, "main.aura", "json", false, {}));
check("imports across buffers", r.ok && r.output.includes("8080"), JSON.stringify(r));

// 4. The capability demo: root granted, import still denied
const cap = {
  "main.aura": 'import "dependency.aura" as dep\ndata: dep.contents\n',
  "dependency.aura": 'contents: read_file("secrets.txt")\n',
  "secrets.txt": "s3cr3t\n",
};
r = JSON.parse(evaluate(cap, "main.aura", "json", true, {}));
check("import denied even with allow_read", !r.ok && r.diagnostics.some(d => d.code === "E0310"),
      JSON.stringify(r));
const d = r.ok ? null : r.diagnostics[0];
check("diagnostic names the buffer and a line", d && d.file === "dependency.aura" && d.line > 0,
      JSON.stringify(d));

// 5. Highlighting: byte offsets over non-ASCII must stay on boundaries
const src = '# комментарий\ndef f()\n  s: "строка"\nend\n';
const spans = JSON.parse(highlight(src));
const bytes = Buffer.from(src, "utf8");
check("highlight returns spans", spans.length > 0);
let ordered = true, inRange = true;
let prev = 0;
for (const sp of spans) {
  if (sp.s < prev) ordered = false;
  if (sp.e > bytes.length) inRange = false;
  prev = sp.e;
}
check("spans ordered and in range", ordered && inRange, JSON.stringify(spans));
const cmt = spans.find(s => s.k === "cmt");
check("comment span decodes correctly",
      cmt && bytes.subarray(cmt.s, cmt.e).toString("utf8") === "# комментарий",
      cmt && bytes.subarray(cmt.s, cmt.e).toString("utf8"));

// 6. Half-typed input must not throw
check("unterminated string is safe", highlight('x: "unterm') === "[]");
r = JSON.parse(evaluate({ "main.aura": 'x: "unterm' }, "main.aura", "json", false, {}));
check("unterminated string reports, not throws", !r.ok && r.diagnostics.length > 0);

// 7. Formatting is the canonicalisation `aura fmt` performs, and it must never
// destroy a buffer that does not lex — the button is one keystroke away from a
// half-typed string.
const messy = 'domain "d"\n      x: 1\nend\n';
check(
  "format canonicalises",
  format(messy) === 'domain "d"\n  x: 1\nend\n',
  JSON.stringify(format(messy)),
);
check("format is idempotent", format(format(messy)) === format(messy));
check("format leaves unlexable input alone", format('x: "unterm') === 'x: "unterm');

console.log(failures ? `\n${failures} FAILED` : "\nвсе проверки прошли");
process.exit(failures ? 1 : 0);
