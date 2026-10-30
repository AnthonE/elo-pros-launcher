// The line to the hub on this machine. Every page holds one: it carries the
// stage's key from the page's own url, reconnects on its own, and is the only
// place a page talks to anything.

export const KEY = new URLSearchParams(location.search).get('k') || '';

export function fileUrl(path) {
  return path + (path.includes('?') ? '&' : '?') + 'k=' + encodeURIComponent(KEY);
}

export async function post(path, body, type = 'application/json') {
  const r = await fetch(fileUrl(path), {
    method: 'POST',
    headers: { 'Content-Type': type },
    body: type === 'application/json' ? JSON.stringify(body) : body,
  });
  let out = {};
  try { out = await r.json(); } catch (_) { out = { t: 'error', why: r.status + ' ' + r.statusText }; }
  if (!r.ok && out.t !== 'error') out = { t: 'error', why: String(r.status) };
  return out;
}

// connect({role, name, onEvent, onStatus}) -> {send}
export function connect({ role, name = '', onEvent, onStatus = () => {} }) {
  let ws = null;
  let wait = 500;
  let queue = [];
  const open = () => {
    const url = `ws://${location.host}/ws?k=${encodeURIComponent(KEY)}&role=${role}&name=${encodeURIComponent(name)}`;
    ws = new WebSocket(url);
    ws.onopen = () => {
      wait = 500;
      onStatus(true);
      ws.send(JSON.stringify({ op: 'hello', role, name }));
      for (const m of queue.splice(0)) ws.send(m);
    };
    ws.onmessage = (e) => {
      let v;
      try { v = JSON.parse(e.data); } catch (_) { return; }
      onEvent(v);
    };
    ws.onclose = () => {
      onStatus(false);
      setTimeout(open, wait);
      wait = Math.min(wait * 2, 8000);
    };
    ws.onerror = () => { try { ws.close(); } catch (_) {} };
  };
  if (!KEY) {
    onStatus(false, 'This page needs its key. Open the url `elo stage url` prints.');
    return { send() {} };
  }
  open();
  return {
    send(op) {
      const m = JSON.stringify(op);
      if (ws && ws.readyState === 1) ws.send(m);
      else if (queue.length < 20) queue.push(m);
    },
  };
}
