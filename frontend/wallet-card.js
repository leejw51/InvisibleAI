// The wallet's credit card: a three.js metal-and-glass card whose face is
// drawn on a 2D canvas (artwork + balances), so the text tilts with the card.
// Yew owns the attributes; this element owns everything inside it. Balances
// tween with motion.dev, eased exponentially in and out; the page's panels
// fade in and out with the same curve. Never renders a key or a phrase: the
// element only ever receives an address, a name and balances.
import { animate } from 'motion';
import * as THREE from 'three';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';

// Standard easeInOutExpo; the same curve as crates/frontend/src/tween.rs.
export const easeInOutExpo = t => t <= 0 ? 0 : t >= 1 ? 1 : t < 0.5 ? 2 ** (20 * t - 10) / 2 : (2 - 2 ** (-20 * t + 10)) / 2;
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const W = 1712, H = 1080; // ISO/IEC 7810 ID-1: 85.6 × 53.98 mm
const ART = { 'cronos-mainnet': '/assets/wallet-card-mainnet.jpg', 'cronos-testnet': '/assets/wallet-card-testnet.jpg' };
const TINT = {
  // Low metalness and a whisper of iridescence: the artwork keeps its own
  // colours (navy and gold stay navy and gold) and the clearcoat adds gloss.
  'cronos-mainnet': { metal: 0.12, rough: 0.42, iridescence: 0.06, sheen: '#f3d38a', rim: '#d9b86a', accent: '#f7d58b', glow: 'rgba(247,213,139,.55)' },
  'cronos-testnet': { metal: 0.1, rough: 0.36, iridescence: 0.22, sheen: '#cfc2ff', rim: '#c9bdf5', accent: '#e4dbff', glow: 'rgba(196,181,253,.6)' },
};
const images = new Map();
function image(src) {
  if (!images.has(src)) images.set(src, new Promise(resolve => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = src;
  }));
  return images.get(src);
}
function digits(value) {
  const n = Number(value);
  return Number.isFinite(n) ? n : 0;
}
function money(value, places = 2) {
  return value.toLocaleString('en-US', { minimumFractionDigits: places, maximumFractionDigits: places });
}
function masked(address) {
  if (!/^0x[0-9a-fA-F]{40}$/.test(address || '')) return '0x•••• •••• •••• ••••';
  return `${address.slice(0, 6)}  ••••  ••••  ${address.slice(-4)}`;
}
function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}
function webgl() {
  try {
    const c = document.createElement('canvas');
    return !!(c.getContext('webgl2') || c.getContext('webgl'));
  } catch { return false; }
}

class WalletCard extends HTMLElement {
  static observedAttributes = ['data-network', 'data-usdc', 'data-native', 'data-address', 'data-holder', 'data-symbol', 'data-native-symbol', 'data-state'];
  constructor() {
    super();
    this.shown = { usdc: 0, native: 0, flip: 0, glow: 0 };
    this.target = { usdc: 0, native: 0 };
    this.tilt = { x: 0, y: 0 };
    this.controls = new Map();
  }
  connectedCallback() {
    if (this.face) return;
    this.face = document.createElement('canvas');
    this.face.width = W; this.face.height = H;
    this.face.className = 'wallet-card-face';
    this.ctx = this.face.getContext('2d');
    if (webgl()) this.mount3d(); else this.mount2d();
    this.onMove = event => this.pointer(event);
    this.onLeave = () => this.pointer(null);
    this.addEventListener('pointermove', this.onMove, { passive: true });
    this.addEventListener('pointerleave', this.onLeave, { passive: true });
    this.sync(true);
    // Fade the whole card in, rising out of the page.
    this.run('enter', this, { opacity: [0, 1], transform: ['translateY(24px) scale(.96)', 'translateY(0) scale(1)'] }, { duration: 1.1, ease: easeInOutExpo });
  }
  disconnectedCallback() {
    for (const control of this.controls.values()) control.stop?.();
    this.controls.clear();
    this.removeEventListener('pointermove', this.onMove);
    this.removeEventListener('pointerleave', this.onLeave);
    cancelAnimationFrame(this.frame);
    this.resizeObserver?.disconnect();
    this.renderer?.dispose();
    this.texture?.dispose();
    this.renderer = undefined;
    this.face = undefined;
  }
  attributeChangedCallback() {
    if (this.face) this.sync(false);
  }
  run(name, subject, keyframes, options) {
    this.controls.get(name)?.stop?.();
    if (reduced.matches) {
      // Reduced motion: land on the end state at once, no movement.
      if (subject instanceof Element) {
        for (const [k, v] of Object.entries(keyframes)) subject.style[k] = Array.isArray(v) ? v[v.length - 1] : v;
      } else {
        for (const [k, v] of Object.entries(keyframes)) subject[k] = Array.isArray(v) ? v[v.length - 1] : v;
        options.onUpdate?.();
      }
      options.onComplete?.();
      return;
    }
    const control = animate(subject, keyframes, options);
    this.controls.set(name, control);
    control.then?.(() => { if (this.controls.get(name) === control) this.controls.delete(name); });
  }
  async sync(first) {
    const network = this.dataset.network || 'cronos-testnet';
    const changed = network !== this.network;
    this.network = network;
    this.target = { usdc: digits(this.dataset.usdc), native: digits(this.dataset.native) };
    const art = await image(ART[network] || ART['cronos-testnet']);
    if (this.network !== network) return;
    const swap = () => { this.art = art; this.tint = TINT[network] || TINT['cronos-testnet']; this.applyTint(); this.draw(); };
    if (changed && !first) {
      // Flip the card a full turn; the new face is painted edge-on.
      let swapped = false;
      this.shown.flip = 0;
      this.run('flip', this.shown, { flip: [0, 1] }, {
        duration: 1.2, ease: easeInOutExpo,
        onUpdate: () => { if (!swapped && this.shown.flip >= 0.5) { swapped = true; swap(); } },
        onComplete: () => { if (!swapped) swap(); this.shown.flip = 0; },
      });
    } else swap();
    // Balances count to their new value — exponential in, exponential out.
    this.run('usdc', this.shown, { usdc: this.target.usdc, native: this.target.native }, {
      duration: first ? 1.6 : 1.1, ease: easeInOutExpo,
      onUpdate: () => this.draw(),
      onComplete: () => { this.draw(); this.dataset.shownUsdc = money(this.target.usdc); },
    });
    // A soft glow pulses while the wallet is talking to the network.
    const busy = this.dataset.state === 'loading' || this.dataset.state === 'sending';
    this.run('glow', this.shown, { glow: busy ? 1 : 0 }, { duration: 0.8, ease: easeInOutExpo, onUpdate: () => this.draw() });
  }
  applyTint() {
    if (!this.material) return;
    this.material.metalness = this.tint.metal;
    this.material.roughness = this.tint.rough;
    this.material.iridescence = this.tint.iridescence;
    this.material.sheenColor = new THREE.Color(this.tint.sheen);
    this.rim.color = new THREE.Color(this.tint.rim);
  }
  draw() {
    const ctx = this.ctx;
    if (!ctx) return;
    const tint = this.tint || TINT['cronos-testnet'];
    ctx.clearRect(0, 0, W, H);
    ctx.save();
    roundRect(ctx, 0, 0, W, H, 72);
    ctx.clip();
    if (this.art) {
      const scale = Math.max(W / this.art.width, H / this.art.height);
      const w = this.art.width * scale, h = this.art.height * scale;
      ctx.drawImage(this.art, (W - w) / 2, (H - h) / 2, w, h);
    } else {
      ctx.fillStyle = '#3b3270'; ctx.fillRect(0, 0, W, H);
    }
    // Legibility veil: darker where the text sits.
    const veil = ctx.createLinearGradient(0, H, W * 0.7, 0);
    veil.addColorStop(0, 'rgba(8,6,24,.62)');
    veil.addColorStop(0.55, 'rgba(8,6,24,.18)');
    veil.addColorStop(1, 'rgba(8,6,24,0)');
    ctx.fillStyle = veil; ctx.fillRect(0, 0, W, H);
    if (this.shown.glow > 0.01) {
      const g = ctx.createRadialGradient(W * 0.8, H * 0.2, 0, W * 0.8, H * 0.2, W * 0.7);
      g.addColorStop(0, tint.glow.replace(/[\d.]+\)$/, `${0.55 * this.shown.glow})`));
      g.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    }
    ctx.fillStyle = '#fff';
    ctx.textBaseline = 'alphabetic';
    const font = (weight, size) => `${weight} ${size}px "Inter", "SF Pro Display", system-ui, -apple-system, sans-serif`;
    // Brand and network
    ctx.font = font(700, 44); ctx.globalAlpha = 0.92;
    ctx.fillText('INVISIBLE AI', 96, 140);
    const label = this.network === 'cronos-mainnet' ? 'CRONOS MAINNET' : 'CRONOS TESTNET';
    ctx.font = font(600, 34);
    const lw = ctx.measureText(label).width + 56;
    roundRect(ctx, W - 96 - lw, 92, lw, 64, 32);
    ctx.fillStyle = 'rgba(12,8,30,.42)'; ctx.fill();
    ctx.strokeStyle = 'rgba(255,255,255,.28)'; ctx.lineWidth = 2; ctx.stroke();
    ctx.fillStyle = tint.accent; ctx.fillText(label, W - 96 - lw + 28, 137);
    // EMV chip
    const chip = ctx.createLinearGradient(96, 260, 256, 380);
    chip.addColorStop(0, '#f6e7b4'); chip.addColorStop(0.5, '#c9a65a'); chip.addColorStop(1, '#f1dc9c');
    roundRect(ctx, 96, 250, 168, 128, 22); ctx.fillStyle = chip; ctx.fill();
    ctx.strokeStyle = 'rgba(90,64,20,.45)'; ctx.lineWidth = 3;
    for (const y of [292, 336]) { ctx.beginPath(); ctx.moveTo(96, y); ctx.lineTo(264, y); ctx.stroke(); }
    ctx.beginPath(); ctx.moveTo(180, 250); ctx.lineTo(180, 378); ctx.stroke();
    // Contactless waves
    ctx.strokeStyle = 'rgba(255,255,255,.75)'; ctx.lineWidth = 7; ctx.lineCap = 'round';
    for (let i = 0; i < 3; i++) { ctx.beginPath(); ctx.arc(300, 314, 26 + i * 22, -0.75, 0.75); ctx.stroke(); }
    // Balance
    ctx.fillStyle = '#fff';
    ctx.font = font(500, 36); ctx.globalAlpha = 0.78;
    ctx.fillText(`${this.dataset.symbol || 'USDC'} BALANCE`, 96, 560);
    ctx.globalAlpha = 1; ctx.font = font(700, 132);
    ctx.fillText(`$${money(this.shown.usdc)}`, 90, 700);
    ctx.font = font(500, 40); ctx.globalAlpha = 0.8;
    ctx.fillText(`${money(this.shown.native, 4)} ${this.dataset.nativeSymbol || 'CRO'} for fees`, 96, 770);
    // Address and holder
    ctx.globalAlpha = 0.95; ctx.font = font(500, 50);
    ctx.fillText(masked(this.dataset.address), 96, 920);
    ctx.globalAlpha = 0.72; ctx.font = font(600, 32);
    ctx.fillText((this.dataset.holder || 'YOUR WALLET').toUpperCase().slice(0, 32), 96, 990);
    ctx.globalAlpha = 1;
    ctx.restore();
    if (this.texture) this.texture.needsUpdate = true;
  }
  pointer(event) {
    if (reduced.matches) return;
    const r = this.getBoundingClientRect();
    const x = event ? ((event.clientX - r.left) / r.width) * 2 - 1 : 0;
    const y = event ? ((event.clientY - r.top) / r.height) * 2 - 1 : 0;
    this.run('tilt', this.tilt, { x: y * 0.22, y: x * 0.32 }, { type: 'spring', stiffness: 140, damping: 18, mass: 0.8 });
    if (this.flat) this.flat.style.transform = `perspective(1100px) rotateX(${-this.tilt.x * 40}deg) rotateY(${this.tilt.y * 40}deg)`;
  }
  mount2d() {
    this.flat = this.face;
    this.dataset.renderer = '2d';
    this.append(this.face);
    const loop = () => {
      this.frame = requestAnimationFrame(loop);
      const flip = this.shown.flip * 360;
      this.flat.style.transform = `perspective(1100px) rotateX(${-this.tilt.x * 40}deg) rotateY(${this.tilt.y * 40 + flip}deg)`;
    };
    loop();
  }
  mount3d() {
    this.dataset.renderer = 'webgl';
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: 'low-power' });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    // Neutral tone mapping keeps the artwork's colours true to the source.
    renderer.toneMapping = THREE.NeutralToneMapping;
    renderer.toneMappingExposure = 1;
    this.renderer = renderer;
    this.append(renderer.domElement);
    const scene = new THREE.Scene();
    const pmrem = new THREE.PMREMGenerator(renderer);
    scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    scene.environmentIntensity = 0.45;
    pmrem.dispose();
    const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 50);
    camera.position.set(0, 0, 6);

    // Card body: a rounded rectangle with a thin bevelled edge.
    const cw = 3.4, ch = cw * H / W, r = 0.14;
    const shape = new THREE.Shape();
    shape.moveTo(-cw / 2 + r, -ch / 2);
    shape.lineTo(cw / 2 - r, -ch / 2); shape.quadraticCurveTo(cw / 2, -ch / 2, cw / 2, -ch / 2 + r);
    shape.lineTo(cw / 2, ch / 2 - r); shape.quadraticCurveTo(cw / 2, ch / 2, cw / 2 - r, ch / 2);
    shape.lineTo(-cw / 2 + r, ch / 2); shape.quadraticCurveTo(-cw / 2, ch / 2, -cw / 2, ch / 2 - r);
    shape.lineTo(-cw / 2, -ch / 2 + r); shape.quadraticCurveTo(-cw / 2, -ch / 2, -cw / 2 + r, -ch / 2);
    const body = new THREE.ExtrudeGeometry(shape, { depth: 0.03, bevelEnabled: true, bevelThickness: 0.012, bevelSize: 0.012, bevelSegments: 4, curveSegments: 24 });
    body.translate(0, 0, -0.015);
    this.rim = new THREE.MeshPhysicalMaterial({ color: '#cfc2ff', metalness: 1, roughness: 0.25, clearcoat: 1 });
    const card = new THREE.Group();
    card.add(new THREE.Mesh(body, this.rim));

    // Face: the 2D canvas as a texture, under a clearcoat with iridescence.
    this.texture = new THREE.CanvasTexture(this.face);
    this.texture.colorSpace = THREE.SRGBColorSpace;
    this.texture.anisotropy = renderer.capabilities.getMaxAnisotropy();
    const faceGeometry = new THREE.ShapeGeometry(shape, 24);
    const uv = faceGeometry.attributes.uv;
    for (let i = 0; i < uv.count; i++) uv.setXY(i, (uv.getX(i) + cw / 2) / cw, (uv.getY(i) + ch / 2) / ch);
    this.material = new THREE.MeshPhysicalMaterial({
      map: this.texture, metalness: 0.1, roughness: 0.36, clearcoat: 0.55, clearcoatRoughness: 0.12,
      iridescence: 0.22, iridescenceIOR: 1.5, sheen: 0.35, sheenRoughness: 0.4, sheenColor: new THREE.Color('#cfc2ff'),
      side: THREE.DoubleSide,
    });
    const face = new THREE.Mesh(faceGeometry, this.material);
    face.position.z = 0.03;
    card.add(face);
    scene.add(card);

    const key = new THREE.DirectionalLight('#ffffff', 1.1); key.position.set(-3, 4, 6); scene.add(key);
    scene.add(new THREE.AmbientLight('#ffffff', 0.55));
    // A small moving highlight sweeps the clearcoat: the "sheen" of a metal card.
    const fill = new THREE.PointLight('#ffffff', 6, 12); fill.position.set(3, -2, 3); scene.add(fill);

    // Soft shadow on the "table" below the card.
    const shadowCanvas = document.createElement('canvas'); shadowCanvas.width = shadowCanvas.height = 128;
    const sctx = shadowCanvas.getContext('2d');
    const sg = sctx.createRadialGradient(64, 64, 0, 64, 64, 64);
    sg.addColorStop(0, 'rgba(20,10,60,.45)'); sg.addColorStop(1, 'rgba(20,10,60,0)');
    sctx.fillStyle = sg; sctx.fillRect(0, 0, 128, 128);
    const shadow = new THREE.Mesh(new THREE.PlaneGeometry(4.2, 1.1), new THREE.MeshBasicMaterial({ map: new THREE.CanvasTexture(shadowCanvas), transparent: true, depthWrite: false }));
    shadow.position.set(0, -ch / 2 - 0.55, -0.4);
    shadow.rotation.x = -1.2;
    scene.add(shadow);

    const resize = () => {
      const { width, height } = this.getBoundingClientRect();
      if (!width || !height) return;
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      // Keep the whole card in frame on narrow screens.
      const half = 2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
      camera.position.z = Math.max(cw / 0.82 / (half * camera.aspect), ch / 0.8 / half);
      camera.updateProjectionMatrix();
    };
    this.resizeObserver = new ResizeObserver(resize);
    this.resizeObserver.observe(this);
    resize();

    const clock = new THREE.Clock();
    const loop = () => {
      this.frame = requestAnimationFrame(loop);
      if (document.hidden) return;
      const t = clock.getElapsedTime();
      const idle = reduced.matches ? 0 : 1;
      card.rotation.x = this.tilt.x + Math.sin(t * 0.7) * 0.04 * idle;
      card.rotation.y = this.tilt.y + Math.sin(t * 0.45) * 0.07 * idle + this.shown.flip * Math.PI * 2;
      card.position.y = Math.sin(t * 0.9) * 0.05 * idle;
      shadow.material.opacity = 0.85 - card.position.y * 2;
      fill.position.x = 3 * Math.cos(t * 0.3);
      renderer.render(scene, camera);
    };
    loop();
  }
}
if (!customElements.get('cwb-wallet-card')) customElements.define('cwb-wallet-card', WalletCard);

// Page motion: panels fade in rising, and fade out before Yew removes them.
// Rust marks a leaving panel with `is-leaving` and drops it once the fade ends.
const seen = new WeakSet();
const enter = el => {
  if (seen.has(el) || reduced.matches) return;
  seen.add(el);
  const delay = Number(el.dataset.motionDelay || 0);
  animate(el, { opacity: [0, 1], transform: ['translateY(18px)', 'translateY(0)'] }, { duration: 0.9, delay, ease: easeInOutExpo });
};
const leave = el => {
  if (reduced.matches) { el.style.opacity = '0'; return; }
  animate(el, { opacity: [1, 0], transform: ['translateY(0) scale(1)', 'translateY(-10px) scale(.98)'] }, { duration: 0.4, ease: easeInOutExpo });
};
const scan = root => {
  if (!(root instanceof Element)) return;
  for (const el of [...(root.matches('[data-motion]') ? [root] : []), ...root.querySelectorAll('[data-motion]')]) enter(el);
};
new MutationObserver(records => {
  for (const record of records) {
    if (record.type === 'attributes') {
      const el = record.target;
      if (el.classList.contains('is-leaving') && !el.dataset.left) { el.dataset.left = '1'; leave(el); }
    } else for (const node of record.addedNodes) scan(node);
  }
}).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class'] });
scan(document.body);
