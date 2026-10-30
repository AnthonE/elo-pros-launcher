// The desk: a person's hands on the stage. It drives the hub exactly as an
// agent does (the same ops), plus what only a person here does: import a
// model, set how it looks and sounds, and keep a hand on chat.

import { connect, post, KEY, fileUrl } from './link.js';

const $ = (id) => document.getElementById(id);
const el = (tag, { dataset, ...props } = {}, kids = []) => {
  const e = Object.assign(document.createElement(tag), props);
  Object.assign(e.dataset, dataset || {});
  for (const k of kids) e.append(k);
  return e;
};
const LOOKS = ['viewer', 'left', 'right', 'up', 'down', 'around'];
const LAYOUTS = ['corner', 'center', 'full', 'hidden', 'brb'];
const ORIGIN = 'https://elopros.com';

let config = {};
let live = {};
let chat = [];
let feed = [];
let replyTo = null;
// Lines the stage is saying now, so the ears know to cut in and to listen
// harder while her voice may be in the room.
const sayingNow = new Set();

$('preview').src = `/stage?k=${encodeURIComponent(KEY)}&preview=1&bg=check`;

const hub = connect({
  role: 'desk',
  name: 'desk',
  onEvent(ev) {
    if (ev.t === 'state') {
      config = ev.config || {};
      live = ev.live || {};
      chat = ev.chat || chat;
      feed = ev.feed || feed;
      render();
      earsRender();
      feedList();
      hands(ev.clients);
      chatList();
    } else if (ev.t === 'chat') {
      chat.push(ev.msg);
      if (chat.length > 120) chat.shift();
      chatList();
    } else if (ev.t === 'chat_state') {
      for (const m of chat) if (ev.ids.includes(m.id)) m.state = ev.state;
      chatList();
    } else if (ev.t === 'chat_dropped') {
      chat.push({ dropped: true, from: ev.from, why: ev.why, id: 'd' + Date.now() + Math.random() });
      if (chat.length > 120) chat.shift();
      chatList();
    } else if (ev.t === 'feed') {
      feed.push(ev.item);
      if (feed.length > 60) feed.shift();
      feedList();
    } else if (ev.t === 'game_status') {
      live.game_status = ev.status;
      earsRender();
    } else if (ev.t === 'say') {
      sayingNow.add(ev.id);
      // No stage may be open to report it heard.
      setTimeout(() => sayingNow.delete(ev.id), 60000);
    } else if (ev.t === 'spoken') {
      sayingNow.delete(ev.id);
    } else if (ev.t === 'hush') {
      sayingNow.clear();
    } else if (ev.t === 'live') {
      liveRender(ev.live);
    } else if (ev.t === 'youtube') {
      yt = { ...yt, ...ev.youtube };
      ytRender();
    } else if (ev.t === 'upload') {
      yt.uploads = { ...(yt.uploads || {}), [ev.file]: ev.upload };
      recsRender(recs);
    } else if (ev.t === 'chat_status') {
      live.chat_status = ev.status;
      chatStatus();
    } else if (ev.t === 'clients') {
      hands(ev.clients);
    } else if (ev.t === 'layout' || ev.t === 'look' || ev.t === 'expression') {
      live[{ layout: 'layout', look: 'look', expression: 'expression' }[ev.t]] = ev.name || ev.at;
      marks();
    } else if (ev.t === 'error') {
      toast(ev.why);
    }
  },
  onStatus(up, why) {
    $('conn').textContent = why ? 'no key' : up ? 'live' : 'reconnecting';
    $('conn').className = 'pill ' + (up ? 'on' : 'off');
    if (why) toast(why, 0);
  },
});

// The faces list comes back after the stage loads the model; ask again then.
setInterval(() => refreshFaces(), 3000);
async function refreshFaces() {
  try {
    const r = await fetch(`/state?k=${encodeURIComponent(KEY)}`);
    if (!r.ok) return;
    const s = await r.json();
    const before = JSON.stringify(live.expressions || []);
    live.expressions = s.live.expressions;
    if (JSON.stringify(live.expressions) !== before) faces();
    hands(s.clients);
    // ElevenLabs' count for today moves with every line.
    if (s.config?.voice?.kind === 'elevenlabs' && config.voice?.used_today !== s.config.voice.used_today) {
      config.voice = s.config.voice;
      voiceRender();
    }
  } catch (_) {}
}

function hands(clients = []) {
  const box = $('hands');
  box.textContent = '';
  const drivers = clients.filter((c) => c.role === 'drive');
  const views = clients.filter((c) => c.role === 'view' && c.name !== 'desk preview');
  for (const d of drivers) box.append(el('span', { className: 'pill on', textContent: (d.name || 'a driver') + ' driving' }));
  box.append(el('span', {
    className: 'pill ' + (views.length ? 'on' : 'off'),
    textContent: views.length ? `on ${views.length} screen${views.length > 1 ? 's' : ''}` : 'not in OBS yet',
  }));
}

function render() {
  $('who-name').textContent = config.name || '';
  const f = $('look');
  f.who.value = config.name || '';
  f.label.value = config.label ?? '';
  f.accent.value = config.accent || '#b8ff38';
  f.agent.value = config.agent || '';
  const m = config.model;
  $('model-now').textContent = !m ? 'No model yet.'
    : m.kind === 'vrm' || m.kind === 'glb' ? `Now: ${m.file}`
    : `Now: pictures — ${Object.keys(m.files || {}).filter((k) => !k.endsWith('-talk')).join(', ')}`;
  voiceRender();
  chatSettings();
  chatStatus();
  const bv = config.browser_voice || {};
  const vf = $('voice');
  vf.rate.value = bv.rate || 1;
  vf.pitch.value = bv.pitch ?? 1;
  voices(bv.name || '');
  faces();
  moves();
  buttons();
  obs();
}

function faces() {
  const list = live.expressions?.length ? live.expressions
    : ['neutral', 'happy', 'angry', 'sad', 'relaxed', 'surprised'];
  const box = $('faces');
  box.textContent = '';
  box.append(el('span', { className: 'k', textContent: 'face' }));
  for (const n of list) {
    box.append(el('button', { type: 'button', textContent: n, dataset: { face: n },
      onclick: () => hub.send({ op: 'expression', name: n }) }));
  }
  const sel = $('say-face');
  const keep = sel.value;
  sel.textContent = '';
  sel.append(el('option', { value: '', textContent: 'face' }));
  for (const n of list) sel.append(el('option', { value: n, textContent: n }));
  sel.value = list.includes(keep) ? keep : '';
  marks();
}

// A glb pack's moves, by use: each plays once on the stage. Idle, talk and
// dance also play on their own (at rest, while a line plays, on brb).
function moves() {
  const box = $('moves');
  const list = config.model?.kind === 'glb' ? Object.keys(config.model.clips || {}) : [];
  box.hidden = !list.length;
  box.textContent = '';
  box.append(el('span', { className: 'k', textContent: 'move' }));
  for (const n of list) {
    box.append(el('button', { type: 'button', textContent: n, onclick: () => hub.send({ op: 'move', name: n }) }));
  }
}

function buttons() {
  const mk = (box, list, op, key) => {
    $(box).textContent = '';
    for (const n of list) {
      $(box).append(el('button', { type: 'button', textContent: n, dataset: { [key]: n },
        onclick: () => hub.send({ op, [op === 'look' ? 'at' : 'name']: n }) }));
    }
  };
  mk('looks', LOOKS, 'look', 'look');
  mk('layouts', LAYOUTS, 'layout', 'layout');
  marks();
}

function marks() {
  for (const b of document.querySelectorAll('[data-face]')) b.classList.toggle('on', b.dataset.face === live.expression);
  for (const b of document.querySelectorAll('[data-look]')) b.classList.toggle('on', b.dataset.look === (live.look || 'viewer'));
  for (const b of document.querySelectorAll('[data-layout]')) b.classList.toggle('on', b.dataset.layout === (live.layout || 'corner'));
}

function obs() {
  const box = $('obs');
  box.textContent = '';
  const add = (what, url, note) => {
    const row = el('div', { className: 'link' }, [
      el('b', { textContent: what }),
      el('code', { textContent: url }),
      el('button', { type: 'button', textContent: 'copy', onclick: () => copy(url) }),
    ]);
    if (note) row.append(el('span', { className: 'note', textContent: note }));
    box.append(row);
  };
  add('the stage', `${location.origin}/stage?k=${KEY}`, 'the avatar, captions and cards, on a clear background');
  const a = /^0x[0-9a-fA-F]{40}$/.test(config.agent || '') ? config.agent : '';
  if (a) {
    add('agent cam', `${ORIGIN}/games/gates/play/?spectate=${a}&obs=1`, 'Gates, watching your agent (OBS 29.1 or later)');
    add('agent card', `${ORIGIN}/overlay/?kind=agent&agent=${a}`, 'what it is doing and why, 540×420');
  } else {
    box.append(el('p', { className: 'note', textContent: 'Add your agent’s wallet above for its game cam and card.' }));
  }
  $('curl').textContent =
    `curl -s ${location.origin}/op?k=${KEY} -H 'Content-Type: application/json' \\\n  -d '{"op":"say","text":"found it."}'`;
}

function voices(want) {
  const sel = $('voice').voice;
  const list = 'speechSynthesis' in window ? speechSynthesis.getVoices() : [];
  sel.textContent = '';
  sel.append(el('option', { value: '', textContent: list.length ? 'default' : 'none in this browser' }));
  for (const v of list) sel.append(el('option', { value: v.name, textContent: `${v.name} (${v.lang})` }));
  sel.value = list.some((v) => v.name === want) ? want : '';
}
if ('speechSynthesis' in window) speechSynthesis.onvoiceschanged = () => voices(config.browser_voice?.name || '');

// ── chat ────────────────────────────────────────────────────────────────────

const BADGES = { broadcaster: 'host', mod: 'mod', vip: 'vip', sub: 'sub' };

function chatList() {
  const box = $('chat-list');
  const atEnd = box.scrollTop + box.clientHeight >= box.scrollHeight - 8;
  box.textContent = '';
  if (!chat.length) {
    box.append(el('li', { className: 'empty', textContent: 'Viewers’ messages show up here, and what your agent has read.' }));
  }
  for (const m of chat.slice(-80)) {
    if (m.dropped) {
      box.append(el('li', { className: 'msg dropped', textContent: `dropped from ${m.from}: ${m.why}` }));
      continue;
    }
    const acts = el('span', { className: 'acts' });
    if (m.state === 'held') {
      acts.append(el('button', { type: 'button', textContent: 'pass on', onclick: () => hub.send({ op: 'chat_mod', action: 'approve', id: m.id }) }));
    }
    if (m.state === 'held' || m.state === 'new') {
      acts.append(el('button', { type: 'button', className: 'quiet', textContent: 'drop', onclick: () => hub.send({ op: 'chat_mod', action: 'drop', id: m.id }) }));
    }
    if (m.state === 'new' || m.state === 'read') {
      acts.append(el('button', { type: 'button', className: 'quiet', textContent: 'answer', onclick: () => reply(m) }));
    }
    const label = { new: 'waiting', held: 'held', read: 'read by agent', dropped: 'taken down' }[m.state] || m.state;
    box.append(el('li', { className: 'msg st-' + m.state }, [
      el('span', { className: 'plat', textContent: m.platform }),
      el('b', { textContent: m.from }),
      ...(BADGES[m.badge] ? [el('span', { className: 'badge', textContent: BADGES[m.badge] })] : []),
      el('span', { className: 'txt', textContent: m.text }),
      el('span', { className: 'st', textContent: label }),
      acts,
    ]));
  }
  if (atEnd) box.scrollTop = box.scrollHeight;
  const mode = config.chat?.mode || 'open';
  for (const b of document.querySelectorAll('#chat-mode [data-mode]')) b.classList.toggle('on', b.dataset.mode === mode);
}

// ── you and the game: the ears, and her body followed on the island ─────────

const EARS_NOTE = {
  none: 'To hear you she needs speech-to-text: add your ElevenLabs key under Voice (the same key does both), '
    + 'or set a local one with `elo stage ears -- <command> {in}`.',
  elevenlabs: 'What you say goes to ElevenLabs speech-to-text with your key, and nowhere else. '
    + 'Wear headphones so she doesn’t hear herself.',
  command: 'What you say is heard by your own command on this machine. Wear headphones so she doesn’t hear herself.',
};

function earsRender() {
  const e = config.ears || {};
  const engine = e.engine === 'none' && e.elevenlabs_key ? 'none-but-key' : e.engine || 'none';
  $('ears-note').textContent = engine === 'none-but-key'
    ? 'Your ElevenLabs key can hear you too: press talk to her to use it.'
    : EARS_NOTE[e.engine || 'none'];
  const st = $('ears-state');
  st.textContent = mic ? (mic.talking ? 'hearing you' : 'listening') : e.on ? 'press talk to her' : 'not listening';
  st.className = 'pill ' + (mic ? 'on' : 'off');
  $('ears-go').classList.toggle('on', !!mic);
  $('ears-go').textContent = mic ? 'stop listening' : 'talk to her';
  const f = $('follow');
  f.checked = !!e.follow;
  const g = live.game_status;
  f.parentElement.title = !config.agent ? 'set the agent wallet under Name and look first'
    : g && g.state ? `${g.state}${g.why ? ': ' + g.why : ''}` : '';
}

function feedList() {
  const box = $('feed-list');
  const atEnd = box.scrollTop + box.clientHeight >= box.scrollHeight - 8;
  box.textContent = '';
  if (!feed.length) {
    box.append(el('li', { className: 'empty', textContent: 'What you say to her, and what happens to her in the game, show up here.' }));
  }
  for (const m of feed.slice(-40)) {
    box.append(el('li', { className: `msg feed-${m.kind} st-${m.state}` }, [
      el('span', { className: 'plat', textContent: m.kind === 'owner' ? 'you' : 'game' }),
      el('span', { className: 'txt', textContent: m.text }),
      el('span', { className: 'st', textContent: m.state === 'read' ? 'heard by agent' : 'waiting' }),
    ]));
  }
  if (atEnd) box.scrollTop = box.scrollHeight;
}

$('follow').onchange = async (e) => {
  if (e.target.checked && !config.agent) {
    e.target.checked = false;
    return toast('Set the agent wallet under Name and look first.');
  }
  const r = await post('/config', { game: { follow: e.target.checked } });
  if (r.t === 'error') toast(r.why);
};

$('ears-go').onclick = async () => {
  if (mic) return micStop();
  const e = config.ears || {};
  if (e.engine === 'none') {
    if (!e.elevenlabs_key) return toast('Add your ElevenLabs key under Voice first, or set a local command.', 5000);
    const r = await post('/ears/engine', { engine: 'elevenlabs' });
    if (r.t === 'error') return toast(r.why);
  }
  if (!e.on) {
    const r = await post('/config', { ears: { on: true } });
    if (r.t === 'error') return toast(r.why);
  }
  try {
    await micStart();
  } catch (err) {
    toast('The microphone didn’t open: ' + (err && err.message ? err.message : err), 5000);
  }
  earsRender();
};

// The microphone, cut into utterances at a pause. Constants match what Gates'
// agents use for players' voices (`agent_voice.rs`).
const RATE = 16000;
const PAUSE_S = 0.7;
const MIN_S = 0.35;
const MAX_S = 15;
let mic = null;

async function micStart() {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
  });
  const ctx = new AudioContext();
  const src = ctx.createMediaStreamSource(stream);
  const node = ctx.createScriptProcessor(2048, 1, 1);
  const sink = ctx.createGain();
  sink.gain.value = 0;
  src.connect(node);
  node.connect(sink);
  sink.connect(ctx.destination);
  const frame = 2048 / ctx.sampleRate;
  const m = { stream, ctx, node, talking: false, chunks: [], loud: 0, quiet: 0, pre: null };
  node.onaudioprocess = (ev) => {
    const x = new Float32Array(ev.inputBuffer.getChannelData(0));
    let sum = 0;
    for (const v of x) sum += v * v;
    const rms = Math.sqrt(sum / x.length);
    // Her own voice may reach the mic: while she talks, only a clear voice counts.
    const thr = sayingNow.size ? 0.04 : 0.015;
    if (!m.talking) {
      if (rms > thr) {
        if (!m.chunks.length && m.pre) m.chunks.push(m.pre);
        m.chunks.push(x);
        if (++m.loud >= 2) {
          m.talking = true;
          m.quiet = 0;
          if (sayingNow.size) hub.send({ op: 'hush' });
          earsRender();
        }
      } else {
        m.loud = 0;
        m.chunks = [];
        m.pre = x;
      }
      return;
    }
    m.chunks.push(x);
    m.quiet = rms > thr * 0.6 ? 0 : m.quiet + frame;
    const len = m.chunks.length * frame;
    if (m.quiet >= PAUSE_S || len >= MAX_S) {
      const chunks = m.chunks;
      m.talking = false;
      m.chunks = [];
      m.loud = 0;
      if (len - m.quiet >= MIN_S) send(chunks, ctx.sampleRate);
      m.quiet = 0;
      earsRender();
    }
  };
  mic = m;
}

function micStop() {
  if (!mic) return;
  mic.node.disconnect();
  mic.stream.getTracks().forEach((t) => t.stop());
  mic.ctx.close().catch(() => {});
  mic = null;
  post('/config', { ears: { on: false } });
  earsRender();
}

// 16 kHz, 16-bit mono WAV: small, and what every speech-to-text takes.
function wav(chunks, rate) {
  const n = chunks.reduce((a, c) => a + c.length, 0);
  const all = new Float32Array(n);
  let o = 0;
  for (const c of chunks) { all.set(c, o); o += c.length; }
  const step = rate / RATE;
  const out = Math.floor(n / step);
  const buf = new ArrayBuffer(44 + out * 2);
  const v = new DataView(buf);
  const str = (at, s) => { for (let i = 0; i < s.length; i++) v.setUint8(at + i, s.charCodeAt(i)); };
  str(0, 'RIFF'); v.setUint32(4, 36 + out * 2, true); str(8, 'WAVE');
  str(12, 'fmt '); v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
  v.setUint32(24, RATE, true); v.setUint32(28, RATE * 2, true); v.setUint16(32, 2, true); v.setUint16(34, 16, true);
  str(36, 'data'); v.setUint32(40, out * 2, true);
  for (let i = 0; i < out; i++) {
    const a = Math.floor(i * step), b = Math.max(a + 1, Math.floor((i + 1) * step));
    let s = 0;
    for (let j = a; j < b && j < n; j++) s += all[j];
    const x = Math.max(-1, Math.min(1, s / (b - a)));
    v.setInt16(44 + i * 2, x < 0 ? x * 0x8000 : x * 0x7fff, true);
  }
  return new Blob([buf], { type: 'audio/wav' });
}

async function send(chunks, rate) {
  const r = await post('/ears', wav(chunks, rate), 'audio/wav');
  if (r.t === 'error' && !/too short/.test(r.why || '')) toast(r.why, 4000);
}

const SOURCES = { twitch: 'Twitch', youtube: 'YouTube', kick: 'Kick' };

function chatStatus() {
  const st = live.chat_status || {};
  const box = $('chat-status');
  box.textContent = '';
  if ((config.chat?.mode || 'open') === 'off') {
    box.append(el('span', { className: 'pill off', textContent: 'chat off' }));
    return;
  }
  let any = false;
  for (const [k, name] of Object.entries(SOURCES)) {
    const s = st[k];
    if (!s || !s.state) continue;
    any = true;
    const on = s.state === 'reading';
    box.append(el('span', {
      className: 'pill ' + (on ? 'on' : 'off'),
      textContent: on ? name : `${name}: ${s.state}`,
      title: [s.channel, s.why, s.units != null ? `${s.units} of ${s.budget} quota units today` : ''].filter(Boolean).join(' · '),
    }));
  }
  if (!any) box.append(el('span', { className: 'pill off', textContent: 'no channel set' }));
}

function chatSettings() {
  const c = config.chat || {};
  const f = $('chat-set');
  if (f.contains(document.activeElement)) return;
  f.twitch.value = c.twitch || '';
  f.youtube.value = c.youtube || '';
  f.kick.value = c.kick || '';
  $('yt-key').placeholder = config.youtube_key ? 'key saved on this machine — paste a new one to replace it' : 'YouTube API key';
  f.trigger.value = c.trigger || '';
  f.cooldown.value = c.cooldown ?? 15;
  f.block_words.value = (c.block_words || []).join('\n');
  f.block_users.value = (c.block_users || []).join('\n');
  f.links.checked = !!c.links;
  f.bots.checked = c.bots ?? true;
  $('chat-url').textContent =
    `curl -s ${location.origin}/chat?k=${KEY} -H 'Content-Type: application/json' \\\n  -d '{"from":"sam","text":"hi!","platform":"youtube"}'`;
  chatList();
}

function reply(m) {
  replyTo = m;
  $('reply-to').hidden = false;
  $('reply-to').textContent = '';
  $('reply-to').append(
    el('span', { textContent: `↩ ${m.from}: ${m.text.slice(0, 60)}` }),
    el('button', { type: 'button', className: 'quiet', textContent: '×', title: 'not a reply',
      onclick: () => { replyTo = null; $('reply-to').hidden = true; } }),
  );
  $('say-text').focus();
}

$('chat-set').onsubmit = async (e) => {
  e.preventDefault();
  const f = e.target;
  const lines = (t) => t.split(/[\n,]/).map((x) => x.trim()).filter(Boolean);
  const r = await post('/config', { chat: {
    twitch: f.twitch.value.trim(), youtube: f.youtube.value.trim(), kick: f.kick.value.trim(),
    trigger: f.trigger.value.trim(), cooldown: Number(f.cooldown.value) || 0,
    block_words: lines(f.block_words.value), block_users: lines(f.block_users.value),
    links: f.links.checked, bots: f.bots.checked,
  } });
  toast(r.t === 'error' ? r.why : 'saved');
};

for (const b of document.querySelectorAll('#chat-mode [data-mode]')) {
  b.onclick = async () => {
    const r = await post('/config', { chat: { mode: b.dataset.mode } });
    if (r.t === 'error') toast(r.why);
  };
}
$('chat-clear').onclick = () => hub.send({ op: 'chat_mod', action: 'clear' });

$('yt-key-save').onclick = async () => {
  const key = $('yt-key').value.trim();
  if (!key) return;
  const r = await post('/youtube/key', { key });
  $('yt-key').value = '';
  toast(r.t === 'error' ? r.why : 'key saved');
};

$('chat-try').onsubmit = async (e) => {
  e.preventDefault();
  const text = $('ct-text').value.trim();
  if (!text) return;
  const r = await post('/chat', { from: $('ct-from').value.trim() || 'viewer', text, platform: 'test' });
  if (r.t === 'dropped') toast('dropped: ' + r.why);
  else if (r.t === 'error') toast(r.why);
  $('ct-text').value = '';
};

// ── going live ──────────────────────────────────────────────────────────────

let golive = null;

async function liveLoad() {
  try { liveRender(await (await fetch(fileUrl('/live'))).json()); } catch (_) {}
}

function liveRender(g) {
  golive = g;
  const st = $('live-state');
  const on = g.state === 'live';
  st.textContent = g.state || 'off';
  st.className = 'pill ' + (on ? 'on' : 'off');
  $('live-rate').textContent = on && g.bitrate ? g.bitrate : '';
  $('live-go').textContent = g.armed ? 'stop' : 'go live';
  $('live-go').classList.toggle('stop', !!g.armed);
  $('live-ffmpeg').hidden = !!g.ffmpeg;
  const obs = $('live-obs');
  obs.textContent = '';
  for (const [what, val] of [['Server', g.obs.server], ['Stream key', g.obs.key]]) {
    obs.append(el('div', { className: 'link' }, [
      el('b', { textContent: what }), el('code', { textContent: val }),
      el('button', { type: 'button', textContent: 'copy', onclick: () => copy(val) }),
    ]));
  }
  const list = $('live-dests');
  list.textContent = '';
  if (!g.dests.length) list.append(el('li', { className: 'note', textContent: 'No places yet: add Twitch, YouTube, Kick, X…' }));
  for (const d of g.dests) {
    const bad = d.state && d.state.ok === false;
    list.append(el('li', { className: 'dest' + (bad ? ' bad' : '') }, [
      el('input', { type: 'checkbox', checked: d.on, title: 'send here',
        onchange: (e) => livePost('/live/dest', { id: d.id, platform: d.platform, name: d.name, url: d.url, on: e.target.checked }) }),
      el('b', { textContent: d.name }),
      el('code', { textContent: d.key_hint }),
      el('span', { className: 'st', textContent: !d.state ? '' : d.state.ok ? 'sending' : d.state.why }),
      el('button', { type: 'button', className: 'quiet', textContent: 'remove',
        onclick: () => livePost('/live/dest/remove', { id: d.id }) }),
    ]));
  }
  const sel = $('live-add').platform;
  if (!sel.options.length) {
    for (const p of g.platforms) sel.append(el('option', { value: p.id, textContent: p.name }));
    sel.onchange = () => liveWhere();
    liveWhere();
  }
  $('live-record').checked = g.record !== false;
  recsRender(g.recordings || []);
}

// ── recordings, and posting one to YouTube ──────────────────────────────────

let yt = { connected: false, client: false, uploads: {} };
let recs = [];

function recsRender(list) {
  recs = list;
  const box = $('live-recs');
  box.textContent = '';
  for (const r of recs.slice(0, 6)) {
    const up = yt.uploads?.[r.file];
    const title = el('input', { placeholder: r.file.replace(/\.mkv$/, ''), maxLength: 100, className: 'grow' });
    const privacy = el('select', {}, ['private', 'unlisted', 'public'].map((p) => el('option', { value: p, textContent: p })));
    const status = el('span', { className: 'note' });
    if (up?.state === 'uploading') status.textContent = up.total ? ` ${Math.floor(100 * up.sent / up.total)}%` : ' starting…';
    else if (up?.state === 'done') status.append(' posted: ', el('a', { href: up.url, target: '_blank', rel: 'noopener', textContent: up.url }));
    else if (up?.state === 'failed') status.textContent = ' ' + up.why;
    const post = el('button', { type: 'button', textContent: 'post to YouTube', disabled: !yt.connected || up?.state === 'uploading',
      title: yt.connected ? '' : 'connect YouTube below first',
      onclick: async () => {
        const r2 = await post('/youtube/upload', { file: r.file, title: title.value.trim(), privacy: privacy.value });
        if (r2.t === 'error') toast(r2.why);
      } });
    box.append(el('li', { className: 'rec' }, [
      el('div', { className: 'row' }, [el('code', { textContent: r.file }),
        el('span', { className: 'note', textContent: ` ${(r.bytes / 1e6).toFixed(1)} MB` }), status]),
      el('div', { className: 'row' }, [title, privacy, post]),
    ]));
  }
}

function ytRender() {
  $('yt-client').hidden = yt.client && yt.connected;
  $('yt-connect').hidden = !yt.client || yt.connected;
  $('yt-forget').hidden = !yt.connected;
  $('yt-state').textContent = yt.connected ? 'YouTube connected' : yt.client ? '' : '';
  recsRender(recs);
}

async function ytLoad() {
  try {
    const v = await (await fetch(fileUrl('/youtube'))).json();
    yt = { ...yt, ...v };
    recs = v.recordings || recs;
    ytRender();
  } catch (_) {}
}

$('yt-client').onsubmit = async (e) => {
  e.preventDefault();
  const r = await post('/youtube/client', { id: e.target.id.value.trim(), secret: e.target.secret.value.trim() });
  if (r.t === 'error') return toast(r.why);
  e.target.secret.value = '';
  yt.client = true;
  ytRender();
  toast('saved: now connect');
};
$('yt-connect').onclick = async () => {
  const r = await post('/youtube/connect', {});
  if (r.t === 'error') return toast(r.why);
  window.open(r.url, '_blank', 'noopener');
  toast('finish on Google’s page, then come back');
};
$('yt-forget').onclick = async () => { await post('/youtube/forget', {}); ytLoad(); };
ytLoad();

function liveWhere() {
  const p = golive.platforms.find((x) => x.id === $('live-add').platform.value);
  $('live-where').textContent = p ? p.where : '';
  $('live-add').url.value = p?.url || '';
  $('live-url-row').hidden = !!p?.url;
}

async function livePost(path, body) {
  const r = await post(path, body);
  if (r.live) liveRender(r.live);
  if (r.t === 'error') toast(r.why);
  return r;
}

$('live-add').onsubmit = async (e) => {
  e.preventDefault();
  const f = e.target;
  const r = await livePost('/live/dest', { platform: f.platform.value, url: f.url.value.trim(), key: f.key.value.trim() });
  if (r.t !== 'error') { f.key.value = ''; toast('added'); liveWhere(); }
};
$('live-go').onclick = () => livePost(golive?.armed ? '/live/stop' : '/live/start', {});
$('live-record').onchange = (e) => livePost('/live/record', { on: e.target.checked });
liveLoad();

// ── voice ───────────────────────────────────────────────────────────────────

let voiceShown = null;
let elVoices = null;

function voiceRender() {
  const v = config.voice || { kind: 'page' };
  const now = {
    page: 'Speaking with the browser’s voice. OBS may not have one: ElevenLabs or a command plays from the stage itself.',
    command: 'Speaking with the command set on this machine (elo stage voice).',
    elevenlabs: `Speaking with ElevenLabs: ${v.voice_name || 'a voice'}, ${v.model}. ` +
      `${v.used_today || 0}${v.daily_chars ? ' of ' + v.daily_chars : ''} characters today.`,
  }[v.kind] || '';
  $('voice-now').textContent = now;
  showVoice(voiceShown || v.kind);
  const pick = $('el-pick');
  const models = v.models || [];
  if (pick.model.options.length !== models.length) {
    pick.model.textContent = '';
    for (const m of models) pick.model.append(el('option', { value: m.id, textContent: m.about }));
  }
  if (v.kind === 'elevenlabs') {
    pick.model.value = v.model;
    pick.speed.value = v.settings?.speed ?? 1;
    pick.stability.value = v.settings?.stability ?? 0.5;
    pick.daily_chars.value = v.daily_chars || 0;
  }
  $('el-key').key.placeholder = v.key ? 'key saved on this machine — paste a new one to replace it' : 'your ElevenLabs API key';
  pick.hidden = !v.key;
  if (v.key && !elVoices) loadVoices();
}

function showVoice(kind) {
  voiceShown = kind;
  for (const b of document.querySelectorAll('#voice-kind [data-kind]')) b.classList.toggle('on', b.dataset.kind === kind);
  $('vk-elevenlabs').hidden = kind !== 'elevenlabs';
  $('vk-command').hidden = kind !== 'command';
  $('voice').hidden = kind !== 'page';
}
for (const b of document.querySelectorAll('#voice-kind [data-kind]')) b.onclick = () => showVoice(b.dataset.kind);

function fillVoices(list) {
  elVoices = list;
  const sel = $('el-pick').voice_id;
  sel.textContent = '';
  for (const v of list) {
    sel.append(el('option', { value: v.voice_id, textContent: `${v.name}${v.about ? ' — ' + v.about : ''}` }));
  }
  if (config.voice?.voice_id) sel.value = config.voice.voice_id;
  $('el-pick').hidden = false;
}

async function loadVoices() {
  elVoices = [];
  try {
    const r = await (await fetch(fileUrl('/voice/elevenlabs/voices'))).json();
    if (r.t === 'error') { elVoices = null; return toast(r.why); }
    fillVoices(r.voices);
  } catch (_) { elVoices = null; }
}

$('el-key').onsubmit = async (e) => {
  e.preventDefault();
  const key = e.target.key.value.trim();
  if (!key) return;
  toast('checking the key with ElevenLabs…', 0);
  const r = await post('/voice/elevenlabs/key', { key });
  e.target.key.value = '';
  if (r.t === 'error') return toast(r.why);
  fillVoices(r.voices);
  toast(`key saved — ${r.voices.length} voices`);
};

$('el-pick').onsubmit = async (e) => {
  e.preventDefault();
  const f = e.target;
  const pick = (elVoices || []).find((v) => v.voice_id === f.voice_id.value);
  const r = await post('/voice/elevenlabs', {
    voice_id: f.voice_id.value, voice_name: pick?.name || '', model: f.model.value,
    settings: { speed: Number(f.speed.value), stability: Number(f.stability.value) },
    daily_chars: Number(f.daily_chars.value) || 0,
  });
  toast(r.t === 'error' ? r.why : 'voice set');
};
$('el-try').onclick = () => hub.send({ op: 'say', text: 'This is my voice. How do I sound?' });

// ── the controls ────────────────────────────────────────────────────────────

$('say').onsubmit = (e) => {
  e.preventDefault();
  const text = $('say-text').value.trim();
  if (!text) return;
  const op = { op: 'say', text };
  if ($('say-face').value) op.expression = $('say-face').value;
  if (replyTo) op.reply_to = replyTo.id;
  hub.send(op);
  $('say-text').value = '';
  replyTo = null;
  $('reply-to').hidden = true;
};

$('card-form').onsubmit = (e) => {
  e.preventDefault();
  hub.send({ op: 'card', title: $('cf-title').value, body: $('cf-body').value, seconds: 20 });
};
$('clear').onclick = () => hub.send({ op: 'clear' });

$('mute').onchange = () => {
  const m = $('mute').checked ? '&mute=1' : '';
  $('preview').src = `/stage?k=${encodeURIComponent(KEY)}&preview=1&bg=check${m}`;
};

$('look').onsubmit = async (e) => {
  e.preventDefault();
  const f = e.target;
  const r = await post('/config', {
    name: f.who.value, label: f.label.value, accent: f.accent.value, agent: f.agent.value.trim(),
  });
  toast(r.t === 'error' ? r.why : 'saved');
};

$('voice').onsubmit = async (e) => {
  e.preventDefault();
  const f = e.target;
  const r = await post('/config', {
    browser_voice: { name: f.voice.value, rate: Number(f.rate.value), pitch: Number(f.pitch.value) },
  });
  if (r.t === 'error') return toast(r.why);
  const r2 = await post('/voice/page', {});
  toast(r2.t === 'error' ? r2.why : 'using the browser voice');
};

// Import: each file to the hub (it stays on this machine), then one setting.
async function upload(files) {
  files = [...files];
  if (!files.length) return;
  const vrm = files.find((f) => /\.(vrm|glb)$/i.test(f.name));
  const pics = files.filter((f) => /\.(png|webp|gif|jpe?g)$/i.test(f.name));
  if (!vrm && !pics.length) return toast('a .vrm, or pictures');
  const send = vrm ? [vrm] : pics;
  for (const f of send) {
    const name = safeName(f.name);
    if (!name) return toast(`${f.name}: rename it to plain letters, like happy.png`);
    toast(`sending ${name}…`, 0);
    const r = await post('/model/file?file=' + encodeURIComponent(name), f, 'application/octet-stream');
    if (r.t === 'error') return toast(r.why);
  }
  let model;
  if (vrm) {
    model = { kind: /\.glb$/i.test(vrm.name) ? 'glb' : 'vrm', file: safeName(vrm.name) };
  } else {
    const filesBy = {};
    for (const f of pics) filesBy[safeName(f.name).replace(/\.[^.]+$/, '').toLowerCase()] = safeName(f.name);
    const first = Object.keys(filesBy).find((k) => !k.endsWith('-talk')) || Object.keys(filesBy)[0];
    model = { kind: 'png', default: filesBy.neutral ? 'neutral' : first, files: filesBy };
  }
  const r = await post('/config', { model });
  toast(r.t === 'error' ? r.why : 'model set');
}
const safeName = (n) => {
  const s = n.split(/[\\/]/).pop().replace(/\s+/g, '-');
  return /^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/.test(s) ? s : '';
};

$('files').onchange = (e) => upload(e.target.files);
const drop = $('drop');
drop.ondragover = (e) => { e.preventDefault(); drop.classList.add('over'); };
drop.ondragleave = () => drop.classList.remove('over');
drop.ondrop = (e) => { e.preventDefault(); drop.classList.remove('over'); upload(e.dataTransfer.files); };

for (const pre of document.querySelectorAll('pre.copy')) {
  pre.title = 'click to copy';
  pre.onclick = () => copy(pre.textContent);
}

async function copy(text) {
  try { await navigator.clipboard.writeText(text); toast('copied'); } catch (_) { toast('select it and copy'); }
}

let toastTimer = 0;
function toast(text, ms = 2600) {
  let t = document.querySelector('.toast');
  if (!t) document.body.append((t = el('div', { className: 'toast' })));
  t.textContent = text;
  t.classList.add('on');
  clearTimeout(toastTimer);
  if (ms) toastTimer = setTimeout(() => t.classList.remove('on'), ms);
}

addEventListener('keydown', (e) => {
  if (e.target.matches('input, textarea, select')) return;
  if (e.key === '/') { e.preventDefault(); $('say-text').focus(); }
});
