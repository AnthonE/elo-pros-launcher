// The body: a VRM (three.js + three-vrm), a rigged glTF that is no VRM, or a
// set of pictures. All answer the same calls, so the stage never asks which one it holds:
//   setExpression(name)   setLook(at)   setLevel(0..1)   faces()   dispose()
//   setMouth(shape, open) the line's mouth: aa ih ou ee oh, mm (closed) or
//                         '' (at rest); null hands the mouth back to the level
// and both keep themselves alive between lines: breathing, blinking, glancing.

import { fileUrl } from './link.js';

const now = () => performance.now() / 1000;
const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
const ease = (cur, want, rate, dt) => cur + (want - cur) * (1 - Math.exp(-rate * dt));

// Where the eyes go, as a small offset from looking straight out.
const LOOK = {
  viewer: [0, 0], left: [-1, 0], right: [1, 0], up: [0, 1], down: [0, -1], around: null,
};

class Glance {
  constructor() { this.at = 'viewer'; this.x = 0; this.y = 0; this.next = 0; this.tx = 0; this.ty = 0; }
  step(t, dt) {
    let want = LOOK[this.at];
    if (!want) {
      // Wander: a new point every few seconds, mostly near the viewer.
      if (t > this.next) {
        this.tx = (Math.random() - 0.5) * 1.6;
        this.ty = (Math.random() - 0.5) * 0.8;
        this.next = t + 1.5 + Math.random() * 3;
      }
      want = [this.tx, this.ty];
    } else if (this.at === 'viewer' && t > this.next) {
      // Even looking at the viewer, an eye drifts now and then.
      this.tx = (Math.random() - 0.5) * 0.25;
      this.ty = (Math.random() - 0.5) * 0.15;
      this.next = t + 2 + Math.random() * 4;
    }
    const [wx, wy] = this.at === 'viewer' ? [this.tx, this.ty] : want;
    this.x = ease(this.x, wx, 6, dt);
    this.y = ease(this.y, wy, 6, dt);
  }
}

// ── pictures ────────────────────────────────────────────────────────────────

export class PictureAvatar {
  constructor(host, model) {
    this.host = host;
    this.model = model;
    this.el = document.createElement('div');
    this.el.className = 'pics';
    this.imgs = {};
    for (const [face, file] of Object.entries(model.files || {})) {
      const img = new Image();
      img.decoding = 'async';
      img.src = fileUrl('/model/' + file);
      img.alt = '';
      img.dataset.face = face;
      this.el.appendChild(img);
      this.imgs[face] = img;
    }
    host.appendChild(this.el);
    this.standing = model.default;
    this.face = model.default;
    this.level = 0;
    this.lv = 0;
    this.talkHold = 0;
    this.mouth = null;
    this.glance = new Glance();
    this.t0 = now();
    this.ready = Promise.all(Object.values(this.imgs).map((i) => i.decode().catch(() => {})));
    this.show(this.face, false);
    this.raf = requestAnimationFrame(() => this.tick(now()));
  }
  faces() { return Object.keys(this.imgs).filter((f) => !f.endsWith('-talk')); }
  setExpression(name) { if (this.imgs[name]) { this.face = name; } }
  setLook(at) { this.glance.at = at in LOOK ? at : 'viewer'; }
  setLevel(x) { this.level = clamp(x, 0, 1); }
  setMouth(shape, open = 0) { this.mouth = shape == null ? null : { shape, open }; }
  show(face, talking) {
    const want = talking && this.imgs[face + '-talk'] ? face + '-talk' : face;
    for (const [k, img] of Object.entries(this.imgs)) img.classList.toggle('on', k === want);
  }
  tick(last) {
    const t = now();
    const dt = Math.min(0.1, t - last);
    this.lv = ease(this.lv, this.level, 18, dt);
    this.glance.step(t, dt);
    // A talking frame holds a little, so the mouth doesn't flicker per sample.
    // With a line's mouth, it opens on the open shapes and shuts on m, b, p.
    const open = this.mouth ? this.mouth.shape && this.mouth.shape !== 'mm' && this.mouth.open > 0.25 : this.lv > 0.14;
    if (open) this.talkHold = t + (this.mouth ? 0.05 : 0.09);
    this.show(this.face, t < this.talkHold);
    const breathe = Math.sin((t - this.t0) * 1.6) * 0.006;
    const bounce = this.lv * 0.045;
    const sway = Math.sin((t - this.t0) * 0.7) * 0.6;
    const x = this.glance.x * 1.6;
    const y = -this.glance.y * 1.0 - bounce * 100;
    const rot = sway + this.glance.x * 2.2;
    this.el.style.transform =
      `translate(${x}%, ${y}%) rotate(${rot}deg) scale(${1 + breathe + bounce * 0.4}, ${1 + breathe - bounce * 0.15})`;
    this.raf = requestAnimationFrame(() => this.tick(t));
  }
  dispose() { cancelAnimationFrame(this.raf); this.el.remove(); }
}

// ── a VRM ───────────────────────────────────────────────────────────────────

const VISEMES = ['aa', 'ih', 'ou', 'ee', 'oh'];
const NOT_FACES = new Set([...VISEMES, 'blink', 'blinkLeft', 'blinkRight', 'lookUp', 'lookDown',
  'lookLeft', 'lookRight']);

export class VrmAvatar {
  static async load(host, V, gltf) {
    const a = new VrmAvatar(host, V);
    await a.open(gltf);
    return a;
  }
  constructor(host, V) {
    this.V = V;
    this.host = host;
    this.renderer = new V.WebGLRenderer({ alpha: true, antialias: true, premultipliedAlpha: true });
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.outputColorSpace = V.SRGBColorSpace;
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
    this.renderer.domElement.className = 'vrm';
    host.appendChild(this.renderer.domElement);
    this.scene = new V.Scene();
    this.camera = new V.PerspectiveCamera(24, 1, 0.1, 20);
    const sun = new V.DirectionalLight(0xffffff, Math.PI * 0.9);
    sun.position.set(0.6, 1.2, 1.4);
    this.scene.add(sun, new V.HemisphereLight(0xffffff, 0x445566, 1.2));
    this.target = new V.Object3D();
    this.scene.add(this.target);
    this.level = 0;
    this.lv = 0;
    this.face = 'neutral';
    this.weights = {};
    this.glance = new Glance();
    this.blinkAt = now() + 2;
    this.viseme = 'aa';
    this.visemeAt = 0;
    this.mouth = null;
    this.resize = new ResizeObserver(() => this.fit());
    this.resize.observe(host);
  }
  async open(gltf) {
    const V = this.V;
    const vrm = gltf.userData.vrm;
    V.VRMUtils.removeUnnecessaryVertices(gltf.scene);
    if (V.VRMUtils.combineSkeletons) V.VRMUtils.combineSkeletons(gltf.scene);
    V.VRMUtils.rotateVRM0(vrm);
    vrm.scene.traverse((o) => { o.frustumCulled = false; });
    this.scene.add(vrm.scene);
    this.vrm = vrm;
    if (vrm.lookAt) vrm.lookAt.target = this.target;
    // Arms down from the T-pose they are modelled in.
    const bone = (n) => vrm.humanoid?.getNormalizedBoneNode(n);
    this.bones = {
      lu: bone('leftUpperArm'), ru: bone('rightUpperArm'), ll: bone('leftLowerArm'),
      rl: bone('rightLowerArm'), spine: bone('spine'), chest: bone('chest') || bone('upperChest'),
      neck: bone('neck'), head: bone('head'), hips: bone('hips'),
    };
    vrm.update(0);
    const head = new V.Vector3();
    (this.bones.head || vrm.scene).getWorldPosition(head);
    this.headY = head.y;
    this.fit();
    this.t0 = now();
    this.raf = requestAnimationFrame(() => this.tick(now()));
  }
  fit() {
    const w = this.host.clientWidth || 1;
    const h = this.host.clientHeight || 1;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    // A bust shot: above the head to mid-chest, about 0.8 m tall (the head
    // bone is the base of the skull, so the hair needs room above it),
    // with the head in the upper third. Wider frames back off so the
    // shoulders still fit.
    const y = this.headY || 1.4;
    const span = 0.8 / Math.min(1, this.camera.aspect / 0.62);
    const d = span / (2 * Math.tan((this.camera.fov / 2) * Math.PI / 180));
    this.camera.position.set(0, y, d);
    this.camera.lookAt(0, y - 0.08, 0);
    this.camera.updateProjectionMatrix();
  }
  faces() {
    const list = this.vrm?.expressionManager?.expressions?.map((e) => e.expressionName) || [];
    return list.filter((n) => !NOT_FACES.has(n));
  }
  setExpression(name) { this.face = name; }
  setLook(at) { this.glance.at = at in LOOK ? at : 'viewer'; }
  setLevel(x) { this.level = clamp(x, 0, 1); }
  setMouth(shape, open = 0) { this.mouth = shape == null ? null : { shape, open }; }
  tick(last) {
    const t = now();
    const dt = Math.min(0.1, t - last);
    const vrm = this.vrm;
    const em = vrm.expressionManager;
    this.lv = ease(this.lv, this.level, 20, dt);
    this.glance.step(t, dt);

    // The face: ease every face toward 0 except the one asked for.
    if (em) {
      for (const n of this.faces()) {
        const want = n === this.face && n !== 'neutral' ? 1 : 0;
        this.weights[n] = ease(this.weights[n] || 0, want, 8, dt);
        em.setValue(n, this.weights[n]);
      }
      // The mouth: the line's own shapes when it carries them; else a
      // viseme per syllable-ish, its size the voice's level.
      const m = this.mouth;
      if (!m && t > this.visemeAt) {
        this.viseme = VISEMES[(Math.random() * VISEMES.length) | 0];
        this.visemeAt = t + 0.08 + Math.random() * 0.1;
      }
      for (const v of VISEMES) {
        const want = m ? (v === m.shape ? m.open : 0)
          : v === this.viseme ? this.lv * (v === 'aa' ? 1 : 0.8) : this.lv * 0.15;
        this.weights[v] = ease(this.weights[v] || 0, want, m ? 28 : 30, dt);
        em.setValue(v, this.weights[v]);
      }
      // Blink, unless the face already closes the eyes.
      let blink = 0;
      if (t > this.blinkAt) {
        const p = (t - this.blinkAt) / 0.16;
        blink = p < 1 ? Math.sin(p * Math.PI) : 0;
        if (p >= 1) this.blinkAt = t + 1.8 + Math.random() * 4.5;
      }
      em.setValue('blink', this.face === 'happy' || this.face === 'relaxed' ? 0 : blink);
    }

    // The body: breathing, a slow sway, a nod while talking.
    const b = this.bones;
    const s = t - this.t0;
    if (b.lu) b.lu.rotation.z = -1.15 + Math.sin(s * 1.6) * 0.015;
    if (b.ru) b.ru.rotation.z = 1.15 - Math.sin(s * 1.6) * 0.015;
    if (b.ll) b.ll.rotation.z = -0.12;
    if (b.rl) b.rl.rotation.z = 0.12;
    if (b.chest) b.chest.rotation.x = Math.sin(s * 1.6) * 0.012;
    if (b.spine) b.spine.rotation.z = Math.sin(s * 0.6) * 0.02;
    if (b.hips) b.hips.rotation.z = Math.sin(s * 0.6 + 0.4) * 0.01;
    if (b.neck) {
      b.neck.rotation.y = this.glance.x * 0.18;
      b.neck.rotation.x = -this.glance.y * 0.1 + this.lv * Math.sin(s * 9) * 0.04;
    }
    if (b.head) b.head.rotation.z = Math.sin(s * 0.5) * 0.03 + this.glance.x * -0.04;

    // The eyes follow a point in front of the face.
    this.target.position.set(this.glance.x * 0.6, (this.headY || 1.4) + this.glance.y * 0.35, 1.2);

    vrm.update(dt);
    this.renderer.render(this.scene, this.camera);
    this.raf = requestAnimationFrame(() => this.tick(t));
  }
  dispose() {
    cancelAnimationFrame(this.raf);
    this.resize.disconnect();
    if (this.vrm) this.V.VRMUtils.deepDispose(this.vrm.scene);
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }
}

// ── a rigged glTF that is no VRM ────────────────────────────────────────────
// A Meshy, Mixamo or Rigify export: a skeleton, maybe animation clips, and a
// face painted into its texture with no shapes to move. The arms come down
// from the T-pose, the body breathes and looks about, and a pack's `face`
// places eyelids (a blink, ^^ for happy) and a mouth drawn over the painted
// one, fixed to the head so they move with it. Clips play by use (the pack's
// `clips`, from the model or its baked `moves` file): `idle` loops at rest,
// `talk` while a line plays, `dance` on brb, and any use plays once on `move`.

// A part to the node names that usually move it, as three.js writes them
// (it drops . : / [ ] from names: Rigify's spine.005 is spine005).
const PARTS = {
  head: ['spine005', 'spine006', 'Head', 'head', 'mixamorigHead', 'J_Bip_C_Head'],
  neck: ['spine004', 'Neck', 'neck', 'mixamorigNeck', 'J_Bip_C_Neck'],
  chest: ['spine003', 'Chest', 'chest', 'UpperChest', 'mixamorigSpine2', 'J_Bip_C_Chest'],
  spine: ['spine001', 'Spine', 'spine', 'mixamorigSpine', 'J_Bip_C_Spine'],
  upper_arm_l: ['upper_armL', 'LeftArm', 'mixamorigLeftArm', 'UpperArmL', 'J_Bip_L_UpperArm'],
  upper_arm_r: ['upper_armR', 'RightArm', 'mixamorigRightArm', 'UpperArmR', 'J_Bip_R_UpperArm'],
  forearm_l: ['forearmL', 'LeftForeArm', 'mixamorigLeftForeArm', 'LowerArmL', 'J_Bip_L_LowerArm'],
  forearm_r: ['forearmR', 'RightForeArm', 'mixamorigRightForeArm', 'LowerArmR', 'J_Bip_R_LowerArm'],
  hand_l: ['handL', 'LeftHand', 'mixamorigLeftHand', 'J_Bip_L_Hand'],
  hand_r: ['handR', 'RightHand', 'mixamorigRightHand', 'J_Bip_R_Hand'],
};
const sanitize = (n) => String(n).replace(/\s/g, '_').replace(/[\[\].:/]/g, '');

// How a mouth shape opens: width and height against the mouth's full size.
const OPEN = { aa: [1, 1], oh: [0.6, 0.85], ou: [0.42, 0.5], ee: [0.95, 0.38], ih: [0.72, 0.45] };

export class GltfAvatar {
  constructor(host, V, gltf, model, moves = []) {
    this.V = V;
    this.host = host;
    this.model = model;
    this.renderer = new V.WebGLRenderer({ alpha: true, antialias: true, premultipliedAlpha: true });
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.outputColorSpace = V.SRGBColorSpace;
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
    this.renderer.domElement.className = 'vrm';
    host.appendChild(this.renderer.domElement);
    this.scene = new V.Scene();
    this.camera = new V.PerspectiveCamera(24, 1, 0.05, 30);
    const sun = new V.DirectionalLight(0xffffff, Math.PI * 0.9);
    sun.position.set(0.6, 1.2, 1.4);
    this.scene.add(sun, new V.HemisphereLight(0xffffff, 0x445566, 1.2));
    this.root = gltf.scene;
    this.root.traverse((o) => { o.frustumCulled = false; });
    this.scene.add(this.root);
    this.root.updateMatrixWorld(true);
    this.bones = {};
    for (const [part, names] of Object.entries(PARTS)) {
      const want = model.bones?.[part] ? [sanitize(model.bones[part])] : names;
      for (const n of want) {
        const b = this.root.getObjectByName(n);
        if (b) { this.bones[part] = b; break; }
      }
    }
    this.level = 0;
    this.lv = 0;
    this.face = 'neutral';
    this.mouth = null;
    this.open = { w: 0, h: 0 };
    this.blinkAt = now() + 2;
    this.glance = new Glance();
    this.layout = 'corner';
    // The face goes on first, at rest, where its points were measured.
    this.lids = [];
    this.mouthMesh = null;
    if (model.face) this.makeFace(model.face);
    this.armsDown();
    this.rest = new Map();
    for (const b of Object.values(this.bones)) this.rest.set(b, b.quaternion.clone());
    this.clips = [...(gltf.animations || []), ...moves];
    this.mixer = this.clips.length ? new V.AnimationMixer(this.root) : null;
    this.actions = new Map();
    this.action = null;   // what is playing now, looping or once
    this.once = null;     // a move playing once, back to the loop when done
    this.talking = false;
    this.mixer?.addEventListener('finished', (e) => { if (e.action === this.once) this.back(); });
    const head = new V.Vector3();
    (this.bones.head || this.root).getWorldPosition(head);
    this.headY = head.y;
    const box = new V.Box3().setFromObject(this.root);
    this.height = Math.max(0.5, box.max.y - box.min.y);
    this.floor = box.min.y;
    this.resize = new ResizeObserver(() => this.fit());
    this.resize.observe(host);
    this.fit();
    this.t0 = now();
    this.raf = requestAnimationFrame(() => this.tick(now()));
  }

  // One overlay: a plane drawn on a canvas, hung from its top edge so it
  // grows downward (a lid closing, a jaw dropping), fixed to the head.
  overlay(spot, draw, lit, color) {
    const V = this.V;
    const [w, h] = spot.size;
    const cv = document.createElement('canvas');
    cv.width = 256;
    cv.height = Math.max(32, Math.round(256 * h / w));
    draw(cv.getContext('2d'), cv.width, cv.height);
    const tex = new V.CanvasTexture(cv);
    tex.colorSpace = V.SRGBColorSpace;
    const geo = new V.PlaneGeometry(w, h);
    geo.translate(0, -h / 2, 0);
    const opts = { map: tex, transparent: true, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -4 };
    const mat = lit ? new V.MeshStandardMaterial({ ...opts, color: color || 0xffffff, roughness: 0.75, metalness: 0 })
      : new V.MeshBasicMaterial(opts);
    const mesh = new V.Mesh(geo, mat);
    mesh.renderOrder = 2;
    const n = new V.Vector3(...spot.normal).normalize();
    // Up along the face: the world's up, made square to the normal.
    const up = new V.Vector3(0, 1, 0).addScaledVector(n, -n.y).normalize();
    const at = new V.Vector3(...spot.at).addScaledVector(n, 0.0022).addScaledVector(up, h / 2);
    mesh.position.copy(at);
    mesh.lookAt(at.clone().add(n));
    // lookAt aims +z along n; turn about it so the plane's +y is the face's up.
    const yNow = new V.Vector3(0, 1, 0).applyQuaternion(mesh.quaternion);
    const turn = Math.atan2(new V.Vector3().crossVectors(yNow, up).dot(n), yNow.dot(up));
    mesh.rotateZ(turn);
    mesh.visible = false;
    (this.bones.head || this.root).attach(mesh);
    return mesh;
  }

  makeFace(face) {
    const skin = face.skin || '#f3eef0';
    const lid = (happy) => (c, w, h) => {
      // Feathered, so the patch has no edge of its own against the face.
      c.save();
      c.filter = `blur(${Math.round(h * 0.06)}px)`;
      c.fillStyle = skin;
      c.beginPath();
      c.ellipse(w / 2, h / 2, w * 0.44, h * 0.42, 0, 0, Math.PI * 2);
      c.fill();
      c.restore();
      c.strokeStyle = '#1b1a26';
      c.lineCap = 'round';
      c.lineWidth = h * 0.09;
      c.beginPath();
      // Closed: an arc that dips (︶). Happy: one that rises (︵).
      if (happy) c.arc(w / 2, h * 0.95, w * 0.34, Math.PI * 1.15, Math.PI * 1.85);
      else c.arc(w / 2, h * 0.18, w * 0.4, Math.PI * 0.2, Math.PI * 0.8);
      c.stroke();
    };
    // The lid patch comes out a little past the eye, to cover its lashes.
    for (const e of face.eyes || []) {
      const spot = { ...e, size: [e.size[0] * 1.15, e.size[1] * 1.2] };
      this.lids.push({ blink: this.overlay(spot, lid(false), true), happy: this.overlay(spot, lid(true), true) });
    }
    if (face.mouth) {
      const inside = face.mouth_color || '#3a1822';
      this.mouthMesh = this.overlay(face.mouth, (c, w, h) => {
        c.fillStyle = inside;
        c.beginPath();
        c.ellipse(w / 2, h * 0.5, w * 0.48, h * 0.48, 0, 0, Math.PI * 2);
        c.fill();
        c.save();
        c.clip();
        c.fillStyle = '#b4566a';
        c.beginPath();
        c.ellipse(w / 2, h * 1.02, w * 0.36, h * 0.38, 0, 0, Math.PI * 2);
        c.fill();
        c.restore();
      }, false);
    }
  }

  // Bring the arms down from the T-pose: aim each upper arm, then each
  // forearm, along a direction in the world, whatever the rig's own axes.
  armsDown() {
    const V = this.V;
    const aim = (bone, child, dir) => {
      if (!bone || !child || !bone.parent) return;
      this.root.updateMatrixWorld(true);
      const a = bone.getWorldPosition(new V.Vector3());
      const cur = child.getWorldPosition(new V.Vector3()).sub(a).normalize();
      const dq = new V.Quaternion().setFromUnitVectors(cur, dir.clone().normalize());
      const world = bone.getWorldQuaternion(new V.Quaternion());
      const parent = bone.parent.getWorldQuaternion(new V.Quaternion());
      bone.quaternion.copy(parent.invert().multiply(dq.multiply(world)));
    };
    const b = this.bones;
    for (const [up, fore, hand] of [['upper_arm_l', 'forearm_l', 'hand_l'], ['upper_arm_r', 'forearm_r', 'hand_r']]) {
      if (!b[up] || !b[fore]) continue;
      const side = Math.sign(b[fore].getWorldPosition(new V.Vector3()).x - b[up].getWorldPosition(new V.Vector3()).x) || 1;
      aim(b[up], b[fore], new V.Vector3(side * 0.2, -1, 0.02));
      aim(b[fore], b[hand], new V.Vector3(side * 0.1, -1, 0.2));
    }
    this.root.updateMatrixWorld(true);
  }

  fit() {
    const w = this.host.clientWidth || 1;
    const h = this.host.clientHeight || 1;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    const tan = Math.tan((this.camera.fov / 2) * Math.PI / 180);
    if (this.layout === 'brb') {
      // The whole body, for a dance.
      const span = this.height * 1.12 / Math.min(1, this.camera.aspect / 0.5);
      const y = this.floor + this.height * 0.5;
      this.camera.position.set(0, y, span / (2 * tan));
      this.camera.lookAt(0, y, 0);
    } else {
      // A bust: above the head to mid-chest, as the VRM is framed.
      const y = this.headY || 1.4;
      const span = (this.height * 0.42) / Math.min(1, this.camera.aspect / 0.62);
      this.camera.position.set(0, y, span / (2 * tan));
      this.camera.lookAt(0, y - this.height * 0.05, 0);
    }
    this.camera.updateProjectionMatrix();
  }

  faces() {
    if (!this.lids.length && !this.mouthMesh) return ['neutral'];
    return ['neutral', ...(this.lids.length ? ['happy'] : []), ...(this.mouthMesh ? ['surprised'] : [])];
  }
  setExpression(name) { this.face = name; }
  setLook(at) { this.glance.at = at in LOOK ? at : 'viewer'; }
  setLevel(x) { this.level = clamp(x, 0, 1); }
  setMouth(shape, open = 0) { this.mouth = shape == null ? null : { shape, open }; }
  setLayout(name) {
    this.layout = name;
    this.fit();
    this.settle();
  }

  // ── clips by use ──────────────────────────────────────────────────────────
  clip(use) {
    const name = this.model.clips?.[use];
    return name ? this.clips.find((c) => c.name === name) : null;
  }
  // One action per use and way of playing: a move played once is its own
  // action even when the same clip also loops.
  act(use, once) {
    const key = use + (once ? '/once' : '');
    if (!this.actions.has(key)) {
      const c = this.clip(use);
      const a = this.mixer.clipAction(once ? c.clone() : c);
      a.setLoop(this.V.LoopRepeat, once ? 1 : Infinity);
      a.clampWhenFinished = !!once;
      a.tracked = new Set(c.tracks.map((t) => t.name.split('.')[0]));
      this.actions.set(key, a);
    }
    return this.actions.get(key);
  }
  // What loops when no move is playing: the dance on brb, else talk while a
  // line plays, else idle; nothing (the drawn breathing) if the pack has none.
  wanted() {
    if (!this.mixer) return null;
    const use = this.layout === 'brb' ? 'dance' : this.talking && this.clip('talk') ? 'talk' : 'idle';
    return this.clip(use) ? this.act(use) : null;
  }
  fadeTo(next, seconds = 0.35) {
    if (next === this.action) return;
    this.action?.fadeOut(seconds);
    next?.reset().fadeIn(seconds).play();
    this.action = next;
  }
  settle() { if (!this.once) this.fadeTo(this.wanted()); }
  back() { this.once = null; this.fadeTo(this.wanted(), 0.4); }
  setTalking(on) { this.talking = !!on; this.settle(); }
  moves() { return Object.keys(this.model.clips || {}).filter((u) => this.clip(u)); }
  play(use) {
    if (!this.mixer || !this.clip(use) || this.layout === 'brb') return false;
    this.once = this.act(use, true);
    this.fadeTo(this.once, 0.25);
    return true;
  }

  // Turn a bone about an axis of the world, in its parent's frame.
  turn(bone, axis, angle) {
    if (!bone || !bone.parent || !angle) return;
    const V = this.V;
    bone.parent.updateWorldMatrix(true, false);
    const p = bone.parent.getWorldQuaternion(new V.Quaternion());
    const r = new V.Quaternion().setFromAxisAngle(axis, angle);
    bone.quaternion.premultiply(p.clone().invert().multiply(r).multiply(p));
  }

  tick(last) {
    const V = this.V;
    const t = now();
    const dt = Math.min(0.1, t - last);
    const s = t - this.t0;
    this.lv = ease(this.lv, this.level, 20, dt);
    this.glance.step(t, dt);
    const b = this.bones;
    if (this.mixer) this.mixer.update(dt);
    // A move eases back into the loop just before its last frame.
    if (this.once && this.once.time >= this.once.getClip().duration * 0.97 - Math.min(0.35, this.once.getClip().duration * 0.3)) this.back();
    if (this.action) {
      // The clip holds the body; the eyes still go where they are sent, and
      // the head nods with the voice, on top of whatever the clip says.
      const X = new V.Vector3(1, 0, 0), Y = new V.Vector3(0, 1, 0);
      const on = (part) => b[part] && this.action.tracked?.has(b[part].name);
      if (on('neck')) {
        this.turn(b.neck, Y, this.glance.x * 0.14);
        this.turn(b.neck, X, -this.glance.y * 0.08 + this.lv * Math.sin(s * 9) * 0.03);
      }
    } else {
      for (const [bone, q] of this.rest) bone.quaternion.copy(q);
      const X = new V.Vector3(1, 0, 0), Y = new V.Vector3(0, 1, 0), Z = new V.Vector3(0, 0, 1);
      this.turn(b.spine, Z, Math.sin(s * 0.6) * 0.02);
      this.turn(b.chest, X, Math.sin(s * 1.6) * 0.012);
      this.turn(b.neck, Y, this.glance.x * 0.18);
      this.turn(b.neck, X, -this.glance.y * 0.1 + this.lv * Math.sin(s * 9) * 0.04);
      this.turn(b.head, Z, Math.sin(s * 0.5) * 0.03 - this.glance.x * 0.04);
      this.turn(b.upper_arm_l, Z, Math.sin(s * 1.6) * 0.012);
      this.turn(b.upper_arm_r, Z, -Math.sin(s * 1.6) * 0.012);
    }

    // The eyes: a blink now and then, or ^^ while happy.
    let blink = 0;
    if (t > this.blinkAt) {
      const p = (t - this.blinkAt) / 0.16;
      blink = p < 1 ? Math.sin(p * Math.PI) : 0;
      if (p >= 1) this.blinkAt = t + 1.8 + Math.random() * 4.5;
    }
    for (const l of this.lids) {
      const happy = this.face === 'happy';
      l.happy.visible = happy;
      l.blink.visible = !happy && blink > 0.02;
      l.blink.scale.y = Math.max(0.02, blink);
    }

    // The mouth: the line's shapes, or the level when a line carries none;
    // a small round one held while surprised.
    if (this.mouthMesh) {
      const m = this.mouth;
      let want = [0, 0];
      if (m && OPEN[m.shape]) want = OPEN[m.shape].map((x) => x * m.open);
      else if (!m && this.lv > 0.05) want = OPEN.aa.map((x) => x * this.lv * 0.8);
      if (this.face === 'surprised' && want[1] < 0.45) want = [0.45, 0.6];
      this.open.w = ease(this.open.w, want[0], 28, dt);
      this.open.h = ease(this.open.h, want[1], 28, dt);
      this.mouthMesh.visible = this.open.h > 0.04;
      this.mouthMesh.scale.set(Math.max(0.05, this.open.w), Math.max(0.05, this.open.h), 1);
    }

    this.renderer.render(this.scene, this.camera);
    this.raf = requestAnimationFrame(() => this.tick(t));
  }

  dispose() {
    cancelAnimationFrame(this.raf);
    this.resize.disconnect();
    this.root.traverse((o) => {
      o.geometry?.dispose?.();
      for (const m of [].concat(o.material || [])) { m.map?.dispose?.(); m.dispose?.(); }
    });
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }
}

export async function makeAvatar(host, model) {
  if (!model) return null;
  if (model.kind === 'vrm' || model.kind === 'glb') {
    // Read the file once; it is a VRM if it says so, else a rigged glTF.
    const V = await import('./vendor.js');
    const loader = new V.GLTFLoader();
    loader.register((p) => new V.VRMLoaderPlugin(p));
    const gltf = await loader.loadAsync(fileUrl('/model/' + model.file));
    if (gltf.userData.vrm) return VrmAvatar.load(host, V, gltf);
    // A pack's moves, baked onto this body by the hub at import: the clips'
    // tracks name this model's own nodes.
    const moves = model.moves ? (await loader.loadAsync(fileUrl('/model/' + model.moves))).animations : [];
    return new GltfAvatar(host, V, gltf, model, moves);
  }
  if (model.kind === 'png') {
    const a = new PictureAvatar(host, model);
    await a.ready;
    return a;
  }
  return null;
}
