// Aura playground.
//
// Everything runs locally: the WebAssembly module is the real interpreter, so
// what you see here is what `aura eval` produces. Nothing is sent anywhere.

import init, { evaluate, format, highlight } from "./pkg/aura_wasm.js";

// ---------------------------------------------------------------- examples

const EXAMPLES = {
  "Schemas and enums": {
    entry: "main.aura",
    allowRead: false,
    env: "APP_ENV=production",
    files: {
      "main.aura": `enum Tier
  "frontend"
  "backend"
end

type Service
  name: String
  tier: Tier
  replicas: Int = 2
  port: Int     = 8080
end

is_prod = env("APP_ENV", "dev") == "production"

domain "checkout"
  api_replicas    = is_prod ? 6 : 2
  worker_replicas = is_prod ? 3 : 1

  assert api_replicas >= worker_replicas, "api must not be smaller than worker"

  api: new Service
    name:     "checkout-api"
    tier:     "backend"
    replicas: api_replicas
  end

  worker: new Service
    name:     "checkout-worker"
    tier:     "backend"
    replicas: worker_replicas
  end
end
`,
    },
  },

  "Imports across files": {
    entry: "main.aura",
    allowRead: false,
    env: "",
    files: {
      "main.aura": `import "lib.aura" as lib

# A property crosses the module boundary; a \`=\` binding does not.
port: lib.default_port
api:  lib.label("checkout")
`,
      "lib.aura": `# Only \`pub\` items and properties are visible to importers (D12).
pub def label(name)
  service:    name
  managed_by: "aura"
end

default_port: 8080

# Private: computed here, never exported and never in the output.
internal = 42
`,
    },
  },

  "Capabilities: an import cannot read": {
    entry: "main.aura",
    allowRead: true,
    env: "",
    files: {
      "main.aura": `# The root manifest is granted read access (see the checkbox above),
# yet the imported module still cannot read. Rights do not flow into imports.
import "dependency.aura" as dep

data: dep.contents
`,
      "dependency.aura": `# A "helpful" module from the internet.
contents: read_file("secrets.txt")
`,
      "secrets.txt": `s3cr3t
`,
    },
  },

  "Generating a non-JSON file": {
    entry: "main.aura",
    allowRead: false,
    env: "APP_ENV=production",
    files: {
      "main.aura": `app  = "gateway"
port = env("APP_ENV", "dev") == "production" ? 443 : 8080

# A block string keeps its contents verbatim, interpolation included.
nginx_conf: text
  server {
    listen #{port};
    server_name #{app}.example.com;

    location / {
      proxy_pass http://127.0.0.1:8080;
    }
  }
end
`,
    },
  },

  "An assertion stops the build": {
    entry: "main.aura",
    allowRead: false,
    env: "APP_ENV=production",
    files: {
      "main.aura": `is_prod = env("APP_ENV", "dev") == "production"
replicas = 1

# In production one replica is not enough — this fails the build, not the deploy.
assert !is_prod || replicas >= 2, "production needs at least two replicas"

service:
  replicas: replicas
end
`,
    },
  },
};

// ------------------------------------------------------------------- state

const state = {
  files: {},
  entry: "main.aura",
  active: "main.aura",
  format: "json",
  ready: false,
};

const $ = (id) => document.getElementById(id);
const els = {
  code: $("code"),
  highlight: $("highlight"),
  fileTabs: $("file-tabs"),
  formatTabs: $("format-tabs"),
  output: $("output"),
  diagnostics: $("diagnostics"),
  status: $("status"),
  example: $("example"),
  allowRead: $("allow-read"),
  env: $("env"),
  theme: $("theme"),
  formatBtn: $("format"),
  gutter: $("gutter"),
  splitter: $("splitter"),
};

// ------------------------------------------------------- byte offsets ↔ JS
//
// Rust reports byte offsets into UTF-8; JavaScript strings are UTF-16. For any
// non-ASCII character — a Cyrillic comment, an emoji in a string — the two
// disagree, and slicing with the wrong one corrupts the highlighting. Encoding
// once and slicing the bytes keeps them exact.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function sliceByBytes(bytes, start, end) {
  return decoder.decode(bytes.subarray(start, end));
}

const escapeHtml = (s) =>
  s.replace(
    /[&<>]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c],
  );

// ----------------------------------------------------------------- editor

function paintHighlight() {
  const src = els.code.value;
  let spans = [];
  if (state.ready) {
    try {
      spans = JSON.parse(highlight(src));
    } catch {
      spans = [];
    }
  }

  const bytes = encoder.encode(src);
  let html = "";
  let at = 0;
  for (const { s, e, k } of spans) {
    if (s < at) continue; // defensive: never emit overlapping spans
    html += escapeHtml(sliceByBytes(bytes, at, s));
    html += `<span class="${k}">${escapeHtml(sliceByBytes(bytes, s, e))}</span>`;
    at = e;
  }
  html += escapeHtml(sliceByBytes(bytes, at, bytes.length));

  // A trailing newline would otherwise collapse and shift the last line.
  els.highlight.innerHTML = html + "\n";
}

/** Line numbers, kept in their own element so a selection never includes them. */
function paintGutter() {
  const lines = els.code.value.split("\n").length;
  let text = "";
  for (let i = 1; i <= lines; i++) text += `${i}\n`;
  els.gutter.textContent = text;
}

function syncScroll() {
  const { scrollLeft: x, scrollTop: y } = els.code;
  els.highlight.style.transform = `translate(${-x}px, ${-y}px)`;
  // The gutter follows vertically only: it must stay pinned to the left edge.
  els.gutter.style.transform = `translateY(${-y}px)`;
}

// ------------------------------------------------------------------- tabs

function renderFileTabs() {
  els.fileTabs.textContent = "";
  for (const name of Object.keys(state.files)) {
    const tab = document.createElement("button");
    tab.className = "tab";
    tab.type = "button";
    tab.role = "tab";
    tab.setAttribute("aria-selected", String(name === state.active));
    tab.textContent = name;
    tab.title = name === state.entry ? "Entry point" : "Click to edit";
    tab.addEventListener("click", () => selectFile(name));

    if (name !== state.entry) {
      const close = document.createElement("span");
      close.className = "close";
      close.textContent = "×";
      close.title = `Remove ${name}`;
      close.addEventListener("click", (event) => {
        event.stopPropagation();
        delete state.files[name];
        if (state.active === name) state.active = state.entry;
        renderFileTabs();
        selectFile(state.active);
        run();
      });
      tab.append(close);
    }
    els.fileTabs.append(tab);
  }

  const add = document.createElement("button");
  add.className = "tab";
  add.type = "button";
  add.textContent = "+";
  add.title = "New file";
  add.addEventListener("click", () => {
    const name = prompt("File name", "extra.aura");
    if (!name || state.files[name] !== undefined) return;
    state.files[name] = "";
    renderFileTabs();
    selectFile(name);
  });
  els.fileTabs.append(add);
}

function selectFile(name) {
  state.active = name;
  els.code.value = state.files[name] ?? "";
  renderFileTabs();
  paintHighlight();
  paintGutter();
  syncScroll();
  els.code.focus();
}

// -------------------------------------------------------------- evaluation

let timer = null;

function scheduleRun() {
  els.output.classList.add("stale");
  clearTimeout(timer);
  timer = setTimeout(run, 200);
}

function parseEnv(text) {
  const out = {};
  for (const pair of text.split(",")) {
    const at = pair.indexOf("=");
    if (at <= 0) continue;
    out[pair.slice(0, at).trim()] = pair.slice(at + 1).trim();
  }
  return out;
}

function run() {
  if (!state.ready) return;
  state.files[state.active] = els.code.value;

  let result;
  try {
    result = JSON.parse(
      evaluate(
        state.files,
        state.entry,
        state.format,
        els.allowRead.checked,
        parseEnv(els.env.value),
      ),
    );
  } catch (error) {
    setStatus(`The module failed: ${error}`, false);
    return;
  }

  els.output.classList.remove("stale");
  const diagnostics = result.ok ? (result.warnings ?? []) : result.diagnostics;
  renderDiagnostics(diagnostics);

  if (result.ok) {
    els.output.textContent = result.output;
    const warnings = diagnostics.length;
    setStatus(
      warnings
        ? `Evaluated with ${warnings} warning${warnings > 1 ? "s" : ""}.`
        : "Evaluated.",
      true,
    );
  } else {
    els.output.textContent = "";
    setStatus(
      `${diagnostics.length} problem${diagnostics.length > 1 ? "s" : ""} — nothing was produced.`,
      false,
    );
  }
}

function setStatus(text, ok) {
  els.status.textContent = text;
  els.status.classList.toggle("ok", ok);
}

function renderDiagnostics(list) {
  els.diagnostics.textContent = "";
  for (const d of list) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = `diag ${d.severity}`;

    const code = document.createElement("span");
    code.className = "code";
    code.textContent = d.code;

    const message = document.createElement("span");
    message.textContent = d.help ? `${d.message} — ${d.help}` : d.message;

    const where = document.createElement("span");
    where.className = "where";
    where.textContent = d.line ? `${d.file}:${d.line}:${d.column}` : d.file;

    row.append(code, message, where);
    row.addEventListener("click", () => jumpTo(d));
    els.diagnostics.append(row);
  }
}

/** Open the buffer a diagnostic came from and put the caret on its line. */
function jumpTo(d) {
  if (d.file && state.files[d.file] !== undefined && d.file !== state.active) {
    state.files[state.active] = els.code.value;
    selectFile(d.file);
  }
  if (!d.line) return;
  const lines = els.code.value.split("\n");
  let offset = 0;
  for (let i = 0; i < d.line - 1 && i < lines.length; i++) {
    offset += lines[i].length + 1;
  }
  const caret = offset + Math.max(0, (d.column || 1) - 1);
  els.code.focus();
  els.code.setSelectionRange(caret, caret);
  // Bring the line into view: scroll roughly to it, then let the browser settle.
  const lineHeight = els.code.scrollHeight / Math.max(1, lines.length);
  els.code.scrollTop = Math.max(0, (d.line - 4) * lineHeight);
  syncScroll();
}

// -------------------------------------------------------------- examples UI

function loadExample(name) {
  const example = EXAMPLES[name];
  if (!example) return;
  state.files = structuredClone(example.files);
  state.entry = example.entry;
  state.active = example.entry;
  els.allowRead.checked = example.allowRead;
  els.env.value = example.env;
  renderFileTabs();
  selectFile(state.entry);
  run();
}

// ------------------------------------------------------------------ theme

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem("aura-theme", theme);
  } catch {
    /* private mode: the choice simply does not persist */
  }
  const system = matchMedia("(prefers-color-scheme: dark)").matches;
  const dark = theme === "dark" || (theme === "" && system);
  els.theme.textContent = dark ? "☀ Light" : "☾ Dark";
}

// ------------------------------------------------------------------- wiring

els.code.addEventListener("input", () => {
  state.files[state.active] = els.code.value;
  paintHighlight();
  paintGutter();
  scheduleRun();
});
els.code.addEventListener("scroll", syncScroll);

// Canonical formatting, the same the CLI applies. Keeps the caret roughly where
// it was: jumping to the top after every format is the reason people stop using
// a format button.
els.formatBtn.addEventListener("click", () => {
  if (!state.ready) return;
  const caret = els.code.selectionStart;
  const before = els.code.value;
  const after = format(before);
  if (after === before) return;
  els.code.value = after;
  state.files[state.active] = after;
  els.code.setSelectionRange(
    Math.min(caret, after.length),
    Math.min(caret, after.length),
  );
  paintHighlight();
  paintGutter();
  run();
});

// ---- the splitter ----
//
// Dragging writes a percentage into --split, which is the first grid track.
// Arrow keys move it too, so the layout is not mouse-only.

function setSplit(percent) {
  const clamped = Math.min(80, Math.max(20, percent));
  document.documentElement.style.setProperty("--split", `${clamped}%`);
  els.splitter.setAttribute("aria-valuenow", String(Math.round(clamped)));
  try {
    localStorage.setItem("aura-split", String(clamped));
  } catch {
    /* private mode */
  }
  syncScroll();
}

function splitFromEvent(event) {
  const main = els.splitter.parentElement.getBoundingClientRect();
  return ((event.clientX - main.left) / main.width) * 100;
}

els.splitter.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  els.splitter.setPointerCapture(event.pointerId);
  const move = (e) => setSplit(splitFromEvent(e));
  const up = (e) => {
    els.splitter.releasePointerCapture(e.pointerId);
    els.splitter.removeEventListener("pointermove", move);
    els.splitter.removeEventListener("pointerup", up);
    document.body.style.userSelect = "";
  };
  // Without this a drag selects the text it passes over.
  document.body.style.userSelect = "none";
  els.splitter.addEventListener("pointermove", move);
  els.splitter.addEventListener("pointerup", up);
});

els.splitter.addEventListener("keydown", (event) => {
  const step = event.key === "ArrowLeft" ? -2 : event.key === "ArrowRight" ? 2 : 0;
  if (!step) return;
  event.preventDefault();
  const current =
    parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--split")) ||
    50;
  setSplit(current + step);
});

els.splitter.addEventListener("dblclick", () => setSplit(50));

// Tab indents instead of leaving the editor. Shift+Tab still moves focus out,
// so the page stays keyboard-navigable.
els.code.addEventListener("keydown", (event) => {
  if (event.key !== "Tab" || event.shiftKey) return;
  event.preventDefault();
  const { selectionStart: start, selectionEnd: end, value } = els.code;
  els.code.value = `${value.slice(0, start)}  ${value.slice(end)}`;
  els.code.setSelectionRange(start + 2, start + 2);
  state.files[state.active] = els.code.value;
  paintHighlight();
  paintGutter();
  scheduleRun();
});

els.formatTabs.addEventListener("click", (event) => {
  const tab = event.target.closest("[data-format]");
  if (!tab) return;
  state.format = tab.dataset.format;
  for (const other of els.formatTabs.children) {
    other.setAttribute("aria-selected", String(other === tab));
  }
  run();
});

els.example.addEventListener("change", () => loadExample(els.example.value));
els.allowRead.addEventListener("change", run);
els.env.addEventListener("input", scheduleRun);
els.theme.addEventListener("click", () => {
  const system = matchMedia("(prefers-color-scheme: dark)").matches;
  const now = document.documentElement.dataset.theme || (system ? "dark" : "light");
  applyTheme(now === "dark" ? "light" : "dark");
});

// -------------------------------------------------------------------- boot

for (const name of Object.keys(EXAMPLES)) {
  const option = document.createElement("option");
  option.value = name;
  option.textContent = name;
  els.example.append(option);
}

let saved = "";
let savedSplit = null;
try {
  saved = localStorage.getItem("aura-theme") ?? "";
  savedSplit = localStorage.getItem("aura-split");
} catch {
  /* ignore */
}
applyTheme(saved);
setSplit(savedSplit ? parseFloat(savedSplit) : 50);

init()
  .then(() => {
    state.ready = true;
    setStatus("Ready.", true);
    loadExample(Object.keys(EXAMPLES)[0]);
  })
  .catch((error) => {
    els.output.textContent = "";
    setStatus(
      `Could not start WebAssembly: ${error}. The playground needs a browser with WebAssembly support.`,
      false,
    );
  });
