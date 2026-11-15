/* Siksik front-end. Talks to the Rust backend through Tauri's global API. */
const T = window.__TAURI__ || null;
const invoke = (cmd, args) => (T ? T.core.invoke(cmd, args) : Promise.resolve(null));
const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];

let settings = null;
let watchStatus = { active: [], errors: [] };
const batches = new Map(); // id -> {total, done}
const MAX_ROWS = 400;

// ---------- helpers ----------
function fmtBytes(n) {
  if (!n || n < 0) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), u.length - 1);
  const v = n / Math.pow(1024, i);
  return `${v >= 100 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${u[i]}`;
}
const pct = (before, after) => (before > 0 ? ((before - after) / before) * 100 : 0);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const dirOf = (p) => p.replace(/[\\/][^\\/]*$/, "");

let toastTimer;
function toast(msg) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), 2600);
}

// ---------- navigation ----------
$$(".nav-item").forEach((b) =>
  b.addEventListener("click", () => {
    $$(".nav-item").forEach((x) => x.classList.toggle("active", x === b));
    $$(".view").forEach((v) => v.classList.toggle("active", v.id === `view-${b.dataset.view}`));
  })
);

// ---------- theme ----------
const media = window.matchMedia("(prefers-color-scheme: dark)");
function applyTheme() {
  const pref = settings?.theme || "system";
  const dark = pref === "dark" || (pref === "system" && media.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  $$("[data-theme-set]").forEach((b) => b.classList.toggle("active", b.dataset.themeSet === pref));
  try {
    T?.window.getCurrentWindow().setTheme(pref === "system" ? null : pref);
  } catch (_) {}
}
media.addEventListener("change", applyTheme);
$$("[data-theme-set]").forEach((b) => b.addEventListener("click", () => update({ theme: b.dataset.themeSet })));

// ---------- settings ----------
async function update(patch) {
  settings = { ...settings, ...patch };
  renderSettings();
  try {
    const st = await invoke("save_settings", { settings });
    if (st) watchStatus = st;
    renderWatch();
  } catch (e) {
    toast(`Could not save settings: ${e}`);
  }
}

function renderSettings() {
  const s = settings;
  applyTheme();
  $$(".segmented[data-setting]").forEach((seg) => {
    const val = String(s[seg.dataset.setting]);
    seg.querySelectorAll("button").forEach((b) => b.classList.toggle("active", b.dataset.value === val));
  });
  $$("input[data-setting]").forEach((inp) => {
    const k = inp.dataset.setting;
    if (inp.type === "checkbox") inp.checked = !!s[k];
    else if (document.activeElement !== inp) inp.value = s[k];
  });
  $("#qualityVal").textContent = s.jpeg_quality;
  $("#levelVal").textContent = s.png_level;
  $("#qualityField").hidden = s.mode !== "smart";
  $("#modeHelp").textContent =
    s.mode === "smart"
      ? "JPEGs are re-encoded with MozJPEG when it saves more (visually lossless, like JPEGmini). PNG and WebP stay strictly lossless."
      : "Every pixel stays identical. JPEG data is repacked, PNG and WebP are re-compressed losslessly.";
  $("#modeLine").textContent =
    s.mode === "smart" ? `Smart · visually lossless JPEG at quality ${s.jpeg_quality}` : "Lossless · pixels stay identical";
  $("#outDirField").hidden = s.output_mode !== "folder";
  $("#outDirText").textContent = s.output_dir || "Not set. Choose a folder.";
  const inPlace = s.output_mode === "same";
  $("#suffixField").hidden = !(inPlace && !s.overwrite);
  $("#overwriteHelp").textContent = s.overwrite
    ? inPlace
      ? "Originals are replaced with the compressed version."
      : "Files with the same name in the output folder are replaced."
    : inPlace
      ? "A compressed copy is saved next to each original."
      : "Existing files are kept; new files get a number, e.g. photo (1).jpg.";
  $$(".suffix-ref").forEach((e) => (e.textContent = s.suffix || "-siksik"));
  $("#watchEnabled").checked = s.watch_enabled;
  const name = s.custom_icon ? s.custom_icon.split(/[\\/]/).pop() : "";
  $("#resetIcon").hidden = !s.custom_icon;
  $("#pickIcon").textContent = name ? name : "Choose PNG…";
}

$$(".segmented[data-setting]").forEach((seg) =>
  seg.addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    const k = seg.dataset.setting;
    let v = b.dataset.value;
    if (v === "true" || v === "false") v = v === "true";
    update({ [k]: v });
  })
);
$$("input[data-setting]").forEach((inp) => {
  const k = inp.dataset.setting;
  if (inp.type === "checkbox") inp.addEventListener("change", () => update({ [k]: inp.checked }));
  else if (inp.type === "range") {
    inp.addEventListener("input", () => {
      $(k === "jpeg_quality" ? "#qualityVal" : "#levelVal").textContent = inp.value;
    });
    inp.addEventListener("change", () => update({ [k]: Number(inp.value) }));
  } else {
    inp.addEventListener("change", () => update({ [k]: inp.value.trim().replace(/[\\/:*?"<>|]/g, "") }));
  }
});

async function pickDir(title) {
  if (!T) return null;
  return await T.dialog.open({ directory: true, multiple: false, title });
}

$("#pickOutDir").addEventListener("click", async () => {
  const d = await pickDir("Choose output folder");
  if (d) update({ output_dir: d, output_mode: "folder" });
});
$("#pickIcon").addEventListener("click", async () => {
  if (!T) return;
  const f = await T.dialog.open({ multiple: false, title: "Choose an app icon", filters: [{ name: "PNG image", extensions: ["png"] }] });
  if (f) {
    await update({ custom_icon: f });
    toast("Window icon updated");
  }
});
$("#resetIcon").addEventListener("click", () => update({ custom_icon: "" }));

// ---------- compress ----------
const IMG_EXT = ["jpg", "jpeg", "jfif", "png", "webp"];
async function compress(paths) {
  if (!paths || !paths.length) return;
  if (settings.output_mode === "folder" && !settings.output_dir) {
    toast("Choose an output folder in Settings first");
    return;
  }
  const n = await invoke("compress_paths", { paths });
  if (n === 0) toast("No JPEG, PNG or WebP images found");
}

$("#pickFiles").addEventListener("click", async () => {
  if (!T) return;
  const f = await T.dialog.open({ multiple: true, title: "Choose images", filters: [{ name: "Images", extensions: IMG_EXT }] });
  if (f) compress(Array.isArray(f) ? f : [f]);
});
$("#pickFolder").addEventListener("click", async () => {
  if (!T) return;
  const f = await T.dialog.open({ directory: true, multiple: true, title: "Choose folders" });
  if (f) compress(Array.isArray(f) ? f : [f]);
});
$("#drop").addEventListener("click", (e) => {
  if (e.target === e.currentTarget || e.target.closest(".drop-inner") && !e.target.closest("button")) $("#pickFiles").click();
});

function updateProgress() {
  let total = 0, done = 0;
  for (const b of batches.values()) { total += b.total; done += b.done; }
  const p = $("#progress");
  if (!total) { p.hidden = true; return; }
  p.hidden = false;
  $("#progressFill").style.width = `${(done / total) * 100}%`;
  $("#progressText").textContent = `${done} / ${total}`;
}

function addRow(r) {
  $("#emptyState")?.remove();
  const ext = (r.name.split(".").pop() || "").toLowerCase();
  const kind = ext === "png" ? "png" : ext === "webp" ? "webp" : "jpg";
  const saved = pct(r.before, r.after);
  const row = document.createElement("div");
  row.className = "row";
  let right, sub;
  if (r.status === "optimized") {
    right = `<div class="pct ok">−${saved.toFixed(1)}%</div>`;
    sub = `<div class="file-sub">${esc(dirOf(r.output))}</div>`;
  } else if (r.status === "skipped") {
    right = `<div class="pct skip">Optimal</div>`;
    sub = `<div class="file-sub">${esc(r.message)}</div>`;
  } else {
    right = `<div class="pct fail">Failed</div>`;
    sub = `<div class="file-sub err" title="${esc(r.message)}">${esc(r.message)}</div>`;
  }
  const sizes =
    r.status === "failed"
      ? `<div class="sizes">—</div>`
      : `<div><div class="sizes">${fmtBytes(r.before)} → <b>${fmtBytes(r.after)}</b></div><div class="bar"><div style="width:${Math.max(0, 100 - saved)}%"></div></div></div>`;
  row.innerHTML = `
    <div class="badge ${kind}">${esc(ext.toUpperCase().slice(0, 4))}</div>
    <div style="min-width:0"><div class="file-name" title="${esc(r.path)}">${esc(r.name)}${r.source === "watch" ? '<span class="tag-watch">auto</span>' : ""}</div>${sub}</div>
    ${sizes}
    ${right}
    <button class="icon-btn" title="Show in folder"${r.status === "failed" ? " disabled style='visibility:hidden'" : ""}>
      <svg viewBox="0 0 24 24"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
    </button>`;
  row.querySelector(".icon-btn").addEventListener("click", () => {
    T?.opener.revealItemInDir(r.output || r.path).catch((e) => toast(String(e)));
  });
  const list = $("#results");
  list.prepend(row);
  while (list.children.length > MAX_ROWS) list.lastElementChild.remove();
}

$("#clearList").addEventListener("click", () => {
  $("#results").innerHTML = '<div class="empty" id="emptyState">Results will appear here.</div>';
});

// ---------- stats ----------
function renderStats(p) {
  if (!p) return;
  const l = p.lifetime, s = p.session;
  const lSaved = l.bytes_before - l.bytes_after;
  const sSaved = s.bytes_before - s.bytes_after;
  $("#sideSaved").textContent = fmtBytes(lSaved);
  $("#sOk").textContent = s.optimized;
  $("#sSkip").textContent = s.skipped;
  $("#sFail").textContent = s.failed;
  $("#sSaved").textContent = fmtBytes(sSaved);
  $("#lSaved").textContent = fmtBytes(lSaved);
  $("#lPct").textContent = l.bytes_before ? `${pct(l.bytes_before, l.bytes_after).toFixed(1)}% smaller on average` : "No images yet";
  $("#lOk").textContent = l.files_optimized.toLocaleString();
  $("#lBefore").textContent = `${fmtBytes(l.bytes_before)} processed`;
  $("#lSkip").textContent = l.files_skipped.toLocaleString();
  $("#lFail").textContent = l.files_failed.toLocaleString();
  $("#ssOk").textContent = (s.optimized + s.skipped).toLocaleString();
  $("#ssFail").textContent = s.failed.toLocaleString();
  $("#ssSaved").textContent = fmtBytes(sSaved);
  $("#ssPct").textContent = s.bytes_before ? `${pct(s.bytes_before, s.bytes_after).toFixed(1)}%` : "—";
  if (l.first_used) $("#sinceLine").textContent = `All-time totals since ${new Date(l.first_used * 1000).toLocaleDateString()}`;
}
const session = { optimized: 0, skipped: 0, failed: 0, bytes_before: 0, bytes_after: 0 };
let lifetime = null;
function bump(r) {
  // live update between backend "stats" events
  if (!lifetime) return;
  const key = r.status === "optimized" ? "optimized" : r.status === "skipped" ? "skipped" : "failed";
  session[key]++;
  lifetime[key === "optimized" ? "files_optimized" : key === "skipped" ? "files_skipped" : "files_failed"]++;
  if (key === "optimized") {
    session.bytes_before += r.before; session.bytes_after += r.after;
    lifetime.bytes_before += r.before; lifetime.bytes_after += r.after;
  }
  renderStats({ lifetime, session });
}
function syncStats(p) {
  if (!p) return;
  lifetime = { ...p.lifetime };
  Object.assign(session, p.session);
  renderStats(p);
}
$("#resetSession").addEventListener("click", async () => {
  syncStats(await invoke("reset_stats", { lifetime: false }));
  toast("Session stats reset");
});
$("#resetAll").addEventListener("click", async () => {
  const ok = T ? await T.dialog.ask("Reset all-time statistics? This cannot be undone.", { title: "Siksik", kind: "warning" }) : confirm("Reset?");
  if (!ok) return;
  syncStats(await invoke("reset_stats", { lifetime: true }));
  toast("All statistics reset");
});

// ---------- watch ----------
function renderWatch() {
  const list = $("#folderList");
  const folders = settings.watch_folders || [];
  const live = new Set(watchStatus.active || []);
  list.innerHTML = folders.length
    ? ""
    : '<div class="folder-empty">No folders yet. Add one and new images in it will be compressed automatically.</div>';
  folders.forEach((f) => {
    const el = document.createElement("div");
    el.className = "folder";
    el.innerHTML = `<span class="dot ${live.has(f) ? "live" : ""}" title="${live.has(f) ? "Watching" : "Paused"}"></span>
      <span class="path" title="${esc(f)}">${esc(f)}</span>
      <button class="icon-btn" title="Open folder"><svg viewBox="0 0 24 24"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg></button>
      <button class="icon-btn" title="Stop watching"><svg viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg></button>`;
    const [open, rm] = el.querySelectorAll(".icon-btn");
    open.addEventListener("click", () => T?.opener.openPath(f).catch((e) => toast(String(e))));
    rm.addEventListener("click", () => update({ watch_folders: folders.filter((x) => x !== f) }));
    list.appendChild(el);
  });
  $("#watchErrors").textContent = (watchStatus.errors || []).join("\n");
  const badge = $("#watchBadge");
  badge.hidden = !live.size;
  badge.textContent = live.size;
}
$("#addWatch").addEventListener("click", async () => {
  const d = await pickDir("Choose a folder to watch");
  if (d && !settings.watch_folders.includes(d)) update({ watch_folders: [...settings.watch_folders, d], watch_enabled: true });
});
$("#watchEnabled").addEventListener("change", (e) => update({ watch_enabled: e.target.checked }));

// ---------- boot ----------
async function boot() {
  settings = (await invoke("get_settings")) || {
    theme: "system", mode: "lossless", jpeg_quality: 85, png_level: 4, png_zopfli: false, keep_metadata: true,
    output_mode: "same", output_dir: "", overwrite: false, suffix: "-siksik", preserve_dates: true, recursive: true,
    watch_enabled: true, watch_folders: [], custom_icon: "",
  };
  renderSettings();
  watchStatus = (await invoke("watch_status")) || watchStatus;
  renderWatch();
  syncStats(await invoke("get_stats"));

  if (!T) return; // opened in a plain browser – UI preview only
  const { listen } = T.event;
  listen("batch-start", ({ payload }) => {
    batches.set(payload.id, { total: payload.total, done: 0 });
    updateProgress();
    if (payload.source === "watch") toast(`Auto-compressing ${payload.total} new image${payload.total > 1 ? "s" : ""}`);
  });
  listen("file-done", ({ payload }) => {
    addRow(payload);
    bump(payload);
    // attribute to the oldest unfinished batch
    for (const b of batches.values()) if (b.done < b.total) { b.done++; break; }
    updateProgress();
  });
  listen("batch-end", ({ payload }) => {
    batches.delete(payload.id);
    setTimeout(updateProgress, 400);
  });
  listen("stats", ({ payload }) => syncStats(payload));

  const overlay = $("#dragOverlay");
  T.webview.getCurrentWebview().onDragDropEvent((e) => {
    const t = e.payload.type;
    if (t === "enter" || t === "over") overlay.classList.add("show");
    else if (t === "leave") overlay.classList.remove("show");
    else if (t === "drop") {
      overlay.classList.remove("show");
      compress(e.payload.paths);
    }
  });
}
boot();
