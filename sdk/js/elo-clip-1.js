/* elo-clip-1.js — clips for any game, version 1. No dependencies.

   Turns what a player just did into an MP4 for TikTok, Reels or Shorts, made
   in their browser with WebCodecs: nothing is uploaded, and nothing is sent
   anywhere at all.

     <script src="https://elopros.com/sdk/elo-clip-1.js"
             integrity="sha384-…" crossorigin="anonymous"></script>

   KEEP: an instant replay of any canvas. The game's own frames are encoded
   as it runs and the last `seconds` kept, so a clip is the moment that just
   happened:

     const keep = EloClip.keep(canvas, { seconds: 30, audio: masterGainNode });
     // …the player presses your clip key:
     const video = await keep.save({ shape: "tall", line: "1v3", title: "My Game",
                                     link: "elopros.com/games/my-game" });
     await EloClip.share(video, { name: "my-game-clip.mp4" });

   A WebGL canvas is read as the browser last showed it. If your clips come
   out black, make the context with `preserveDrawingBuffer: true`, or pass
   `manual: true` and call `keep.frame()` right after you draw each frame.
   `audio` is the AudioNode everything you play goes through (a master gain),
   or a MediaStream; leave it out for a silent clip.

   FILM: for a game that can replay its own match (a deterministic sim, a
   recorded server feed), draw the replay with any camera you like, as fast
   or as slow as it draws, and hand each frame over. This is how Gates' F9
   works.

     const film = EloClip.film({ width: 1080, height: 1920, fps: 30, title: "My Game" });
     for (const t of replayFrames) { drawAt(t); await film.frame(canvas); }
     film.audio(pcm, 48000, 2);            // interleaved floats, any time before done
     const video = await film.done();

   What both make: H.264 video with AAC sound (Opus where the browser has no
   AAC encoder) in an MP4 whose index comes first, so it previews before it
   has fully loaded. `shape` is "tall" (1080×1920), "wide" (1920×1080),
   "square" (1080×1080) or "same" (the canvas, at most 1920 on a side); a
   picture of another shape is cropped to fill it, keeping its middle. Over
   it, if you ask: `line`, a few words across the top for the first three
   seconds; `tag`, a word in the corner while it plays; and an end card with
   `title`, `sub` (a line under it) and `link` (and `card: false` to leave it
   off). `font` is a CSS font family, loaded before you save.

   EloClip.supported() says whether this browser can make one at all:
   Chrome, Edge, Safari 16.4+ and Firefox 130+ can. */
"use strict";
(function () {
  if (typeof window === "undefined" || window.EloClip) return;

  var SHAPES = { tall: [1080, 1920], wide: [1920, 1080], square: [1080, 1080] };
  var AVC = ["avc1.640028", "avc1.4d0028", "avc1.640032", "avc1.42e032", "avc1.42e01f"];
  /* What the kept replay is stored as: a codec this browser both encodes and
     decodes, since a save decodes it again to crop and dress it. */
  var KEEP = ["vp8", "vp09.00.10.08", "avc1.42e01f", "avc1.640028"];
  var MAX_SIDE = 1920;
  var CARD_S = 2.4;

  function fail(code, message) {
    var e = new Error(message);
    e.code = code;
    return e;
  }
  var sleep = function (ms) { return new Promise(function (r) { setTimeout(r, ms); }); };
  var even = function (v) { return Math.max(2, Math.round(v / 2) * 2); };
  function bytes(d) {
    if (!d) return null;
    if (d instanceof ArrayBuffer) return new Uint8Array(d.slice(0));
    return new Uint8Array(d.buffer.slice(d.byteOffset, d.byteOffset + d.byteLength));
  }
  function canvas2d(w, h) {
    var c = typeof OffscreenCanvas !== "undefined" ? new OffscreenCanvas(w, h)
      : Object.assign(document.createElement("canvas"), { width: w, height: h });
    return { c: c, g: c.getContext("2d", { alpha: false }) };
  }

  /* ── the MP4 ──────────────────────────────────────────────────────────── */
  var te = new TextEncoder();
  function cat(parts) {
    var n = 0, i, at = 0;
    for (i = 0; i < parts.length; i++) n += parts[i].byteLength;
    var out = new Uint8Array(n);
    for (i = 0; i < parts.length; i++) { out.set(parts[i], at); at += parts[i].byteLength; }
    return out;
  }
  function box(type) {
    var body = cat(Array.prototype.slice.call(arguments, 1));
    var head = new Uint8Array(8);
    new DataView(head.buffer).setUint32(0, 8 + body.byteLength);
    head.set(te.encode(type), 4);
    return cat([head, body]);
  }
  function full(type, version, flags) {
    var rest = Array.prototype.slice.call(arguments, 3);
    return box.apply(null, [type, u8(version, flags >> 16, (flags >> 8) & 255, flags & 255)].concat(rest));
  }
  function u8() { return new Uint8Array(arguments); }
  function nums(bits, signed, v) {
    var b = new Uint8Array(v.length * bits / 8), d = new DataView(b.buffer);
    for (var i = 0; i < v.length; i++) {
      if (bits === 16) d.setUint16(i * 2, v[i]);
      else if (signed) d.setInt32(i * 4, v[i]);
      else d.setUint32(i * 4, v[i] >>> 0);
    }
    return b;
  }
  function u16() { return nums(16, false, arguments); }
  function u32() { return nums(32, false, arguments); }
  var MATRIX = u32(0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000);
  function desc(tag, body) {
    var n = body.byteLength;
    return cat([u8(tag, 0x80 | ((n >> 21) & 127), 0x80 | ((n >> 14) & 127), 0x80 | ((n >> 7) & 127), n & 127), body]);
  }
  function esds(asc, bitrate) {
    var dcd = desc(4, cat([u8(0x40, 0x15, 0, 0, 0), u32(bitrate, bitrate), desc(5, asc)]));
    return full("esds", 0, 0, desc(3, cat([u16(2), u8(0), dcd, desc(6, u8(2))])));
  }
  function dOps(head, ch, rate) {
    var skip = 312, gain = 0, family = 0;
    if (head && head.byteLength >= 19) {
      var d = new DataView(head.buffer, head.byteOffset, head.byteLength);
      ch = d.getUint8(9); skip = d.getUint16(10, true); rate = d.getUint32(12, true);
      gain = d.getInt16(16, true); family = d.getUint8(18);
    }
    var g = new Uint8Array(2);
    new DataView(g.buffer).setInt16(0, gain);
    return box("dOps", u8(0, ch), u16(skip), u32(rate), g, u8(family));
  }
  function runs(deltas) {
    var out = [];
    deltas.forEach(function (d) {
      var last = out[out.length - 1];
      if (last && last[1] === d) last[0]++; else out.push([1, d]);
    });
    return out;
  }
  function table(samples, timescale, step) {
    var t = { timescale: timescale, duration: samples.length * step, ctts: null, sync: [] };
    t.sizes = samples.map(function (s) { return s.data.byteLength; });
    samples.forEach(function (s, i) { if (s.key) t.sync.push(i + 1); });
    var at = function (us) { return Math.round((us * timescale) / 1e6); };
    var base = samples.length ? at(samples[0].pts) : 0;
    if (samples.some(function (s, i) { return at(s.pts) - base !== i * step; })) {
      t.ctts = runs(samples.map(function (s, i) { return at(s.pts) - base - i * step; }));
    }
    t.stts = runs(samples.map(function () { return step; }));
    return t;
  }
  function stbl(t) {
    var n = t.sizes.length;
    var parts = [full("stsd", 0, 0, u32(1), t.entry),
      full.apply(null, ["stts", 0, 0, u32(t.stts.length)].concat(t.stts.map(function (r) { return u32(r[0], r[1]); })))];
    if (t.ctts) {
      parts.push(full.apply(null, ["ctts", 1, 0, u32(t.ctts.length)].concat(t.ctts.map(function (r) {
        return cat([u32(r[0]), nums(32, true, [r[1]])]);
      }))));
    }
    if (t.sync && t.sync.length < n) parts.push(full("stss", 0, 0, u32(t.sync.length), u32.apply(null, t.sync)));
    parts.push(full("stsc", 0, 0, u32(1), u32(1, n, 1)),
      full("stsz", 0, 0, u32(0, n), u32.apply(null, t.sizes)),
      full("stco", 0, 0, u32(1), u32(t.offset)));
    return box.apply(null, ["stbl"].concat(parts));
  }
  function trak(id, video, t, ms, w, h) {
    var tkhd = full("tkhd", 0, 3, u32(0, 0, id, 0, ms), u32(0, 0), u16(0, 0, video ? 0 : 0x0100, 0), MATRIX,
      u32(video ? w * 65536 : 0, video ? h * 65536 : 0));
    var mdhd = full("mdhd", 0, 0, u32(0, 0, t.timescale, t.duration), u16(0x55c4, 0));
    var hdlr = full("hdlr", 0, 0, u32(0), te.encode(video ? "vide" : "soun"), u32(0, 0, 0),
      te.encode(video ? "VideoHandler\0" : "SoundHandler\0"));
    var head = video ? full("vmhd", 0, 1, u16(0, 0, 0, 0)) : full("smhd", 0, 0, u16(0, 0));
    var dinf = box("dinf", full("dref", 0, 0, u32(1), full("url ", 0, 1)));
    return box("trak", tkhd, box("mdia", mdhd, hdlr, box("minf", head, dinf, stbl(t))));
  }
  /* video: {samples [{data, pts, key}], width, height, fps, avcC};
     audio: {samples, codec "aac"|"opus", config, rate, channels, frame, bitrate} | null */
  function mux(video, audio) {
    var vt = table(video.samples, video.fps * 1000, 1000);
    vt.entry = box("avc1", u8(0, 0, 0, 0, 0, 0), u16(1), u16(0, 0), u32(0, 0, 0),
      u16(video.width, video.height), u32(0x480000, 0x480000), u32(0), u16(1),
      new Uint8Array(32), u16(0x18), u16(0xffff), box("avcC", video.avcC));
    var ms = Math.round((vt.duration / vt.timescale) * 1000);
    var at = null;
    if (audio && audio.samples.length) {
      at = table(audio.samples, audio.rate, audio.frame);
      at.ctts = null;
      at.sync = null;
      var entry = function (type, extra) {
        return box(type, u8(0, 0, 0, 0, 0, 0), u16(1), u32(0, 0), u16(audio.channels, 16), u16(0, 0),
          u32((audio.rate * 65536) >>> 0), extra);
      };
      at.entry = audio.codec === "opus" ? entry("Opus", dOps(audio.config, audio.channels, audio.rate))
        : entry("mp4a", esds(audio.config, audio.bitrate));
    }
    var ftyp = box("ftyp", te.encode("isom"), u32(512), te.encode("isomiso2avc1mp41"));
    var sum = function (s) { return s.reduce(function (n, x) { return n + x.data.byteLength; }, 0); };
    var vBytes = sum(video.samples), aBytes = at ? sum(audio.samples) : 0;
    var moov = function () {
      var mvhd = full("mvhd", 0, 0, u32(0, 0, 1000, ms, 0x10000), u16(0x0100, 0), u32(0, 0), MATRIX,
        u32(0, 0, 0, 0, 0, 0), u32(at ? 3 : 2));
      var traks = [trak(1, true, vt, ms, video.width, video.height)];
      if (at) traks.push(trak(2, false, at, Math.round((at.duration / at.timescale) * 1000)));
      return box.apply(null, ["moov", mvhd].concat(traks));
    };
    vt.offset = 0;
    if (at) at.offset = 0;
    var head = ftyp.byteLength + moov().byteLength + 8;
    vt.offset = head;
    if (at) at.offset = head + vBytes;
    var parts = [ftyp, moov(), u32(8 + vBytes + aBytes), te.encode("mdat")];
    video.samples.forEach(function (s) { parts.push(s.data); });
    if (at) audio.samples.forEach(function (s) { parts.push(s.data); });
    return new Blob(parts, { type: "video/mp4" });
  }

  /* ── what goes over the picture ───────────────────────────────────────── */
  function text(g, s, x, y, px, weight, align, alpha, font) {
    g.save();
    g.globalAlpha = alpha;
    g.font = weight + " " + px + "px " + font;
    g.textAlign = align;
    g.textBaseline = "middle";
    g.lineWidth = Math.max(2, px * 0.12);
    g.lineJoin = "round";
    g.strokeStyle = "rgba(0,0,0,.55)";
    g.strokeText(s, x, y);
    g.fillStyle = "#f2ede2";
    g.fillText(s, x, y);
    g.restore();
  }
  /* Fill the frame with the middle of the picture. */
  function cover(g, src, sw, sh, w, h) {
    var s = Math.max(w / sw, h / sh), dw = sw * s, dh = sh * s;
    g.drawImage(src, (w - dw) / 2, (h - dh) / 2, dw, dh);
  }

  /* ── the encoders ─────────────────────────────────────────────────────── */
  function supported() {
    return {
      video: typeof VideoEncoder !== "undefined" && typeof VideoFrame !== "undefined",
      audio: typeof AudioEncoder !== "undefined" && typeof AudioData !== "undefined",
      keep: typeof VideoEncoder !== "undefined" && typeof VideoDecoder !== "undefined",
    };
  }
  async function videoConfig(w, h, fps) {
    if (!supported().video) throw fail("unsupported", "this browser cannot encode video (no WebCodecs); Chrome, Edge, Safari 16.4+ and Firefox 130+ can");
    for (var i = 0; i < AVC.length; i++) {
      var c = { codec: AVC[i], width: w, height: h, framerate: fps, bitrate: w * h > 1.5e6 ? 10e6 : 6e6, avc: { format: "avc" } };
      try {
        var r = await VideoEncoder.isConfigSupported(c);
        if (r.supported) return r.config;
      } catch (err) { /* the next one */ }
    }
    throw fail("unsupported", "this browser cannot encode H.264 at " + w + "×" + h);
  }
  async function audioConfig(rate, ch) {
    if (!supported().audio) return null;
    var tries = [{ codec: "mp4a.40.2", sampleRate: rate, numberOfChannels: ch, bitrate: 160000 },
      { codec: "opus", sampleRate: 48000, numberOfChannels: ch, bitrate: 128000 }];
    for (var i = 0; i < tries.length; i++) {
      try {
        var r = await AudioEncoder.isConfigSupported(tries[i]);
        if (r.supported) return r.config;
      } catch (err) { /* the next one */ }
    }
    return null;
  }
  /* Linear resample of interleaved floats; Opus only takes 48 kHz. */
  function resample(pcm, from, to, ch) {
    if (from === to) return pcm;
    var n = Math.floor((pcm.length / ch) * to / from), out = new Float32Array(n * ch);
    for (var i = 0; i < n; i++) {
      var x = i * from / to, k = Math.floor(x), f = x - k;
      for (var c = 0; c < ch; c++) {
        var a = pcm[k * ch + c] || 0, b = pcm[(k + 1) * ch + c];
        out[i * ch + c] = b === undefined ? a : a + (b - a) * f;
      }
    }
    return out;
  }
  /* The sound, run out to the picture's length and faded under the card. */
  async function encodeAudio(pcm, rate, ch, seconds, fade) {
    var config = await audioConfig(rate, ch);
    if (!config || !pcm || !pcm.length) return null;
    var opus = config.codec === "opus";
    if (opus) { pcm = resample(pcm, rate, 48000, ch); rate = 48000; }
    var total = Math.round(seconds * rate) * ch, all = new Float32Array(total);
    all.set(pcm.subarray(0, Math.min(pcm.length, total)));
    var end = Math.min(pcm.length, total), from = Math.max(0, end - Math.round(rate * fade) * ch);
    for (var i = from; i < total; i++) all[i] *= Math.max(0, 1 - (i - from) / Math.max(1, end - from));
    var out = [], description = null, err = null;
    var enc = new AudioEncoder({
      output: function (chunk, meta) {
        if (meta && meta.decoderConfig && meta.decoderConfig.description) description = bytes(meta.decoderConfig.description);
        var data = new Uint8Array(chunk.byteLength);
        chunk.copyTo(data);
        out.push({ data: data, pts: chunk.timestamp, key: true });
      },
      error: function (e) { err = e; },
    });
    enc.configure(config);
    var block = 4800;
    for (var f = 0; f * ch < total; f += block) {
      var n = Math.min(block, total / ch - f);
      enc.encode(new AudioData({ format: "f32", sampleRate: rate, numberOfChannels: ch, numberOfFrames: n,
        timestamp: Math.round((f * 1e6) / rate), data: all.subarray(f * ch, (f + n) * ch) }));
    }
    await enc.flush();
    enc.close();
    if (err) throw err;
    /* AAC-LC at the rate, stereo or mono, when the encoder named no config. */
    var idx = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000].indexOf(rate);
    return { samples: out, codec: opus ? "opus" : "aac", rate: rate, channels: ch, frame: opus ? 960 : 1024,
      bitrate: config.bitrate || 128000,
      config: description || (opus ? null : u8(0x10 | ((idx < 0 ? 3 : idx) >> 1), (((idx < 0 ? 3 : idx) & 1) << 7) | (ch << 3))) };
  }

  /* The film's back half, shared by both modes: frames are drawn onto a
     canvas the video's size, dressed, encoded in order, and muxed with the
     sound at the end. */
  function maker(o) {
    var w = o.width, h = o.height, fps = o.fps;
    var font = o.font || '"Roboto Condensed", "Arial Narrow", system-ui, sans-serif';
    var page = canvas2d(w, h), last = canvas2d(w, h);
    var samples = [], avcC = null, err = null, frames = 0, enc = null;
    var ready = videoConfig(w, h, fps).then(function (config) {
      enc = new VideoEncoder({
        output: function (chunk, meta) {
          if (meta && meta.decoderConfig && meta.decoderConfig.description) avcC = bytes(meta.decoderConfig.description);
          var data = new Uint8Array(chunk.byteLength);
          chunk.copyTo(data);
          samples.push({ data: data, pts: chunk.timestamp, key: chunk.type === "key" });
        },
        error: function (e) { err = e; },
      });
      enc.configure(config);
    });
    ready.catch(function () { /* thrown again where it is awaited */ });
    async function put(draw, bare) {
      await ready;
      while (enc.encodeQueueSize > 6) await sleep(4);
      if (err) throw err;
      var t = frames / fps, g = page.g;
      draw(g);
      if (!bare) last.g.drawImage(page.c, 0, 0);
      if (o.tag && !bare) {
        var px = Math.round(Math.min(w, h) * 0.032);
        text(g, String(o.tag), px * 1.1, h - px * 2.2, px, 700, "left", 0.72, font);
      }
      if (o.line && t < 3.2 && !bare) {
        text(g, String(o.line), w / 2, h * (h > w ? 0.17 : 0.14), Math.round(Math.min(w, h) * 0.075), 700, "center",
          Math.min(1, t / 0.25, (3.2 - t) / 0.3), font);
      }
      var vf = new VideoFrame(page.c, { timestamp: Math.round((frames * 1e6) / fps), duration: Math.round(1e6 / fps) });
      enc.encode(vf, { keyFrame: frames % (fps * 2) === 0 });
      vf.close();
      frames++;
    }
    return {
      /* a picture of any size, cropped to fill */
      frame: function (src, sw, sh) {
        return put(function (g) { cover(g, src, sw, sh, w, h); });
      },
      done: async function (pcm, rate, ch) {
        await ready;
        if (o.card !== false && (o.title || o.link)) {
          var n = Math.round(fps * CARD_S), big = Math.round(Math.min(w, h) * 0.2);
          for (var k = 0; k < n; k++) {
            var u = k / n, a = Math.min(1, u * 4);
            await put(function (g) {
              g.drawImage(last.c, 0, 0);
              g.fillStyle = "rgba(14,12,10," + (0.35 + 0.45 * Math.min(1, u * 3)) + ")";
              g.fillRect(0, 0, w, h);
              var y = h * 0.44;
              if (o.title) text(g, String(o.title), w / 2, y, big, 700, "center", a, font);
              if (o.sub) { y += big * 0.85; text(g, String(o.sub), w / 2, y, Math.round(big * 0.28), 400, "center", a, font); }
              else y += big * 0.45;
              if (o.link) text(g, String(o.link), w / 2, y + big * 0.5, Math.round(big * 0.3), 700, "center", a, font);
            }, true);
          }
        }
        await enc.flush();
        enc.close();
        if (err) throw err;
        if (!avcC) throw fail("encoder", "the video encoder named no configuration");
        var seconds = frames / fps;
        var audio = await encodeAudio(pcm, rate || 48000, ch || 2, seconds, o.card !== false ? 0.8 : 0.2);
        return mux({ samples: samples, width: w, height: h, fps: fps, avcC: avcC }, audio);
      },
    };
  }

  function size(o, sw, sh) {
    var want = SHAPES[o.shape || "tall"];
    if (want) return want;
    var s = Math.min(1, MAX_SIDE / Math.max(sw, sh));
    return [even(sw * s), even(sh * s)];
  }

  /* ── FILM: frames the game hands over ─────────────────────────────────── */
  function film(o) {
    o = Object.assign({ fps: 30 }, o || {});
    var wh = o.width && o.height ? [even(o.width), even(o.height)] : SHAPES[o.shape || "tall"];
    var m = maker({ width: wh[0], height: wh[1], fps: o.fps, tag: o.tag, line: o.line, title: o.title,
      sub: o.sub, link: o.link, card: o.card, font: o.font });
    var pcm = [], pcmLen = 0, rate = 48000, ch = 2, scratch = null;
    return {
      width: wh[0], height: wh[1],
      /* a canvas, image, VideoFrame or ImageData; resolves when it may take the next */
      frame: function (src) {
        if (typeof ImageData !== "undefined" && src instanceof ImageData) {
          if (!scratch || scratch.c.width !== src.width || scratch.c.height !== src.height) scratch = canvas2d(src.width, src.height);
          scratch.g.putImageData(src, 0, 0);
          src = scratch.c;
        }
        var sw = src.displayWidth || src.videoWidth || src.width, sh = src.displayHeight || src.videoHeight || src.height;
        return m.frame(src, sw, sh);
      },
      /* interleaved floats at `rate` with `channels`, appended in order */
      audio: function (samples, r, c) {
        rate = r || rate; ch = c || ch;
        pcm.push(new Float32Array(samples));
        pcmLen += samples.length;
      },
      done: async function () {
        var all = new Float32Array(pcmLen), at = 0;
        pcm.forEach(function (p) { all.set(p, at); at += p.length; });
        return m.done(all, rate, ch);
      },
    };
  }

  /* ── KEEP: an instant replay of a live canvas ─────────────────────────── */
  async function keepCodec(w, h, fps) {
    for (var i = 0; i < KEEP.length; i++) {
      var c = { codec: KEEP[i], width: w, height: h, framerate: fps, bitrate: Math.round(w * h * fps * 0.12),
        latencyMode: "realtime" };
      if (KEEP[i].indexOf("avc1") === 0) c.avc = { format: "annexb" };
      try {
        var e = await VideoEncoder.isConfigSupported(c);
        var d = await VideoDecoder.isConfigSupported({ codec: KEEP[i], codedWidth: w, codedHeight: h });
        if (e.supported && d.supported) return e.config;
      } catch (err) { /* the next one */ }
    }
    throw fail("unsupported", "this browser cannot keep a replay at " + w + "×" + h);
  }

  function keep(canvas, o) {
    if (!canvas || !(canvas.width > 0)) throw fail("canvas", "keep needs the canvas your game draws into");
    if (!supported().keep) throw fail("unsupported", "this browser cannot keep a replay (no WebCodecs)");
    o = Object.assign({ seconds: 30, fps: 30 }, o || {});
    var seconds = Math.max(3, Math.min(60, o.seconds)), fps = Math.max(10, Math.min(60, o.fps));
    var run = null;            /* {w, h, enc, config, chunks: [{chunk, data, ts, key}]} */
    var t0 = performance.now(), lastAt = -1e9, lastKey = -1e9, stopped = false, raf = 0;
    var shrink = null;

    function restart(w, h) {
      var mine = { w: w, h: h, enc: null, config: null, chunks: [], err: null };
      run = mine;
      keepCodec(w, h, fps).then(function (config) {
        if (run !== mine) return;
        mine.config = config;
        mine.enc = new VideoEncoder({
          output: function (chunk, meta) {
            if (meta && meta.decoderConfig) mine.config = Object.assign({}, mine.config, { description: meta.decoderConfig.description });
            var data = new Uint8Array(chunk.byteLength);
            chunk.copyTo(data);
            mine.chunks.push({ data: data, ts: chunk.timestamp, key: chunk.type === "key", dur: chunk.duration });
            trim(mine);
          },
          error: function (e) { mine.err = e; },
        });
        mine.enc.configure(config);
        lastKey = -1e9;
      }, function (e) { mine.err = e; });
    }
    /* Keep from the newest keyframe that still reaches back `seconds`. */
    function trim(r) {
      var newest = r.chunks.length ? r.chunks[r.chunks.length - 1].ts : 0, cut = newest - seconds * 1e6, from = 0;
      for (var i = 0; i < r.chunks.length; i++) {
        if (r.chunks[i].ts > cut) break;
        if (r.chunks[i].key) from = i;
      }
      if (from > 0) r.chunks.splice(0, from);
    }
    function frame() {
      if (stopped) return;
      var now = performance.now();
      if (now - lastAt < 1000 / fps - 2) return;
      var cw = canvas.width, ch = canvas.height, s = Math.min(1, MAX_SIDE / Math.max(cw, ch));
      var w = even(cw * s), h = even(ch * s);
      if (!run || run.w !== w || run.h !== h) restart(w, h);
      if (!run.enc || run.enc.state !== "configured" || run.enc.encodeQueueSize > 4) return;
      lastAt = now;
      var src = canvas;
      if (s < 1) {
        if (!shrink || shrink.c.width !== w || shrink.c.height !== h) shrink = canvas2d(w, h);
        shrink.g.drawImage(canvas, 0, 0, w, h);
        src = shrink.c;
      }
      var ts = Math.round((now - t0) * 1000), vf;
      try { vf = new VideoFrame(src, { timestamp: ts }); } catch (e) { return; }
      var key = ts - lastKey >= 1e6;
      if (key) lastKey = ts;
      run.enc.encode(vf, { keyFrame: key });
      vf.close();
    }
    function tick() {
      frame();
      if (!stopped && !o.manual) raf = requestAnimationFrame(tick);
    }
    if (!o.manual) raf = requestAnimationFrame(tick);

    /* The sound: the last `seconds` (and a little) of what goes through the
       node, kept as it plays. */
    var ac = null, tap = null, ring = null, ringAt = 0, ringFull = false, rate = 48000, sink = null;
    if (o.audio) {
      var node = o.audio;
      if (typeof MediaStream !== "undefined" && node instanceof MediaStream) {
        ac = o.context || new AudioContext();
        node = ac.createMediaStreamSource(node);
      } else {
        ac = node.context;
      }
      rate = ac.sampleRate;
      ring = new Float32Array(Math.ceil((seconds + 2) * rate) * 2);
      tap = ac.createScriptProcessor(4096, 2, 2);
      tap.onaudioprocess = function (ev) {
        var b = ev.inputBuffer, l = b.getChannelData(0), r = b.numberOfChannels > 1 ? b.getChannelData(1) : l;
        for (var i = 0; i < l.length; i++) {
          ring[ringAt++] = l[i];
          ring[ringAt++] = r[i];
          if (ringAt >= ring.length) { ringAt = 0; ringFull = true; }
        }
      };
      sink = ac.createGain();
      sink.gain.value = 0;
      node.connect(tap);
      tap.connect(sink);
      sink.connect(ac.destination);
    }
    function lastPcm(sec) {
      if (!ring) return null;
      var have = ringFull ? ring.length : ringAt, want = Math.min(have, Math.round(sec * rate) * 2);
      var out = new Float32Array(want), start = (ringAt - want + ring.length) % ring.length;
      for (var i = 0; i < want; i++) out[i] = ring[(start + i) % ring.length];
      return out;
    }

    return {
      /* Encode the canvas now (with `manual: true`, after each draw). */
      frame: frame,
      /* Seconds a save can reach back over now. */
      held: function () {
        var c = run && run.chunks;
        return c && c.length ? (c[c.length - 1].ts - c[0].ts) / 1e6 : 0;
      },
      stop: function () {
        stopped = true;
        cancelAnimationFrame(raf);
        if (run && run.enc && run.enc.state !== "closed") run.enc.close();
        if (tap) { tap.disconnect(); sink.disconnect(); }
        run = null;
      },
      /* The last `seconds` (all that is kept, by default) as an MP4 Blob. */
      save: async function (so) {
        so = Object.assign({}, o, so || {});
        var r = run;
        if (!r || !r.enc) throw fail("empty", "nothing kept yet");
        if (r.err) throw r.err;
        await r.enc.flush();
        var chunks = r.chunks.slice();
        if (!chunks.length) throw fail("empty", "nothing kept yet");
        var want = Math.min(seconds, so.seconds || seconds), end = chunks[chunks.length - 1].ts, from = 0;
        for (var i = 0; i < chunks.length; i++) {
          if (chunks[i].ts > end - want * 1e6) break;
          if (chunks[i].key) from = i;
        }
        chunks = chunks.slice(from);
        var span = (end - chunks[0].ts) / 1e6 + 1 / fps;
        var wh = size(so, r.w, r.h);
        var m = maker({ width: wh[0], height: wh[1], fps: fps, tag: so.tag, line: so.line, title: so.title,
          sub: so.sub, link: so.link, card: so.card, font: so.font });
        /* Decoded in order and drawn on the video's own clock: each 1/fps,
           the newest kept picture at that instant. The decoder is fed a few
           chunks ahead and flushed once, at the end: a flush makes it want a
           keyframe next. */
        var decoded = [], derr = null, flushed = false;
        var dec = new VideoDecoder({ output: function (f) { decoded.push(f); }, error: function (e) { derr = e; } });
        var dconf = { codec: r.config.codec, codedWidth: r.w, codedHeight: r.h };
        if (r.config.description) dconf.description = r.config.description;
        dec.configure(dconf);
        var start = chunks[0].ts, shown = null, n = Math.max(1, Math.round(span * fps)), next = 0, k = 0;
        try {
          while (k < n) {
            while (next < chunks.length && dec.decodeQueueSize < 4 && decoded.length < 8) {
              var c = chunks[next++];
              dec.decode(new EncodedVideoChunk({ type: c.key ? "key" : "delta", timestamp: c.ts, data: c.data }));
            }
            if (next >= chunks.length && !flushed) {
              await dec.flush();
              flushed = true;
            }
            if (derr) throw derr;
            var at = start + (k * 1e6) / fps;
            while (decoded.length && decoded[0].timestamp <= at) {
              if (shown) shown.close();
              shown = decoded.shift();
            }
            /* Only once a later picture is in hand (or none can come) is the
               one shown the right one for this instant. */
            if (decoded.length || flushed) {
              if (shown) await m.frame(shown, shown.displayWidth, shown.displayHeight);
              k++;
            } else {
              await sleep(2);
            }
          }
        } finally {
          if (shown) shown.close();
          decoded.forEach(function (f) { f.close(); });
          if (dec.state !== "closed") dec.close();
        }
        return m.done(lastPcm(span), rate, 2);
      },
    };
  }

  /* Hand a clip to an app (a phone's share sheet) where the browser can, or
     save it. Call it from a click. */
  async function share(blob, o) {
    o = o || {};
    var name = o.name || "clip.mp4";
    var file = typeof File !== "undefined" ? new File([blob], name, { type: "video/mp4" }) : null;
    if (file && navigator.canShare && navigator.canShare({ files: [file] })) {
      try {
        await navigator.share({ files: [file], title: o.title || "" });
        return "shared";
      } catch (e) {
        if (e && e.name === "AbortError") return "cancelled";
      }
    }
    var a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(function () { URL.revokeObjectURL(a.href); }, 60000);
    return "saved";
  }

  window.EloClip = { version: 1, supported: supported, keep: keep, film: film, share: share, mux: mux };
})();
