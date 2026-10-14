// The send stage: a small game played beside the send form. Two planets are
// the two wallets, with their addresses and balances in front. When the
// transfer goes, a comet with a glowing trail leads a stream of coins across
// the nebula while the amount ticks over, the sender's balance counts down and
// the recipient's counts up; on arrival a shockwave and a burst of sparks, and
// the final balances read back from the chain. three.js draws (with bloom),
// motion.dev tweens every number and every move on an exponential in/out ease.
//
// Yew owns the attributes; this element owns everything inside it. It never
// sees a key: only public addresses, balances and the amount.
import { animate } from 'motion';
import * as THREE from 'three';
import { EffectComposer } from 'three/examples/jsm/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/examples/jsm/postprocessing/RenderPass.js';
import { UnrealBloomPass } from 'three/examples/jsm/postprocessing/UnrealBloomPass.js';
import { OutputPass } from 'three/examples/jsm/postprocessing/OutputPass.js';
import { easeInOutExpo, digits, amount as fmt } from './wallet-format.js';

const ease = easeInOutExpo;
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const loader = new THREE.TextureLoader();
const texture = src => {
  const t = loader.load(src);
  t.colorSpace = THREE.SRGBColorSpace;
  return t;
};
/** A soft round glow, for halos, the comet head and sparks. */
function glowTexture(inner = 'rgba(255,255,255,1)') {
  const c = document.createElement('canvas'); c.width = c.height = 128;
  const ctx = c.getContext('2d');
  const g = ctx.createRadialGradient(64, 64, 0, 64, 64, 64);
  g.addColorStop(0, inner); g.addColorStop(0.25, 'rgba(255,255,255,.55)'); g.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = g; ctx.fillRect(0, 0, 128, 128);
  return new THREE.CanvasTexture(c);
}
/** `0x9858…da94` style, for any chain's address. */
const shortAddress = a => (a && a.length > 14 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a || '');
const webgl = () => { try { const c = document.createElement('canvas'); return !!(c.getContext('webgl2') || c.getContext('webgl')); } catch { return false; } };

// Points whose size and alpha fade along their age: one material serves the
// comet's trail, the sparks and the star dust.
const pointsMaterial = map => new THREE.ShaderMaterial({
  uniforms: { map: { value: map }, scale: { value: 1 } },
  vertexShader: `attribute float size; attribute float alpha; attribute vec3 tint; varying float vAlpha; varying vec3 vTint;
    uniform float scale;
    void main() { vAlpha = alpha; vTint = tint; vec4 mv = modelViewMatrix * vec4(position, 1.0);
      gl_PointSize = size * scale * (300.0 / -mv.z); gl_Position = projectionMatrix * mv; }`,
  fragmentShader: `uniform sampler2D map; varying float vAlpha; varying vec3 vTint;
    void main() { vec4 t = texture2D(map, gl_PointCoord); gl_FragColor = vec4(vTint * t.rgb, t.a * vAlpha); }`,
  transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
});
function pointCloud(count, map) {
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(new Float32Array(count * 3), 3));
  g.setAttribute('size', new THREE.BufferAttribute(new Float32Array(count), 1));
  g.setAttribute('alpha', new THREE.BufferAttribute(new Float32Array(count), 1));
  g.setAttribute('tint', new THREE.BufferAttribute(new Float32Array(count * 3).fill(1), 3));
  const p = new THREE.Points(g, pointsMaterial(map));
  p.frustumCulled = false;
  return p;
}

class SendStage extends HTMLElement {
  /** How long the finished stage holds before the page moves on (ms). */
  static PAUSE = 1500;
  static observedAttributes = ['data-phase', 'data-amount', 'data-from-before', 'data-to-before', 'data-from-after', 'data-to-after', 'data-from', 'data-to', 'data-symbol'];
  constructor() {
    super();
    // Everything that moves, as numbers motion.dev can tween.
    this.v = { from: 0, to: 0, amount: 0, launch: 0, impact: 0, flash: 0, fail: 0, shake: 0, zoom: 0 };
    this.controls = new Map();
    this.sparks = [];
  }
  connectedCallback() {
    if (this.hud) return;
    this.buildHud();
    if (webgl()) this.mount(); else this.dataset.renderer = '2d';
    this.phase = null;
    this.sync();
    this.run('enter', this, { opacity: [0, 1], transform: ['translateY(20px) scale(.97)', 'translateY(0px) scale(1)'] }, { duration: 0.9, ease });
  }
  disconnectedCallback() {
    clearTimeout(this.finishing);
    for (const c of this.controls.values()) c.stop?.();
    cancelAnimationFrame(this.frame);
    this.resizeObserver?.disconnect();
    this.composer?.dispose?.();
    this.renderer?.dispose();
    this.renderer = undefined;
    this.hud = undefined;
  }
  attributeChangedCallback() { if (this.hud) this.sync(); }
  run(name, subject, keyframes, options = {}) {
    this.controls.get(name)?.stop?.();
    if (reduced.matches) {
      const last = v => (Array.isArray(v) ? v[v.length - 1] : v);
      if (subject instanceof Element) for (const [k, v] of Object.entries(keyframes)) subject.style[k] = last(v);
      else { for (const [k, v] of Object.entries(keyframes)) subject[k] = last(v); options.onUpdate?.(); }
      options.onComplete?.();
      return Promise.resolve();
    }
    const control = animate(subject, keyframes, { ease, ...options });
    this.controls.set(name, control);
    return control;
  }
  places() { return Number(this.dataset.places || 4); }
  label(key, fallback) { return this.dataset[`label${key}`] || fallback; }

  buildHud() {
    const hud = document.createElement('div');
    hud.className = 'send-hud';
    hud.innerHTML = `
      <div class="send-side from"><span class="send-tag"></span><code class="send-address"></code><strong class="send-balance" data-testid="stage-from-balance"></strong><em class="send-symbol"></em></div>
      <div class="send-center"><span class="send-amount" data-testid="stage-amount"></span><em class="send-symbol"></em><span class="send-status" data-testid="stage-status"></span></div>
      <div class="send-side to"><span class="send-tag"></span><code class="send-address"></code><strong class="send-balance" data-testid="stage-to-balance"></strong><em class="send-symbol"></em></div>`;
    this.append(hud);
    this.hud = hud;
    const q = s => hud.querySelector(s);
    this.el = {
      fromTag: q('.from .send-tag'), toTag: q('.to .send-tag'),
      fromAddress: q('.from .send-address'), toAddress: q('.to .send-address'),
      fromBalance: q('.from .send-balance'), toBalance: q('.to .send-balance'),
      amount: q('.send-amount'), status: q('.send-status'), symbols: hud.querySelectorAll('.send-symbol'),
    };
  }
  /** Write the numbers as they are now; called on every tween frame. */
  paint() {
    const p = this.places();
    this.el.fromBalance.textContent = fmt(this.v.from, p);
    this.el.toBalance.textContent = fmt(this.v.to, p);
    this.el.amount.textContent = fmt(this.v.amount, p);
  }
  sync() {
    const d = this.dataset;
    this.el.fromTag.textContent = this.label('From', 'FROM');
    this.el.toTag.textContent = this.label('To', 'TO');
    this.el.fromAddress.textContent = shortAddress(d.from);
    this.el.toAddress.textContent = shortAddress(d.to);
    this.el.fromAddress.title = d.from || '';
    this.el.toAddress.title = d.to || '';
    for (const s of this.el.symbols) s.textContent = d.symbol || '';
    const phase = d.phase || 'ready';
    if (phase === this.phase) {
      // Yew can set the balances after the element is attached: a ready stage
      // follows them; a running one keeps its course.
      if (phase === 'ready') {
        Object.assign(this.v, { from: digits(d.fromBefore), to: digits(d.toBefore) });
        this.run('amount', this.v, { amount: digits(d.amount) }, { duration: 0.8, onUpdate: () => this.paint(), onComplete: () => this.paint() });
        this.paint();
      }
      return;
    }
    const previous = this.phase;
    this.phase = phase;
    this.classList.remove('is-ready', 'is-flying', 'is-done', 'is-failed');
    this.classList.add(`is-${phase === 'pending' ? 'flying' : phase}`);
    const amount = digits(d.amount), fromBefore = digits(d.fromBefore), toBefore = digits(d.toBefore);
    const done = () => { this.dataset.shownFrom = fmt(this.v.from, this.places()); this.dataset.shownTo = fmt(this.v.to, this.places()); this.paint(); };
    if (phase === 'ready') {
      this.el.status.textContent = this.label('Ready', 'Ready to send');
      Object.assign(this.v, { from: fromBefore, to: toBefore, amount: 0, launch: 0, impact: 0, fail: 0 });
      this.paint();
      // The amount to send counts up into view.
      this.run('amount', this.v, { amount }, { duration: 1.1, onUpdate: () => this.paint(), onComplete: done });
    } else if (phase === 'flying' || phase === 'pending') {
      this.el.status.textContent = this.label(phase === 'pending' ? 'Pending' : 'Flying', 'Sending…');
      if (previous === 'flying' || previous === 'pending') return;
      // The value leaves one wallet and arrives at the other as the coins fly.
      this.v.amount = amount; this.v.launch = 0;
      this.run('launch', this.v, { launch: [0, 1] }, { duration: 2.6 });
      this.run('from', this.v, { from: [fromBefore, fromBefore - amount] }, { duration: 2.6, onUpdate: () => this.paint() });
      this.run('to', this.v, { to: [toBefore, toBefore + amount] }, { duration: 2.6, delay: 0.35, onUpdate: () => this.paint() });
      this.run('amount', this.v, { amount: [amount, 0] }, { duration: 2.6, onUpdate: () => this.paint() });
      this.run('zoom', this.v, { zoom: [0, 1, 0.6] }, { duration: 2.6, times: [0, 0.5, 1] });
      this.launchStream();
    } else if (phase === 'done') {
      this.el.status.textContent = this.label('Done', 'Arrived!');
      // The chain's own numbers have the last word.
      const fromAfter = d.fromAfter !== undefined ? digits(d.fromAfter) : fromBefore - amount;
      const toAfter = d.toAfter !== undefined ? digits(d.toAfter) : toBefore + amount;
      this.v.amount = 0;
      this.run('from', this.v, { from: fromAfter }, { duration: 1.2, onUpdate: () => this.paint() });
      this.run('to', this.v, { to: toAfter }, { duration: 1.2, onUpdate: () => this.paint(), onComplete: done });
      this.run('impact', this.v, { impact: [0, 1] }, { duration: 1.6 });
      this.run('flash', this.v, { flash: [0, 1, 0] }, { duration: 1.2, times: [0, 0.2, 1] });
      this.run('shake', this.v, { shake: [0, 1, 0] }, { duration: 0.7, times: [0, 0.15, 1] });
      this.burst(this.toPos, 260, true);
      this.run('pop', this.el.toBalance, { transform: ['scale(1)', 'scale(1.25)', 'scale(1)'] }, { duration: 0.9, times: [0, 0.3, 1] });
      this.finish(true);
    } else if (phase === 'failed') {
      this.el.status.textContent = this.label('Failed', 'Not sent');
      // Nothing left the wallet: the numbers return.
      this.run('from', this.v, { from: fromBefore }, { duration: 1, onUpdate: () => this.paint() });
      this.run('to', this.v, { to: toBefore }, { duration: 1, onUpdate: () => this.paint(), onComplete: done });
      this.run('fail', this.v, { fail: [0, 1] }, { duration: 0.8 });
      this.run('shake', this.v, { shake: [0, 1, 0] }, { duration: 0.6, times: [0, 0.2, 1] });
      this.finish(false);
    }
  }
  /**
   * Once the arrival (or failure) has fully played, and after a moment to
   * take it in, tell the page the transfer is over: the card takes it from
   * there. `stage-finished` carries whether the money arrived.
   */
  finish(ok) {
    clearTimeout(this.finishing);
    const played = reduced.matches ? 0 : 1800; // the longest arrival effect
    this.finishing = setTimeout(() => {
      this.dataset.finished = ok ? 'done' : 'failed';
      this.dispatchEvent(new CustomEvent('stage-finished', { bubbles: true, detail: { ok } }));
    }, played + SendStage.PAUSE);
  }

  mount() {
    this.dataset.renderer = 'webgl';
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 0.95;
    this.renderer = renderer;
    this.prepend(renderer.domElement);
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(40, 1, 0.1, 100);
    camera.position.set(0, 0.2, 9);
    this.camera = camera;

    // Backdrop: the Grok nebula, drifting slowly.
    const nebula = texture('/assets/wallet-send-nebula.jpg');
    nebula.wrapS = nebula.wrapT = THREE.MirroredRepeatWrapping;
    const backdrop = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.MeshBasicMaterial({ map: nebula, color: '#9aa0c8' }));
    backdrop.position.z = -12;
    scene.add(backdrop);

    // Star dust that twinkles.
    const dot = glowTexture();
    const stars = pointCloud(500, dot);
    const sp = stars.geometry.attributes.position.array, ss = stars.geometry.attributes.size.array;
    const twinkle = Float32Array.from({ length: 500 }, () => Math.random() * 6.28);
    for (let i = 0; i < 500; i++) { sp[i * 3] = (Math.random() - 0.5) * 22; sp[i * 3 + 1] = (Math.random() - 0.5) * 12; sp[i * 3 + 2] = -4 - Math.random() * 6; ss[i] = 0.05 + Math.random() * 0.12; }
    scene.add(stars);

    // The two wallets: textured planets with a halo and an orbit ring.
    const planet = (src, color, x) => {
      const group = new THREE.Group();
      const map = texture(src);
      const body = new THREE.Mesh(new THREE.SphereGeometry(1, 64, 48), new THREE.MeshStandardMaterial({ map, emissiveMap: map, emissive: '#ffffff', emissiveIntensity: 0.12, roughness: 0.55, metalness: 0.05 }));
      const halo = new THREE.Sprite(new THREE.SpriteMaterial({ map: dot, color, transparent: true, opacity: 0.3, depthWrite: false, blending: THREE.AdditiveBlending }));
      halo.scale.setScalar(3.4);
      const ring = new THREE.Mesh(new THREE.TorusGeometry(1.55, 0.015, 8, 160), new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.7 }));
      ring.rotation.x = 1.25;
      group.add(halo, body, ring);
      group.position.x = x;
      scene.add(group);
      return { group, body, halo, ring };
    };
    this.from = planet('/assets/wallet-send-planet-from.jpg', '#c9a4ff', -3.4);
    this.to = planet('/assets/wallet-send-planet-to.jpg', '#6ff5df', 3.4);
    this.fromPos = this.from.group.position;
    this.toPos = this.to.group.position;
    scene.add(new THREE.AmbientLight('#ffffff', 0.35));
    // A key light from the upper left gives the planets a lit side and a night side.
    const sun = new THREE.DirectionalLight('#fff3e0', 1.8); sun.position.set(-4, 3, 5); scene.add(sun);

    // The comet: a bright head with a fading trail of its recent positions.
    const trailLength = 72;
    this.trail = pointCloud(trailLength, dot);
    this.trailHistory = [];
    scene.add(this.trail);
    this.head = new THREE.Sprite(new THREE.SpriteMaterial({ map: dot, color: '#fff2c4', transparent: true, depthWrite: false, blending: THREE.AdditiveBlending }));
    this.head.scale.setScalar(0);
    scene.add(this.head);

    // Coins: the Grok sprite, black melting away under additive blending.
    const coinMap = texture('/assets/wallet-send-coin.jpg');
    this.coins = Array.from({ length: 28 }, () => {
      const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: coinMap, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, opacity: 0 }));
      s.scale.setScalar(0.5);
      scene.add(s);
      return { sprite: s, t: -1, speed: 1, seed: Math.random() };
    });

    // Sparks: a pool shared by the trail's shedding, arrivals and the finale.
    this.sparkCloud = pointCloud(900, dot);
    scene.add(this.sparkCloud);

    // A shockwave ring at the recipient.
    this.wave = new THREE.Mesh(new THREE.RingGeometry(0.95, 1, 128), new THREE.MeshBasicMaterial({ color: '#7ff7e4', transparent: true, opacity: 0, side: THREE.DoubleSide, depthWrite: false, blending: THREE.AdditiveBlending }));
    this.wave.position.copy(this.toPos);
    scene.add(this.wave);

    const composer = new EffectComposer(renderer);
    composer.addPass(new RenderPass(scene, camera));
    // Bloom only on what is meant to glow: a high threshold spares the planets.
    const bloom = new UnrealBloomPass(new THREE.Vector2(256, 256), 0.85, 0.5, 0.55);
    composer.addPass(bloom);
    composer.addPass(new OutputPass());
    this.composer = composer;

    const resize = () => {
      const { width, height } = this.getBoundingClientRect();
      if (!width || !height) return;
      renderer.setSize(width, height, false);
      composer.setSize(width, height);
      camera.aspect = width / height;
      // Keep both planets in frame however narrow the stage gets.
      camera.position.z = Math.max(9, 8.6 / camera.aspect + 3);
      camera.updateProjectionMatrix();
      const h = 2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)) * (camera.position.z + 12);
      backdrop.scale.set(Math.max(h * camera.aspect, h) * 1.15, Math.max(h * camera.aspect, h) * 1.15, 1);
      this.trail.material.uniforms.scale.value = this.sparkCloud.material.uniforms.scale.value = stars.material.uniforms.scale.value = height / 400;
    };
    this.resizeObserver = new ResizeObserver(resize);
    this.resizeObserver.observe(this);
    resize();

    const arc = (t, lift, from = this.fromPos, to = this.toPos) => {
      const u = 1 - t;
      const mid = { x: (from.x + to.x) / 2, y: Math.max(from.y, to.y) + lift };
      return new THREE.Vector3(u * u * (from.x + 1.05) + 2 * u * t * mid.x + t * t * (to.x - 1.05), u * u * from.y + 2 * u * t * mid.y + t * t * to.y, 0.4 * Math.sin(Math.PI * t));
    };
    this.arc = arc;
    const timer = new THREE.Timer();
    let last = 0;
    const loop = now => {
      this.frame = requestAnimationFrame(loop);
      if (document.hidden) return;
      timer.update(now);
      const t = timer.getElapsed(), dt = Math.min(0.05, t - last); last = t;
      const v = this.v;
      // Planets turn; the recipient swells on arrival, the sender shudders on failure.
      this.from.body.rotation.y += dt * 0.25; this.to.body.rotation.y -= dt * 0.2;
      this.from.ring.rotation.z += dt * 0.3; this.to.ring.rotation.z -= dt * 0.25;
      const shake = v.shake * 0.12;
      // Lifted above the HUD; bobbing gently.
      this.from.group.position.y = 0.75 + Math.sin(t * 0.8) * 0.12 + (v.fail ? Math.sin(t * 60) * shake : 0);
      this.to.group.position.y = 0.75 + Math.cos(t * 0.7) * 0.12;
      this.wave.position.copy(this.toPos);
      this.to.group.scale.setScalar(1 + v.flash * 0.18);
      this.to.halo.material.opacity = 0.3 + v.flash * 0.6 + (this.phase === 'flying' || this.phase === 'pending' ? Math.sin(t * 4) * 0.1 : 0);
      this.from.halo.material.color.set(v.fail > 0.5 ? '#ff6b7a' : '#c9a4ff');
      camera.position.x = Math.sin(t * 40) * shake;
      camera.position.y = 0.2 + Math.cos(t * 37) * shake;
      camera.fov = 40 - v.zoom * 4; camera.updateProjectionMatrix();
      backdrop.material.map.offset.set(Math.sin(t * 0.03) * 0.03, Math.cos(t * 0.025) * 0.02);
      // Stars twinkle.
      const sa = stars.geometry.attributes.alpha.array;
      for (let i = 0; i < 500; i++) sa[i] = 0.35 + 0.35 * Math.sin(t * 1.5 + twinkle[i]);
      stars.geometry.attributes.alpha.needsUpdate = true;
      this.step(dt, t);
      composer.render();
    };
    requestAnimationFrame(loop);
  }
  /** Start a stream of coins and the comet that leads them. */
  launchStream() {
    if (!this.coins) return;
    this.coins.forEach((c, i) => { c.t = -i * 0.07; c.speed = 0.55 + Math.random() * 0.25; c.seed = Math.random(); });
    this.trailHistory = [];
  }
  /** Spray `count` sparks from `at`; `party` adds colour. */
  burst(at, count, party = false) {
    if (!at || reduced.matches) return;
    const colors = party ? [[1, 0.85, 0.4], [0.5, 1, 0.9], [0.85, 0.6, 1], [1, 0.55, 0.7]] : [[1, 0.9, 0.6]];
    for (let i = 0; i < count; i++) {
      const a = Math.random() * Math.PI * 2, z = (Math.random() - 0.5) * 2, speed = 1.5 + Math.random() * (party ? 4.5 : 2.5);
      this.sparks.push({ p: new THREE.Vector3(at.x, at.y, at.z ?? 0), v: new THREE.Vector3(Math.cos(a) * speed, Math.sin(a) * speed, z), life: 1, decay: 0.6 + Math.random() * 0.8, size: 0.12 + Math.random() * 0.2, color: colors[i % colors.length] });
    }
    if (this.sparks.length > 900) this.sparks.splice(0, this.sparks.length - 900);
  }
  step(dt, t) {
    const v = this.v, flying = this.phase === 'flying' || this.phase === 'pending';
    // Comet head rides the main arc during the launch.
    const L = v.launch;
    const head = this.arc(L, 2.2);
    const live = flying && L > 0 && L < 1;
    this.head.position.copy(head);
    this.head.scale.setScalar(live ? 0.9 + Math.sin(t * 30) * 0.08 : 0);
    if (live) {
      // Two points per frame, the second halfway back, so the trail reads as one ribbon.
      const prev = this.trailHistory[0];
      if (prev) this.trailHistory.unshift(prev.clone().lerp(head, 0.5));
      this.trailHistory.unshift(head.clone());
      // Shed a few sparks behind the head.
      for (let i = 0; i < 3; i++) this.sparks.push({ p: head.clone(), v: new THREE.Vector3((Math.random() - 0.7) * 1.2, (Math.random() - 0.5) * 1.2, (Math.random() - 0.5)), life: 1, decay: 1.6 + Math.random(), size: 0.08 + Math.random() * 0.1, color: [1, 0.9, 0.65] });
    } else if (this.trailHistory.length) this.trailHistory.pop();
    if (this.trailHistory.length > 72) this.trailHistory.length = 72;
    if (!live && this.trailHistory.length) this.trailHistory.pop();
    const tp = this.trail.geometry.attributes.position.array, ts = this.trail.geometry.attributes.size.array, ta = this.trail.geometry.attributes.alpha.array, tt = this.trail.geometry.attributes.tint.array;
    for (let i = 0; i < 72; i++) {
      const h = this.trailHistory[i];
      const k = 1 - i / 72;
      if (h) { tp[i * 3] = h.x; tp[i * 3 + 1] = h.y; tp[i * 3 + 2] = h.z; }
      ts[i] = h ? 1.1 * k + 0.12 : 0;
      ta[i] = h ? Math.pow(k, 1.6) * 0.9 : 0;
      tt[i * 3] = 1; tt[i * 3 + 1] = 0.75 + 0.25 * k; tt[i * 3 + 2] = 0.45 + 0.5 * k;
    }
    for (const name of ['position', 'size', 'alpha', 'tint']) this.trail.geometry.attributes[name].needsUpdate = true;
    // Coins stream along their own arcs; after the launch a slower trickle
    // keeps going while the network confirms.
    for (const c of this.coins) {
      if (!flying && c.t < 0) { c.sprite.material.opacity = 0; continue; }
      if (flying) c.t += dt * c.speed * (L >= 1 ? 0.6 : 1);
      if (c.t >= 1) {
        this.burst({ x: this.toPos.x - 0.9, y: this.toPos.y + (c.seed - 0.5) * 0.6, z: 0 }, 10);
        c.t = flying && L >= 1 ? -Math.random() * 0.8 : -9;
      }
      const k = Math.max(0, Math.min(1, c.t));
      const p = this.arc(ease(k), 1.2 + c.seed * 1.8);
      c.sprite.position.set(p.x, p.y + Math.sin(c.seed * 20 + k * 8) * 0.15, p.z + (c.seed - 0.5));
      c.sprite.material.opacity = c.t > 0 && c.t < 1 ? Math.min(1, Math.sin(Math.PI * k) * 1.6) : 0;
      c.sprite.material.rotation = k * 10 + c.seed * 6;
      c.sprite.scale.setScalar(0.45 + Math.sin(Math.PI * k) * 0.55);
    }
    // Shockwave on arrival.
    this.wave.material.opacity = v.impact > 0 && v.impact < 1 ? (1 - v.impact) * 0.9 : 0;
    this.wave.scale.setScalar(1 + v.impact * 3.5);
    // Sparks fall under a little gravity and fade.
    const n = Math.min(this.sparks.length, 900);
    const sp = this.sparkCloud.geometry.attributes.position.array, ss = this.sparkCloud.geometry.attributes.size.array, sa = this.sparkCloud.geometry.attributes.alpha.array, sc = this.sparkCloud.geometry.attributes.tint.array;
    for (let i = this.sparks.length - 1; i >= 0; i--) {
      const s = this.sparks[i];
      s.life -= dt * s.decay;
      if (s.life <= 0) { this.sparks.splice(i, 1); continue; }
      s.v.multiplyScalar(1 - dt * 1.4); s.v.y -= dt * 1.2;
      s.p.addScaledVector(s.v, dt);
    }
    for (let i = 0; i < 900; i++) {
      const s = this.sparks[i];
      if (!s || i >= n) { sa[i] = 0; ss[i] = 0; continue; }
      sp[i * 3] = s.p.x; sp[i * 3 + 1] = s.p.y; sp[i * 3 + 2] = s.p.z;
      ss[i] = s.size; sa[i] = s.life;
      sc[i * 3] = s.color[0]; sc[i * 3 + 1] = s.color[1]; sc[i * 3 + 2] = s.color[2];
    }
    for (const name of ['position', 'size', 'alpha', 'tint']) this.sparkCloud.geometry.attributes[name].needsUpdate = true;
  }
}
if (!customElements.get('cwb-send-stage')) customElements.define('cwb-send-stage', SendStage);
