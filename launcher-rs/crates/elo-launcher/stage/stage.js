// The stage: what OBS shows. It holds the body, speaks each line in order,
// and tells the hub when a line has been heard so an agent never talks over
// itself. Every word shown here is set with textContent: lines are someone's
// words, never markup.

import { connect, fileUrl } from './link.js';
import { makeAvatar } from './avatar.js';

const q = new URLSearchParams(location.search);
const MUTE = q.get('mute') === '1';
const $ = (id) => document.getElementById(id);
const body = document.body;
if (q.get('bg')) body.classList.add('bg-' + q.get('bg').replace(/[^a-z]/g, ''));

let config = {};
let live = {};
let avatar = null;
let modelKey = '';
let loading = null;
let lines = [];
let speaking = false;
let cardTimer = 0;
let captionTimer = 0;
let lineFace = null;
// The line playing now stops through this; `cut` marks that it was stopped.
let stopNow = null;
let cut = false;

const PREVIEW = q.get('preview') === '1';
const hub = connect({
  role: 'view',
  name: PREVIEW ? 'desk preview' : 'stage',
  onEvent,
  onStatus(up, why) {
    if (why) return hint(why, true);
    hint(up ? '' : 'waiting for the stage on this machine…');
  },
});

// On the stream the stage says nothing about itself; the desk's preview does.
function hint(text, always = false) {
  if (!PREVIEW && !always) { if (text) console.info('stage:', text); return; }
  $('hint').textContent = text;
  $('hint').hidden = !text;
}

function onEvent(ev) {
  switch (ev.t) {
    case 'state':
      config = ev.config || {};
      live = ev.live || {};
      applyConfig();
      applyLive();
      break;
    case 'say':
      lines.push(ev);
      if (lines.length > 8) lines.shift();
      if (!speaking) nextLine();
      break;
    case 'expression':
      live.expression = ev.name;
      if (!lineFace) face(ev.name);
      break;
    // Kept in `live` too: a change that lands while the model is still
    // loading is what the model must come up in (it once came up in the
    // layout before, dancing on a stage that said center).
    case 'look':
      live.look = ev.at;
      avatar?.setLook(ev.at);
      break;
    case 'move':
      avatar?.play?.(ev.name);
      break;
    case 'layout':
      live.layout = ev.name;
      layout(ev.name);
      break;
    case 'card':
      card(ev.card);
      break;
    case 'clear':
      card(null);
      caption('');
      break;
    case 'hush':
      hush();
      break;
  }
}

function applyConfig() {
  $('name').textContent = config.name || '';
  $('brb-name').textContent = config.name || '';
  $('label').textContent = config.name ? (config.label ?? 'AI agent') : '';
  if (/^#[0-9a-f]{6}$/i.test(config.accent || '')) body.style.setProperty('--accent', config.accent);
  const key = JSON.stringify(config.model || null);
  if (key !== modelKey) {
    modelKey = key;
    loadModel(config.model);
  }
}

function applyLive() {
  layout(live.layout || 'corner');
  card(live.card && live.card.until * 1000 > Date.now() ? live.card : null);
  avatar?.setLook(live.look || 'viewer');
  if (live.expression) face(live.expression);
}

async function loadModel(model) {
  const mine = (loading = Symbol());
  if (!model) {
    avatar?.dispose();
    avatar = null;
    return hint('no model yet — drop one on the desk');
  }
  try {
    const next = await makeAvatar($('body'), model);
    if (mine !== loading) return next?.dispose();
    avatar?.dispose();
    avatar = next;
    hint('');
    avatar.setLook(live.look || 'viewer');
    avatar.setLayout?.(live.layout || 'corner');
    avatar.setTalking?.(speaking);
    face(live.expression || model.default || 'neutral');
    hub.send({ op: 'ready', expressions: avatar.faces() });
  } catch (e) {
    hint("the model didn't load: " + (e && e.message ? e.message : e));
  }
}

function face(name) {
  if (avatar && name) avatar.setExpression(name);
}

function layout(name) {
  avatar?.setLayout?.(name);
  body.className = body.className.replace(/\blayout-\w+/g, '').trim() + ' layout-' + name;
  $('brb').hidden = name !== 'brb';
}

function card(c) {
  clearTimeout(cardTimer);
  $('card').hidden = !c;
  if (!c) return;
  $('card-title').textContent = c.title || '';
  $('card-body').textContent = c.body || '';
  const left = c.until ? c.until * 1000 - Date.now() : (c.seconds || 10) * 1000;
  cardTimer = setTimeout(() => { $('card').hidden = true; }, Math.max(1000, left));
}

function caption(text) {
  clearTimeout(captionTimer);
  if (!text) {
    $('quote').classList.remove('on');
    return $('caption').classList.remove('on');
  }
  $('caption').textContent = text;
  $('caption').classList.add('on');
}

// ── speaking ────────────────────────────────────────────────────────────────

let audioCtx = null;
function ctx() {
  if (!audioCtx) audioCtx = new (window.AudioContext || window.webkitAudioContext)();
  if (audioCtx.state === 'suspended') audioCtx.resume().catch(() => {});
  return audioCtx;
}
// A desk preview starts with no sound until it is clicked; OBS never needs it.
addEventListener('pointerdown', () => ctx(), { once: true });

async function nextLine() {
  const ev = lines.shift();
  if (!ev) { speaking = false; avatar?.setTalking?.(false); return; }
  speaking = true;
  avatar?.setTalking?.(true);
  clearTimeout(captionTimer);
  quote(ev.quote);
  lineFace = ev.expression || null;
  if (lineFace) face(lineFace);
  const words = ev.text || '';
  const mouth = follower(ev.mouth);
  // A line without shapes moves the mouth with the voice's level instead.
  if (!ev.mouth?.cues?.length) avatar?.setMouth(null);
  try {
    if (ev.audio) await playAudio(ev.audio, words, mouth);
    else await speakBrowser(words, mouth);
  } catch (_) {
    await mouthOnly(words, estimate(words), mouth);
  }
  stopNow = null;
  // Shut between lines: left to the level, the mouth would flap as it eases off.
  avatar?.setMouth('', 0);
  avatar?.setLevel(0);
  hub.send({ op: 'spoken', id: ev.id, cut });
  if (cut) caption('');
  else captionTimer = setTimeout(() => caption(''), 3500);
  cut = false;
  setTimeout(() => {
    if (!speaking || lines.length === 0) { lineFace = null; face(live.expression || config.model?.default || 'neutral'); }
  }, 2500);
  nextLine();
}

// The owner started talking: stop the line now and drop what is queued, each
// reported as cut so the agent knows it wasn't heard to the end.
function hush() {
  for (const ev of lines.splice(0)) hub.send({ op: 'spoken', id: ev.id, cut: true });
  if (speaking && stopNow) {
    cut = true;
    stopNow();
  }
  quote(null);
}

// The viewer's message an answer is for, quoted above it. Only one the agent
// was handed: the hub never quotes a held, dropped or taken-down message.
function quote(q) {
  const el = $('quote');
  if (!q || !q.text) { el.classList.remove('on'); return; }
  $('quote-from').textContent = q.from || '';
  $('quote-text').textContent = q.text;
  el.classList.add('on');
}

const estimate = (text) => Math.min(60, 1.2 + text.length * 0.066);

// ── the mouth ───────────────────────────────────────────────────────────────
// A line's mouth is cues of [start, end, shape, open] in seconds (the hub's
// `stage_voice::mouth`). ElevenLabs' are timed to the audio; a `scale` mouth
// is read from the text and stretched over however long the line takes.

// Shapes show a beat early: a mouth that opens with the sound looks late.
const LEAD = 0.04;

function follower(m) {
  const cues = (m && Array.isArray(m.cues)) ? m.cues : [];
  let i = 0;
  return (t, duration, level = 1) => {
    if (!cues.length) return false;
    if (m.scale && duration > 0 && m.length > 0) t = (t / duration) * m.length;
    t += LEAD;
    if (i > 0 && cues[i - 1][0] > t) i = 0;
    while (i < cues.length && cues[i][1] < t) i++;
    const c = cues[i];
    if (!c || c[0] > t) avatar?.setMouth('', 0);
    else avatar?.setMouth(c[2], Math.min(1, c[3] * (0.55 + 0.6 * level)));
    return true;
  };
}

// Words appear as they are said: the caption fills over the line's length.
function typeOut(text, seconds) {
  const el = $('caption');
  el.classList.add('on');
  const t0 = performance.now();
  const step = () => {
    const p = Math.min(1, (performance.now() - t0) / 1000 / Math.max(0.4, seconds * 0.92));
    // The whole line is laid out from the start, the unsaid part clear, so
    // the bubble never grows under the reader's eye.
    const n = Math.ceil(text.length * p);
    const said = document.createElement('span');
    said.textContent = text.slice(0, n);
    const rest = document.createElement('span');
    rest.className = 'rest';
    rest.textContent = text.slice(n);
    el.replaceChildren(said, rest);
    if (p < 1) requestAnimationFrame(step);
  };
  step();
}

function playAudio(path, text, mouth) {
  return new Promise((resolve, reject) => {
    const a = new Audio(fileUrl(path));
    const c = ctx();
    const src = c.createMediaElementSource(a);
    const an = c.createAnalyser();
    an.fftSize = 1024;
    const gain = c.createGain();
    gain.gain.value = MUTE ? 0 : 1;
    src.connect(an);
    an.connect(gain);
    gain.connect(c.destination);
    const buf = new Float32Array(an.fftSize);
    let on = true;
    const meter = () => {
      if (!on) return;
      an.getFloatTimeDomainData(buf);
      let sum = 0;
      for (const v of buf) sum += v * v;
      const rms = Math.sqrt(sum / buf.length);
      const level = Math.min(1, Math.max(0, (rms - 0.015) * 7));
      avatar?.setLevel(level);
      mouth(a.currentTime, a.duration || estimate(text), level);
      requestAnimationFrame(meter);
    };
    a.addEventListener('loadedmetadata', () => typeOut(text, a.duration || estimate(text)));
    a.addEventListener('ended', () => { on = false; src.disconnect(); resolve(); });
    stopNow = () => { if (on) { on = false; a.pause(); src.disconnect(); resolve(); } };
    a.addEventListener('error', () => { on = false; reject(new Error('audio')); });
    a.play().then(meter, reject);
    setTimeout(() => { if (on) { on = false; a.pause(); resolve(); } }, (estimate(text) * 3 + 10) * 1000);
  });
}

function pickVoice() {
  const want = config.browser_voice?.name;
  const voices = speechSynthesis.getVoices();
  return voices.find((v) => v.name === want) || null;
}

function speakBrowser(text, mouth) {
  if (MUTE || !('speechSynthesis' in window) || speechSynthesis.getVoices().length === 0) {
    return mouthOnly(text, estimate(text), mouth);
  }
  return new Promise((resolve) => {
    const u = new SpeechSynthesisUtterance(text);
    const v = pickVoice();
    if (v) u.voice = v;
    u.rate = config.browser_voice?.rate || 1;
    u.pitch = config.browser_voice?.pitch ?? 1;
    let on = true;
    let bump = 0;
    let t0 = 0;
    const flap = () => {
      if (!on) return;
      const t = performance.now() / 1000;
      bump *= 0.9;
      avatar?.setLevel(0.25 + 0.35 * Math.abs(Math.sin(t * 13)) + bump);
      // The browser says nothing of its timing: the text's mouth, over the
      // line's estimated length.
      mouth(t - t0, estimate(text) / (u.rate || 1));
      requestAnimationFrame(flap);
    };
    u.onstart = () => { t0 = performance.now() / 1000; typeOut(text, estimate(text) / (u.rate || 1)); flap(); };
    u.onboundary = () => { bump = 0.3; };
    const done = () => { if (on) { on = false; resolve(); } };
    stopNow = () => { speechSynthesis.cancel(); done(); };
    u.onend = done;
    u.onerror = done;
    speechSynthesis.speak(u);
    setTimeout(done, (estimate(text) * 2 + 5) * 1000);
  });
}

// No voice at all: the mouth moves and the caption fills, for the length a
// line would take to say.
function mouthOnly(text, seconds, mouth = () => false) {
  typeOut(text, seconds);
  return new Promise((resolve) => {
    const t0 = performance.now();
    let stopped = false;
    stopNow = () => { stopped = true; };
    const flap = () => {
      const t = (performance.now() - t0) / 1000;
      if (t >= seconds || stopped) return resolve();
      avatar?.setLevel(0.2 + 0.4 * Math.abs(Math.sin(t * 12)) * (0.6 + 0.4 * Math.sin(t * 3.1)));
      mouth(t, seconds);
      requestAnimationFrame(flap);
    };
    flap();
  });
}

if ('speechSynthesis' in window) speechSynthesis.onvoiceschanged = () => {};
