// Sleepers Researcher — frontend controller
// Talks to the Rust/Tauri backend over IPC. Falls back to a local echo when
// opened in a plain browser (so the design system can be previewed).

const TAURI = window.__TAURI__ ?? null;
const invoke = TAURI ? TAURI.core.invoke : null;
const listen = TAURI ? TAURI.event.listen : null;

const $ = (sel) => document.querySelector(sel);
const el = (tag, cls) => { const n = document.createElement(tag); if (cls) n.className = cls; return n; };

const state = {
  backend: "gemini",
  busy: false,
  contextTokens: 0,
  memChunks: 0,
};

const BACKEND_HINTS = {
  gemini: "Heavy reasoning",
  groq: "Fast / cheap",
};

const chat = $("#chat");
let streamingBubble = null;

// ---------- markdown (XSS-safe: escape first, then insert known tags) ----------
function escapeHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;")
          .replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function inlineMd(s) {
  let t = escapeHtml(s);
  t = t.replace(/`([^`]+)`/g, "<code>$1</code>");
  t = t.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  t = t.replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<em>$2</em>");
  // [text](url)
  t = t.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
    (m, txt, url) => `<a class="md-link" data-url="${url}">${txt}</a>`);
  // bare urls (not already inside an attribute or tag body)
  t = t.replace(/(^|[\s(])(https?:\/\/[^\s<)"]+)/g,
    (m, pre, url) => `${pre}<a class="md-link" data-url="${url}">${url}</a>`);
  return t;
}

function codeBlockHtml(b) {
  const lang = escapeHtml(b.lang || "code");
  const code = escapeHtml(b.code.replace(/\n$/, ""));
  return `<div class="code-wrap"><div class="code-head"><span>${lang}</span>` +
         `<button class="code-copy">Copy</button></div><pre><code>${code}</code></pre></div>`;
}

function renderMarkdown(src) {
  const blocks = [];
  const text = src.replace(/```(\w*)\r?\n?([\s\S]*?)```/g, (m, lang, code) => {
    blocks.push({ lang: lang || "", code });
    return `@@CODEBLOCK${blocks.length - 1}@@`;
  });

  const lines = text.split(/\r?\n/);
  let html = "";
  let listType = null;
  const closeList = () => { if (listType) { html += `</${listType}>`; listType = null; } };

  for (const line of lines) {
    const cb = line.trim().match(/^@@CODEBLOCK(\d+)@@$/);
    if (cb) { closeList(); html += codeBlockHtml(blocks[+cb[1]]); continue; }
    if (/^\s*$/.test(line)) { closeList(); continue; }

    let m;
    if ((m = line.match(/^(#{1,4})\s+(.*)$/))) {
      closeList();
      const lvl = Math.min(m[1].length + 2, 6);
      html += `<h${lvl}>${inlineMd(m[2])}</h${lvl}>`;
    } else if ((m = line.match(/^\s*[-*+]\s+(.*)$/))) {
      if (listType !== "ul") { closeList(); html += "<ul>"; listType = "ul"; }
      html += `<li>${inlineMd(m[1])}</li>`;
    } else if ((m = line.match(/^\s*\d+[.)]\s+(.*)$/))) {
      if (listType !== "ol") { closeList(); html += "<ol>"; listType = "ol"; }
      html += `<li>${inlineMd(m[1])}</li>`;
    } else if ((m = line.match(/^\s*>\s?(.*)$/))) {
      closeList();
      html += `<blockquote>${inlineMd(m[1])}</blockquote>`;
    } else {
      closeList();
      html += `<p>${inlineMd(line)}</p>`;
    }
  }
  closeList();
  return html;
}

// ---------- scrolling ----------
function nearBottom() {
  return chat.scrollHeight - chat.scrollTop - chat.clientHeight < 140;
}
function scrollToBottom(smooth = false) {
  chat.scrollTo({ top: chat.scrollHeight, behavior: smooth ? "smooth" : "auto" });
}
// Only auto-follow when the user is already at the bottom, so scrolling back
// through history is never yanked away mid-read.
function stickScroll(wasNear) {
  if (wasNear) scrollToBottom();
  updateJumpButton();
}
function updateJumpButton() {
  const btn = $("#jump-latest");
  if (btn) btn.classList.toggle("show", !nearBottom());
}

// ---------- chat rendering ----------
function clearEmptyState() {
  const es = chat.querySelector(".empty-state");
  if (es) es.remove();
}

function addMessage(role, text, asMarkdown = false) {
  const wasNear = nearBottom();
  clearEmptyState();
  const msg = el("div", `msg ${role}`);
  const who = el("div", "who");
  who.textContent = role === "user" ? "You" : role === "tool" ? "Tool" : "Agent";
  const bubble = el("div", "bubble");
  if (asMarkdown) {
    bubble.classList.add("md");
    bubble.innerHTML = renderMarkdown(text ?? "");
  } else {
    bubble.textContent = text ?? "";
  }
  msg.append(who, bubble);
  if (role === "agent") {
    const copy = el("button", "msg-copy");
    copy.textContent = "Copy";
    copy.addEventListener("click", () => {
      navigator.clipboard.writeText(bubble._raw ?? bubble.textContent);
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy"), 1200);
    });
    msg.appendChild(copy);
  }
  chat.appendChild(msg);
  stickScroll(wasNear);
  return bubble;
}

function showEmptyState() {
  chat.innerHTML = "";
  const es = el("div", "empty-state");
  const t = el("div", "es-title"); t.textContent = "Sleepers Researcher";
  const s = el("div"); s.textContent = "Ask a question, research a topic, or build something.";
  es.append(t, s);
  chat.appendChild(es);
}

// ---------- activity log ----------
const activityList = $("#activity-list");
function logActivity(kind, detail) {
  const a = el("div", "act");
  const time = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  a.innerHTML = `<span class="act-kind"></span> <span class="act-detail"></span><br><span class="act-time"></span>`;
  a.querySelector(".act-kind").textContent = kind;
  a.querySelector(".act-detail").textContent = detail ?? "";
  a.querySelector(".act-time").textContent = time;
  activityList.appendChild(a);
  activityList.scrollTop = activityList.scrollHeight;
}

// ---------- send / stop ----------
async function send() {
  const input = $("#input");
  const text = input.value.trim();
  if (!text || state.busy) return;

  if (text.startsWith("/")) { handleSlash(text); input.value = ""; autoSize(); return; }

  addMessage("user", text);
  input.value = ""; autoSize();
  setBusy(true);

  if (invoke) {
    try {
      await invoke("chat_send", { message: text, backend: state.backend });
    } catch (e) {
      addMessage("agent", "Error: " + e);
      setBusy(false);
    }
  } else {
    logActivity("preview", "no Tauri backend — echoing");
    const b = addMessage("agent", "");
    b._raw = "Backend not connected (browser preview).";
    b.textContent = b._raw;
    setBusy(false);
  }
}

async function stop() {
  if (!invoke) return;
  try { await invoke("cancel_agent"); logActivity("agent", "stop requested"); } catch {}
}

function setBusy(b) {
  state.busy = b;
  const btn = $("#send-btn");
  btn.textContent = b ? "Stop" : "Send";
  btn.classList.toggle("primary", !b);
  btn.classList.toggle("ghost", b);
}

// ---------- slash commands ----------
function handleSlash(cmd) {
  const [name, ...rest] = cmd.slice(1).split(/\s+/);
  const arg = rest.join(" ");
  switch (name) {
    case "yolo":
      $("#yolo-toggle").checked = !$("#yolo-toggle").checked;
      pushPermissions();
      addMessage("tool", "YOLO mode " + ($("#yolo-toggle").checked ? "enabled — confirmations OFF" : "disabled"));
      break;
    case "autoapprove":
      if (arg === "file_write") $("#auto-file-write").checked = true;
      else if (arg === "code_exec") $("#auto-code-exec").checked = true;
      pushPermissions();
      addMessage("tool", "Auto-approve set for: " + arg);
      break;
    case "new":
      newChat();
      break;
    default:
      addMessage("tool", "Unknown command: /" + name);
  }
}

function pushPermissions() {
  const perms = {
    yolo: $("#yolo-toggle").checked,
    autoFileWrite: $("#auto-file-write").checked,
    autoCodeExec: $("#auto-code-exec").checked,
  };
  if (invoke) invoke("set_permissions", { perms }).catch(() => {});
}

// ---------- confirmation modal ----------
let pendingPermId = null;
function showConfirm({ id, kind, title, sub, body }) {
  pendingPermId = id;
  $("#modal-title").textContent = title || "Confirm action";
  $("#modal-sub").textContent = sub || "";
  $("#modal-body").textContent = body || "";
  $("#modal-overlay").classList.remove("hidden");
  $("#modal-approve").focus();
  logActivity("permission", kind + " awaiting confirmation");
}
function resolveConfirm(approved) {
  if (pendingPermId == null) return;
  $("#modal-overlay").classList.add("hidden");
  if (invoke) invoke("permission_respond", { id: pendingPermId, approved }).catch(() => {});
  logActivity("permission", approved ? "approved" : "denied");
  pendingPermId = null;
}

// ---------- backend toggle ----------
function setBackend(b) {
  state.backend = b;
  document.querySelectorAll('#backend-seg .seg-btn, #set-backend-seg .seg-btn').forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.backend === b);
  });
  if (invoke) {
    invoke("set_backend", { backend: b })
      .then(() => refreshStatus())
      .catch(() => {});
  } else {
    $("#backend-hint").textContent = BACKEND_HINTS[b];
  }
  logActivity("config", "backend → " + b);
}

// ---------- history / sessions / soul ----------
function renderHistory(messages) {
  if (!messages || messages.length === 0) { showEmptyState(); return; }
  chat.innerHTML = "";
  for (const m of messages) {
    if (m.role === "user") addMessage("user", m.content);
    else {
      const b = addMessage("agent", m.content, true);
      b._raw = m.content;
    }
  }
  scrollToBottom();
  updateJumpButton();
}

function renderSessions(sessions) {
  const list = $("#session-list");
  list.innerHTML = "";
  if (!sessions || sessions.length === 0) return;
  for (const s of sessions) {
    const item = el("div", "session-item" + (s.active ? " active" : ""));
    item.textContent = s.preview;
    item.title = s.preview;
    item.addEventListener("click", () => loadSession(s.id));
    list.appendChild(item);
  }
}

async function loadSession(id) {
  if (!invoke) return;
  try {
    const msgs = await invoke("load_session", { id });
    streamingBubble = null;
    renderHistory(msgs);
    await refreshSessions();
    await refreshStatus();
    logActivity("session", "loaded chat #" + id);
  } catch (e) { logActivity("session", "load failed: " + e); }
}

async function newChat() {
  if (invoke) { try { await invoke("new_chat"); } catch {} }
  streamingBubble = null;
  showEmptyState();
  logActivity("session", "new chat started");
  await refreshSessions();
  await refreshStatus();
}

function renderSoul(facts) {
  const list = $("#soul-list");
  if (!facts || facts.length === 0) {
    list.innerHTML = '<div class="soul-empty">Nothing stored yet. Tell the agent about yourself and it will remember.</div>';
    return;
  }
  list.innerHTML = "";
  for (const f of facts) {
    const item = el("div", "soul-item");
    item.textContent = f;
    list.appendChild(item);
  }
}

async function refreshStatus() {
  if (!invoke) return;
  try { applyStatus(await invoke("get_status")); } catch {}
}
async function refreshSessions() {
  if (!invoke) return;
  try { renderSessions(await invoke("list_sessions")); } catch {}
}
async function refreshMemory() {
  await refreshStatus();
  if (!invoke) return;
  try { renderSoul(await invoke("get_soul")); } catch {}
}

function applyStatus(s) {
  if (!s) return;
  state.memChunks = s.memChunks ?? 0;
  state.contextTokens = s.contextTokens ?? 0;
  $("#mem-status").textContent = state.memChunks + " memories";
  $("#mem-dot").classList.toggle("live", state.memChunks > 0);
  $("#token-usage").textContent = "ctx " + state.contextTokens.toLocaleString() + " / 128k";
  $("#set-context").textContent = "Tokens used: " + state.contextTokens.toLocaleString() + " / 128,000";
  if (s.model) $("#backend-hint").textContent = (BACKEND_HINTS[s.backend] || "") + " · " + s.model;
}

// ---------- textarea autosize ----------
function autoSize() {
  const t = $("#input");
  t.style.height = "auto";
  t.style.height = Math.min(t.scrollHeight, 180) + "px";
}

// ---------- wiring ----------
function wire() {
  $("#send-btn").addEventListener("click", () => (state.busy ? stop() : send()));
  $("#input").addEventListener("input", autoSize);
  $("#input").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
  });

  document.querySelectorAll('#backend-seg .seg-btn, #set-backend-seg .seg-btn').forEach((btn) => {
    btn.addEventListener("click", () => setBackend(btn.dataset.backend));
  });

  chat.addEventListener("scroll", updateJumpButton, { passive: true });
  $("#jump-latest").addEventListener("click", () => scrollToBottom(true));

  // Page Up/Down + Home/End scroll the transcript when not typing.
  document.addEventListener("keydown", (e) => {
    if (e.target === $("#input")) return;
    const page = chat.clientHeight * 0.9;
    if (e.key === "PageDown") { chat.scrollBy({ top: page, behavior: "smooth" }); }
    else if (e.key === "PageUp") { chat.scrollBy({ top: -page, behavior: "smooth" }); }
    else if (e.key === "End") { scrollToBottom(true); }
    else if (e.key === "Home") { chat.scrollTo({ top: 0, behavior: "smooth" }); }
  });

  $("#toggle-activity").addEventListener("click", () => {
    $("#activity").classList.toggle("collapsed");
    $("#app").classList.toggle("activity-open");
  });
  $("#clear-activity").addEventListener("click", () => { activityList.innerHTML = ""; });

  $("#settings-btn").addEventListener("click", async () => {
    $("#settings-overlay").classList.remove("hidden");
    await refreshMemory();
  });
  $("#settings-close").addEventListener("click", () => $("#settings-overlay").classList.add("hidden"));
  $("#new-chat-btn").addEventListener("click", newChat);

  $("#modal-approve").addEventListener("click", () => resolveConfirm(true));
  $("#modal-deny").addEventListener("click", () => resolveConfirm(false));

  // Global keyboard: modal Enter/Esc, settings Esc, Ctrl+N new chat.
  document.addEventListener("keydown", (e) => {
    const modalOpen = !$("#modal-overlay").classList.contains("hidden");
    if (modalOpen) {
      if (e.key === "Enter") { e.preventDefault(); resolveConfirm(true); }
      else if (e.key === "Escape") { e.preventDefault(); resolveConfirm(false); }
      return;
    }
    if (e.key === "Escape" && !$("#settings-overlay").classList.contains("hidden")) {
      $("#settings-overlay").classList.add("hidden");
    }
    if (e.key === "n" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); newChat(); }
  });

  ["yolo-toggle", "auto-file-write", "auto-code-exec"].forEach((id) =>
    $("#" + id).addEventListener("change", pushPermissions));

  $("#ingest-btn").addEventListener("click", async () => {
    if (!invoke) { logActivity("ingest", "unavailable in browser preview"); return; }
    const btn = $("#ingest-btn");
    btn.disabled = true; btn.textContent = "Ingesting…";
    try {
      const res = await invoke("ingest_folder");
      if (res) { logActivity("ingest", res); addMessage("tool", res); }
    } catch (e) {
      if (String(e) !== "cancelled") logActivity("ingest", "error: " + e);
    } finally {
      btn.disabled = false; btn.textContent = "Ingest folder…";
      await refreshMemory();
    }
  });

  // Delegated: copy code blocks, open links externally.
  chat.addEventListener("click", (e) => {
    const copyBtn = e.target.closest(".code-copy");
    if (copyBtn) {
      const code = copyBtn.closest(".code-wrap").querySelector("code");
      navigator.clipboard.writeText(code.textContent);
      copyBtn.textContent = "Copied";
      setTimeout(() => (copyBtn.textContent = "Copy"), 1200);
      return;
    }
    const link = e.target.closest("a.md-link");
    if (link) {
      e.preventDefault();
      const url = link.dataset.url;
      if (TAURI?.opener?.openUrl) TAURI.opener.openUrl(url);
      else logActivity("link", url);
    }
  });
}

// ---------- backend events ----------
async function subscribe() {
  if (!listen) return;
  await listen("agent:token", (e) => {
    const wasNear = nearBottom();
    if (!streamingBubble) {
      streamingBubble = addMessage("agent", "");
      streamingBubble._raw = "";
      streamingBubble.classList.add("cursor-blink");
    }
    streamingBubble._raw += e.payload;
    streamingBubble.textContent = streamingBubble._raw;
    stickScroll(wasNear);
  });
  await listen("agent:tool", (e) => addMessage("tool", e.payload.text));
  await listen("agent:done", async () => {
    if (streamingBubble) {
      const wasNear = nearBottom();
      streamingBubble.classList.remove("cursor-blink");
      // Re-render the completed answer as markdown.
      streamingBubble.classList.add("md");
      streamingBubble.innerHTML = renderMarkdown(streamingBubble._raw || "");
      stickScroll(wasNear);
    }
    streamingBubble = null;
    setBusy(false);
    await refreshStatus();
    await refreshSessions();
  });
  await listen("activity:log", (e) => logActivity(e.payload.kind, e.payload.detail));
  await listen("permission:request", (e) => showConfirm(e.payload));
  await listen("memory:update", () => refreshMemory());
}

// ---------- boot ----------
async function boot() {
  wire();
  setBackend("gemini");
  await subscribe();
  if (invoke) {
    try { renderHistory(await invoke("get_history")); } catch { showEmptyState(); }
    await refreshSessions();
    await refreshMemory();
  } else {
    showEmptyState();
    logActivity("startup", "browser preview mode");
  }
}

boot();

// Exposed for debugging/inspection (harmless; no effect on app behaviour).
window.__sr = { renderMarkdown, addMessage };
