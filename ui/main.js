const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

function resetText(iso) {
  if (!iso) return { text: "초기화 시각 미정", at: "" };
  const mins = Math.max(0, Math.round((new Date(iso) - Date.now()) / 60000));
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const m = mins % 60;
  const text = d > 0 ? `${d}일 ${h}시간` : h > 0 ? `${h}시간 ${m}분` : `${m}분`;
  const at = new Date(iso).toLocaleString("ko-KR", {
    month: "numeric", day: "numeric", weekday: "short", hour: "2-digit", minute: "2-digit",
  });
  return { text: `${text} 후 초기화`, at };
}

function color(remaining) {
  if (remaining <= 15) return "var(--low)";
  if (remaining <= 40) return "var(--warn)";
  return "var(--ok)";
}

function card(w) {
  const el = document.createElement("section");
  el.className = "card";
  const row = document.createElement("div");
  row.className = "row";
  const label = document.createElement("span");
  label.className = "label";
  label.textContent = w.label;
  const pct = document.createElement("span");
  pct.className = "pct";
  pct.textContent = `${Math.round(w.remaining_percent)}%`;
  row.append(label, pct);

  const bar = document.createElement("div");
  bar.className = "bar";
  const fill = document.createElement("div");
  fill.className = "fill";
  fill.style.width = `${w.remaining_percent}%`;
  fill.style.background = color(w.remaining_percent);
  bar.append(fill);

  const meta = document.createElement("div");
  meta.className = "muted";
  const reset = resetText(w.resets_at);
  meta.textContent = `${Math.round(w.used_percent)}% 사용 · ${reset.text}`;
  meta.title = reset.at;

  el.append(row, bar, meta);
  return el;
}

function checkedText(at) {
  return `마지막 확인 ${new Date(at).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" })}`;
}

// Fills one provider's cards and error line from `{usage, error}`.
function renderUsage(s, list, err) {
  list.replaceChildren();
  if (s.usage) {
    s.usage.windows.forEach((w) => list.append(card(w)));
  } else if (!s.error) {
    list.innerHTML = '<p class="muted">불러오는 중…</p>';
  }
  err.hidden = !s.error;
  err.textContent = s.error
    ? (s.usage ? "최신 값이 아니에요. " : "") + s.error
    : "";
}

function render(snap) {
  renderUsage(snap, $("windows"), $("error"));
  $("checked").textContent = snap.checked_at
    ? checkedText(snap.checked_at) + (snap.source ? ` · ${snap.source}` : "")
    : "";

  const g = snap.gemini;
  $("gemini").hidden = !g;
  $("gemini-login").hidden = !!g;
  if (g) {
    renderUsage(g, $("gemini-windows"), $("gemini-error"));
    $("gemini-checked").textContent = g.checked_at ? checkedText(g.checked_at) : "";
  }
  const account = $("account");
  account.hidden = false;
  account.textContent = snap.web_logged_in ? "claude.ai 로그아웃" : "claude.ai로 로그인";
  account.onclick = () => invoke(snap.web_logged_in ? "logout" : "login");
}

$("refresh").addEventListener("click", () => invoke("refresh_usage"));
$("gemini-account").addEventListener("click", () => invoke("gemini_logout"));
$("gemini-login").addEventListener("click", () => invoke("gemini_login"));
listen("usage-updated", (e) => render(e.payload));
invoke("get_usage").then(render);
