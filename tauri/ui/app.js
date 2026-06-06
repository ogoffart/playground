// git-review frontend. Plain JS, no framework, no bundler.
// IPC: `window.__TAURI__.core.invoke` (enabled via `withGlobalTauri`).

const invoke = window.__TAURI__.core.invoke;

// --- application state ------------------------------------------------------
const state = {
  commits: [],
  // current selection shown in the main view
  showing: { mode: "working", a: null, b: null }, // a/b are full oids
  from: null, // oid string for comparison "from"
  to: null, // oid string for comparison "to"
  diff: null,
  settings: {
    word_wrap: false,
    show_space: true, // when false -> ignore whitespace
    font_size: 12,
    line_numbers: true,
  },
};

const $ = (sel) => document.querySelector(sel);

// --- icons by extension -----------------------------------------------------
function fileIcon(path) {
  const ext = (path.split(".").pop() || "").toLowerCase();
  const map = {
    rs: "🦀",
    py: "🐍",
    js: "📜",
    ts: "📜",
    tsx: "📜",
    jsx: "📜",
    md: "📝",
    toml: "⚙",
    yaml: "⚙",
    yml: "⚙",
    json: "⚙",
    ini: "⚙",
    cfg: "⚙",
    png: "🖼",
    jpg: "🖼",
    jpeg: "🖼",
    gif: "🖼",
    svg: "🖼",
    sh: "💲",
    bash: "💲",
    html: "🌐",
    css: "🌐",
  };
  return map[ext] || "📄";
}

function esc(s) {
  const d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
}

// --- backend queries --------------------------------------------------------
async function loadCommits() {
  state.commits = await invoke("commits");
  // Default to most recent real commit, else working tree.
  const firstCommit = state.commits.find((c) => !c.is_working_tree);
  if (firstCommit) {
    state.showing = { mode: "commit", a: firstCommit.oid, b: null };
  } else {
    state.showing = { mode: "working", a: null, b: null };
  }
  renderCommits();
  await refreshDiff();
}

async function refreshDiff() {
  const ignore_ws = !state.settings.show_space;
  try {
    state.diff = await invoke("diff", {
      mode: state.showing.mode,
      a: state.showing.a,
      b: state.showing.b,
      ignoreWs: ignore_ws,
    });
    renderToolbar();
    renderFileTree();
    renderDiff();
  } catch (e) {
    $("#diff").innerHTML = `<div class="error">Error: ${esc(String(e))}</div>`;
  }
}

function selectShowing(showing) {
  const s = state.showing;
  if (s.mode === showing.mode && s.a === showing.a && s.b === showing.b) return;
  state.showing = showing;
  renderCommits();
  refreshDiff();
}

function maybeRange() {
  if (state.from && state.to) {
    selectShowing({ mode: "range", a: state.from, b: state.to });
  }
}

// --- rendering: commit list -------------------------------------------------
function isCurrentRow(c) {
  const s = state.showing;
  if (c.is_working_tree) return s.mode === "working";
  return s.mode === "commit" && s.a === c.oid;
}

function renderCommits() {
  const el = $("#commit-list");
  el.innerHTML = "";
  for (const c of state.commits) {
    const row = document.createElement("div");
    row.className = "commit" + (isCurrentRow(c) ? " current" : "");

    const l1 = document.createElement("div");
    l1.className = "commit-l1";

    if (!c.is_working_tree) {
      const from = document.createElement("button");
      from.className = "endpoint" + (state.from === c.oid ? " active" : "");
      from.textContent = "◀";
      from.title = "Compare from this commit";
      from.onclick = (e) => {
        e.stopPropagation();
        state.from = c.oid;
        renderCommits();
        maybeRange();
      };
      const to = document.createElement("button");
      to.className = "endpoint" + (state.to === c.oid ? " active" : "");
      to.textContent = "▶";
      to.title = "Compare to this commit";
      to.onclick = (e) => {
        e.stopPropagation();
        state.to = c.oid;
        renderCommits();
        maybeRange();
      };
      l1.appendChild(from);
      l1.appendChild(to);
    } else {
      const spacer = document.createElement("span");
      spacer.className = "endpoint-spacer";
      l1.appendChild(spacer);
    }

    const sha = document.createElement("span");
    sha.className = "sha";
    sha.textContent = c.short;
    const date = document.createElement("span");
    date.className = "date";
    date.textContent = c.date;
    const author = document.createElement("span");
    author.className = "author";
    author.textContent = c.author;
    l1.appendChild(sha);
    l1.appendChild(date);
    l1.appendChild(author);

    const title = document.createElement("div");
    title.className = "title";
    title.textContent = c.title;

    row.appendChild(l1);
    row.appendChild(title);
    row.onclick = () => {
      if (c.is_working_tree) {
        selectShowing({ mode: "working", a: null, b: null });
      } else {
        selectShowing({ mode: "commit", a: c.oid, b: null });
      }
    };
    el.appendChild(row);
  }
}

// --- rendering: toolbar -----------------------------------------------------
function renderToolbar() {
  const d = state.diff;
  $("#summary-text").textContent = d ? d.summary : "";
  $("#added").textContent = d ? `+${d.added}` : "";
  $("#removed").textContent = d ? `−${d.removed}` : "";

  const s = state.settings;
  $("#t-wrap").classList.toggle("active", s.word_wrap);
  $("#t-space").classList.toggle("active", s.show_space);
  $("#t-lines").classList.toggle("active", s.line_numbers);

  document.documentElement.style.setProperty(
    "--font-size",
    s.font_size + "px"
  );
}

// --- rendering: file tree ---------------------------------------------------
function renderFileTree() {
  const el = $("#file-tree");
  el.innerHTML = "";
  if (!state.diff) return;
  const files = state.diff.files;
  $("#files-head").textContent = `FILES (${files.length})`;

  // Build a directory tree.
  const root = { dirs: new Map(), files: [] };
  files.forEach((f, idx) => {
    const parts = f.path.split("/");
    const name = parts.pop();
    let node = root;
    for (const p of parts) {
      if (!node.dirs.has(p)) node.dirs.set(p, { dirs: new Map(), files: [] });
      node = node.dirs.get(p);
    }
    node.files.push({ name, file: f, idx });
  });

  const collapseState = renderFileTree._collapsed || (renderFileTree._collapsed = new Set());

  function walk(node, prefix, depth, parentCollapsed) {
    // directories first
    for (let [dname, dnode] of node.dirs) {
      // GitHub-style: collapse single-child directory chains (a/b/c) into one node,
      // but stop merging as soon as a directory has files or branches.
      let label = dname;
      let full = prefix + dname + "/";
      while (
        dnode.files.length === 0 &&
        dnode.dirs.size === 1
      ) {
        const [childName, childNode] = dnode.dirs.entries().next().value;
        label += "/" + childName;
        full += childName + "/";
        dnode = childNode;
      }

      const collapsed = collapseState.has(full);
      const dir = document.createElement("div");
      dir.className = "tree-dir";
      dir.style.paddingLeft = 10 + depth * 14 + "px";
      if (parentCollapsed) dir.classList.add("hidden");
      const caret = document.createElement("span");
      caret.className = "caret";
      caret.textContent = collapsed ? "▶" : "▼";
      dir.appendChild(caret);
      dir.appendChild(document.createTextNode("📁 " + label));
      dir.onclick = () => {
        if (collapsed) collapseState.delete(full);
        else collapseState.add(full);
        renderFileTree();
      };
      el.appendChild(dir);
      walk(dnode, full, depth + 1, parentCollapsed || collapsed);
    }
    for (const { name, file, idx } of node.files) {
      const row = document.createElement("div");
      row.className = "tree-file";
      row.style.paddingLeft = 10 + depth * 14 + "px";
      if (parentCollapsed) row.classList.add("hidden");
      const icon = document.createElement("span");
      icon.className = "icon";
      icon.textContent = fileIcon(name);
      const nm = document.createElement("span");
      nm.className = "name";
      nm.textContent = name;
      nm.title = file.path;
      const counts = document.createElement("span");
      counts.className = "counts";
      counts.innerHTML = `<span class="add">+${file.added}</span><span class="del">−${file.removed}</span>`;
      row.appendChild(icon);
      row.appendChild(nm);
      row.appendChild(counts);
      row.onclick = () => scrollToFile(idx);
      el.appendChild(row);
    }
  }
  walk(root, "", 0, false);
}

function scrollToFile(idx) {
  const target = document.getElementById("file-" + idx);
  if (target) target.scrollIntoView({ block: "start", behavior: "smooth" });
}

// --- rendering: diff body ---------------------------------------------------
function renderDiff() {
  const el = $("#diff");
  el.innerHTML = "";
  const s = state.settings;
  el.classList.toggle("wrap", s.word_wrap);
  el.classList.toggle("no-lines", !s.line_numbers);

  const d = state.diff;
  if (!d) return;

  // commit message (single-commit view only)
  if (d.message) {
    const m = d.message;
    const box = document.createElement("div");
    box.className = "commit-message";
    let html = `<h2>${esc(m.title)}</h2>`;
    html += `<div class="meta">${esc(m.author)} · ${esc(m.date)} · ${esc(
      m.short
    )}</div>`;
    if (m.body) html += `<pre>${esc(m.body)}</pre>`;
    box.innerHTML = html;
    el.appendChild(box);
  }

  if (d.files.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No changes.";
    el.appendChild(empty);
    return;
  }

  d.files.forEach((f, idx) => {
    const file = document.createElement("div");
    file.className = "file";
    file.id = "file-" + idx;

    const header = document.createElement("div");
    header.className = "file-header";
    const renamed =
      f.old_path && f.kind === "R"
        ? `${esc(f.old_path)} → ${esc(f.path)}`
        : esc(f.path);
    header.innerHTML =
      `<span class="icon">${fileIcon(f.path)}</span>` +
      `<span class="fpath">${renamed}</span>` +
      `<span class="badge">[${f.kind}]</span>` +
      `<span class="counts"><span class="add">+${f.added}</span><span class="del">−${f.removed}</span></span>`;
    file.appendChild(header);

    const body = document.createElement("div");
    body.className = "file-body";
    if (f.binary) {
      const b = document.createElement("div");
      b.className = "binary";
      b.textContent = "Binary file not shown";
      body.appendChild(b);
    } else {
      for (const line of f.lines) {
        body.appendChild(renderLine(line));
      }
    }
    file.appendChild(body);
    el.appendChild(file);
  });
}

function renderLine(line) {
  const row = document.createElement("div");
  if (line.is_hunk_header) {
    row.className = "row hunk";
    const oldn = document.createElement("span");
    oldn.className = "gutter-num";
    const newn = document.createElement("span");
    newn.className = "gutter-num";
    const sign = document.createElement("span");
    sign.className = "sign";
    const code = document.createElement("span");
    code.className = "code";
    code.textContent = line.spans.map((s) => s.text).join("");
    row.appendChild(oldn);
    row.appendChild(newn);
    row.appendChild(sign);
    row.appendChild(code);
    return row;
  }

  row.className = "row " + line.kind;
  const oldn = document.createElement("span");
  oldn.className = "gutter-num";
  oldn.textContent = line.old_no != null ? line.old_no : "";
  const newn = document.createElement("span");
  newn.className = "gutter-num";
  newn.textContent = line.new_no != null ? line.new_no : "";

  const sign = document.createElement("span");
  sign.className = "sign";
  sign.textContent =
    line.kind === "added" ? "+" : line.kind === "removed" ? "-" : "";

  const code = document.createElement("span");
  code.className = "code";
  if (line.spans.length === 0) {
    code.appendChild(document.createTextNode(" "));
  } else {
    for (const sp of line.spans) {
      const span = document.createElement("span");
      span.textContent = sp.text;
      span.style.color = sp.color;
      if (sp.bold) span.style.fontWeight = "700";
      if (sp.italic) span.style.fontStyle = "italic";
      code.appendChild(span);
    }
  }

  row.appendChild(oldn);
  row.appendChild(newn);
  row.appendChild(sign);
  row.appendChild(code);
  return row;
}

// --- toolbar wiring ---------------------------------------------------------
function wireToolbar() {
  $("#t-wrap").onclick = () => {
    state.settings.word_wrap = !state.settings.word_wrap;
    renderToolbar();
    renderDiff();
  };
  $("#t-space").onclick = () => {
    state.settings.show_space = !state.settings.show_space;
    renderToolbar();
    refreshDiff(); // whitespace setting re-queries the backend
  };
  $("#t-fdec").onclick = () => {
    state.settings.font_size = Math.max(8, state.settings.font_size - 1);
    renderToolbar();
  };
  $("#t-finc").onclick = () => {
    state.settings.font_size = Math.min(28, state.settings.font_size + 1);
    renderToolbar();
  };
  $("#t-lines").onclick = () => {
    state.settings.line_numbers = !state.settings.line_numbers;
    renderToolbar();
    renderDiff();
  };
}

// --- resizable dividers -----------------------------------------------------
function wireDividers() {
  // horizontal: side panel width
  const side = $("#side");
  dragDivider($("#side-hsplit"), (dx, startW) => {
    const w = Math.min(640, Math.max(220, startW + dx));
    side.style.width = w + "px";
  }, () => side.getBoundingClientRect().width, "x");

  // vertical: commits pane height inside the side panel
  const commits = $("#commits-pane");
  dragDivider($("#side-vsplit"), (dy, startH) => {
    const total = side.getBoundingClientRect().height;
    const h = Math.min(total - 120, Math.max(80, startH + dy));
    commits.style.height = h + "px";
    commits.style.flex = "none";
  }, () => commits.getBoundingClientRect().height, "y");
}

function dragDivider(handle, onMove, getStart, axis) {
  handle.addEventListener("mousedown", (e) => {
    e.preventDefault();
    const start = axis === "x" ? e.clientX : e.clientY;
    const startVal = getStart();
    document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
    const move = (ev) => {
      const now = axis === "x" ? ev.clientX : ev.clientY;
      onMove(now - start, startVal);
    };
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      document.body.style.cursor = "";
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  });
}

// --- boot -------------------------------------------------------------------
async function main() {
  wireToolbar();
  wireDividers();
  try {
    const name = await invoke("repo_name");
    $("#commits-head").textContent = "COMMITS · " + name;
  } catch (e) {
    /* ignore */
  }
  await loadCommits();
}

window.addEventListener("DOMContentLoaded", main);
