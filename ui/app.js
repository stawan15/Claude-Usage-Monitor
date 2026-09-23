"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const isMac = navigator.userAgent.includes("Mac");

let snapshot = null;
let range = localStorage.getItem("range") === "week" ? "week" : "today";

// ---------- formatting ----------

function tokens(n) {
  if (n >= 1e9) return (n / 1e9).toFixed(2) + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "K";
  return String(Math.round(n));
}

/** "—" when nothing in the totals could be priced, rather than a misleading $0.00. */
function cost(totals) {
  if (totals.pricedRequests === 0) return "—";
  return totals.cost < 10 ? "$" + totals.cost.toFixed(2) : "$" + Math.round(totals.cost);
}

function time(ms) {
  return new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function duration(ms) {
  const minutes = Math.max(0, Math.floor(ms / 60000));
  if (minutes >= 1440) return `${Math.floor(minutes / 1440)}d ${Math.floor((minutes % 1440) / 60)}h`;
  if (minutes >= 60) return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  return `${minutes}m`;
}

/** 300 → "5h", 10080 → "weekly", 43200 → "30d". */
function windowLabel(minutes) {
  if (minutes === 10080) return "weekly";
  if (minutes % 1440 === 0) return `${minutes / 1440}d`;
  if (minutes % 60 === 0) return `${minutes / 60}h`;
  return `${minutes}m`;
}

// ---------- building blocks (textContent only: project names come from disk) ----------

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function row(label, value, detail, emphasized = false) {
  const node = el("div", emphasized ? "row em" : "row");
  node.append(el("span", "label", label), el("span", "value", value));
  if (detail !== undefined) node.append(el("span", "detail", detail));
  return node;
}

function bar(fraction, width = 26) {
  const filled = Math.round(Math.min(Math.max(fraction, 0), 1) * width);
  const node = el("span", "bar");
  node.append(el("span", "fill", "█".repeat(filled)), el("span", "empty", "░".repeat(width - filled)));
  return node;
}

/** `⏺ Title` followed by an indented `⎿` block. */
function fillSection(container, { title, bullet = "accent", trailing, rows }) {
  const head = el("div", "section-head");
  head.append(el("span", `bullet ${bullet}`, "⏺"), el("span", "", title));
  if (trailing) head.append(el("span", "trailing", trailing));

  const body = el("div", "section-body");
  const list = el("div", "rows");
  list.append(...rows);
  body.append(el("span", "elbow", "⎿"), list);

  container.replaceChildren(head, body);
  container.hidden = false;
}

function tabs(container, options, selected, onSelect, extra = []) {
  const buttons = options.map(({ id, label }) => {
    const button = el("button", id === selected ? "selected" : "", label);
    button.addEventListener("click", () => onSelect(id));
    return button;
  });
  container.replaceChildren(...buttons, ...extra);
}

// ---------- render ----------

const spinnerFrames = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];
let spinnerTimer = null;

function setSpinner(active) {
  if (active && !spinnerTimer) {
    let i = 0;
    spinnerTimer = setInterval(() => ($("spinner").textContent = spinnerFrames[i++ % spinnerFrames.length]), 120);
  } else if (!active && spinnerTimer) {
    clearInterval(spinnerTimer);
    spinnerTimer = null;
    $("spinner").textContent = "✻";
  }
}

function render() {
  if (!snapshot) return;
  const s = snapshot;
  const now = Date.now();
  const stats = range === "today" ? s.today : s.week;
  const live = s.lastActivity !== null && now - s.lastActivity < 60000;

  // Header
  setSpinner(!s.loaded || live);
  const status = $("status");
  if (!s.loaded) {
    status.replaceChildren("Reading logs…");
  } else if (s.lastActivity !== null) {
    status.replaceChildren(el("span", live ? "live" : "dim", live ? "● live" : "○ idle"), ` · last activity ${time(s.lastActivity)}`);
  } else {
    status.replaceChildren("No activity in the last 7 days");
  }

  // Tool tabs
  const toolTabs = $("tools");
  toolTabs.hidden = s.tools.length < 2;
  tabs(
    toolTabs,
    [{ id: null, label: "All" }, ...s.tools.map((t) => ({ id: t.id, label: t.shortName }))],
    s.filter,
    async (id) => {
      snapshot = await invoke("set_filter", { filter: id });
      render();
    },
  );

  // Claude 5-hour window
  const block = $("block");
  const claudeInView = s.filter === null || s.filter === "claudeCode";
  const hasClaude = s.tools.some((t) => t.id === "claudeCode");
  if (claudeInView && (hasClaude || s.block)) {
    if (s.block) {
      const length = s.blockHours * 3600000;
      const elapsed = (now - s.block.start) / length;
      const burn = el("span", s.burnPerMinute > 0 ? "accent" : "dim", `${tokens(s.burnPerMinute)} tok/min`);
      const burnRow = row("burn rate", "");
      burnRow.querySelector(".value").replaceWith(burn);
      const progress = el("div", "row");
      progress.append(bar(elapsed), el("span", "dim", `${Math.round(Math.min(elapsed, 1) * 100)}%`));
      fillSection(block, {
        title: `${s.blockHours}-hour window`,
        bullet: "live",
        rows: [
          row("used", tokens(s.block.totals.total), cost(s.block.totals), true),
          progress,
          row("window", `${time(s.block.start)} → ${time(s.block.end)}`),
          row("resets in", duration(s.block.end - now)),
          burnRow,
        ],
      });
    } else {
      fillSection(block, { title: `${s.blockHours}-hour window`, bullet: "dim", rows: [el("span", "dim", "No active window. It starts with your next request.")] });
    }
  } else {
    block.hidden = true;
  }

  // Plan limits reported by the tools themselves (Codex)
  const limits = $("limits");
  if (s.limits.length) {
    fillSection(limits, {
      title: "Plan limits",
      bullet: "warn",
      rows: s.limits.map((l) => {
        const wrap = el("div", "rows");
        const head = row(`${l.toolName} · ${windowLabel(l.windowMinutes)}`, `${Math.round(l.usedPercent)}% used`);
        if (l.usedPercent >= 90) head.querySelector(".value").classList.add("warn");
        const line = el("div", "row");
        line.append(bar(l.usedPercent / 100, 22));
        if (l.resetsAt !== null) line.append(el("span", "dim", `resets ${duration(l.resetsAt - now)}`));
        wrap.append(head, line);
        return wrap;
      }),
    });
  } else {
    limits.hidden = true;
  }

  // Range tabs
  const hint = el("span", "subtle", "tab ⇥");
  tabs($("ranges"), [{ id: "today", label: "Today" }, { id: "week", label: "7 days" }], range, setRange, [el("span", "spacer"), hint]);

  // Usage
  const t = stats.totals;
  fillSection($("usage"), {
    title: "Usage",
    trailing: `${t.requests} req`,
    rows: [
      row("total", tokens(t.total), cost(t), true),
      row("input", tokens(t.input)),
      row("output", tokens(t.output)),
      row("cache write", tokens(t.cacheWrite)),
      row("cache read", tokens(t.cacheRead)),
    ],
  });

  breakdown($("by-tool"), "Tools", s.filter === null && stats.byTool.length > 1 ? stats.byTool : [], 4);
  breakdown($("by-model"), "Models", stats.byModel, 5);
  breakdown($("by-project"), "Projects", stats.byProject, 6);

  const unpriced = $("unpriced");
  unpriced.hidden = s.unpricedModels.length === 0;
  unpriced.textContent = `— no price for ${s.unpricedModels.join(", ")}`;
  unpriced.title = "Add prices to ~/.config/claude-monitor/pricing.json";

  fit();
}

function breakdown(container, title, rows, limit) {
  if (!rows.length) {
    container.hidden = true;
    return;
  }
  fillSection(container, {
    title,
    bullet: "dim",
    rows: rows.slice(0, limit).map((r) => row(r.name, tokens(r.totals.total), cost(r.totals))),
  });
}

function setRange(next) {
  range = next;
  try {
    localStorage.setItem("range", range);
  } catch {}
  render();
}

// ---------- window sizing ----------

let lastHeight = 0;
function fit() {
  requestAnimationFrame(() => {
    const height = Math.ceil($("panel").getBoundingClientRect().height);
    if (height !== lastHeight) {
      lastHeight = height;
      invoke("fit_window", { height });
    }
  });
}

// ---------- footer ----------

async function refreshAutostart() {
  const button = $("autostart");
  try {
    const enabled = await invoke("plugin:autostart|is_enabled");
    $("autostart-box").textContent = enabled ? "[✓]" : "[ ]";
    $("autostart-box").className = enabled ? "checked" : "dim";
    button.dataset.enabled = String(enabled);
  } catch {
    button.disabled = true;
  }
}

$("autostart").addEventListener("click", async () => {
  const enabled = $("autostart").dataset.enabled === "true";
  try {
    await invoke(enabled ? "plugin:autostart|disable" : "plugin:autostart|enable");
  } finally {
    refreshAutostart();
  }
});

$("quit-key").textContent = isMac ? "⌘Q" : "Ctrl+Q";
$("quit").addEventListener("click", () => invoke("quit"));

// ---------- keyboard ----------

document.addEventListener("keydown", (event) => {
  if (event.key === "Tab") {
    event.preventDefault();
    setRange(range === "today" ? "week" : "today");
  } else if (event.key === "Escape") {
    invoke("hide_window");
  } else if (event.key.toLowerCase() === "q" && (isMac ? event.metaKey : event.ctrlKey)) {
    invoke("quit");
  }
});

// ---------- startup ----------

listen("snapshot", (event) => {
  snapshot = event.payload;
  render();
});

listen("panel-shown", () => {
  render();
  refreshAutostart();
});

(async () => {
  snapshot = await invoke("get_snapshot");
  render();
  refreshAutostart();
})();
