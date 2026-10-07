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

function render(snap) {
  const list = $("windows");
  list.replaceChildren();
  if (snap.usage) {
    snap.usage.windows.forEach((w) => list.append(card(w)));
  } else if (!snap.error) {
    list.innerHTML = '<p class="muted">불러오는 중…</p>';
  }
  const err = $("error");
  err.hidden = !snap.error;
  err.textContent = snap.error
    ? (snap.usage ? "최신 값이 아니에요. " : "") + snap.error
    : "";
  $("checked").textContent = snap.checked_at
    ? `마지막 확인: ${new Date(snap.checked_at).toLocaleTimeString("ko-KR")} · 2분마다 자동 갱신`
    : "";
}

$("refresh").addEventListener("click", () => invoke("refresh_usage"));
listen("usage-updated", (e) => render(e.payload));
invoke("get_usage").then(render);
