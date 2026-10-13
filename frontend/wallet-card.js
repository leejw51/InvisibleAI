// The wallet's stage: a three.js metal-and-glass card for whichever coin is
// active, floating in a slow particle orbit tinted for its chain. The face is
// drawn on a 2D canvas (artwork + balance), so the text tilts with the card.
// Yew owns the attributes; this element owns everything inside it.
//
// Everything that moves is a motion.dev tween eased exponentially in and out:
// balances count, the card flips when the coin changes, and a send plays a
// short choreography — the card kicks back, a particle burst arcs off, a
// shockwave ring spreads, and the amount itself flies out of the Confirm
// button into the card. Never renders a key or a phrase: the element only
// receives an address, a name, balances and the amount being sent.
import { animate } from 'motion';
import * as THREE from 'three';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';
import { easeInOutExpo, digits, money, masked, networkLabel, chainOf, flight } from './wallet-format.js';
import { balance } from './wallet-chains.js';

// The Yew page reads non-EVM balances through this.
window.cwbChains = { balance: (net, coin, address) => balance(net, coin, address) };

const ease = easeInOutExpo;
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const W = 1712, H = 1080; // ISO/IEC 7810 ID-1: 85.6 × 53.98 mm
const ART = {
  'cronos-mainnet': 'mainnet', 'cronos-testnet': 'testnet',
  arbitrum: 'arbitrum', solana: 'solana', cardano: 'cardano', midnight: 'midnight', ecash: 'ecash',
};
const artFor = network => `/assets/wallet-card-${ART[network] || ART[chainOf(network)] || 'testnet'}.jpg`;
// Low metalness keeps each artwork's own colours; testnets get more iridescence.
const TINT = {
  cronos: { metal: 0.12, rough: 0.42, iridescence: 0.06, sheen: '#f3d38a', rim: '#d9b86a', accent: '#f7d58b', glow: 'rgba(247,213,139,.55)', field: '#f2cf86' },
  arbitrum: { metal: 0.14, rough: 0.34, iridescence: 0.12, sheen: '#9fd8ff', rim: '#7fb8ff', accent: '#bfe3ff', glow: 'rgba(120,190,255,.6)', field: '#6fc3ff' },
  solana: { metal: 0.1, rough: 0.32, iridescence: 0.3, sheen: '#b9f5e4', rim: '#b28cff', accent: '#d7c6ff', glow: 'rgba(160,120,255,.6)', field: '#7ef0cf' },
  cardano: { metal: 0.16, rough: 0.38, iridescence: 0.1, sheen: '#b8c8ff', rim: '#8ea4e8', accent: '#cdd8ff', glow: 'rgba(130,160,255,.55)', field: '#8fb0ff' },
  midnight: { metal: 0.18, rough: 0.3, iridescence: 0.26, sheen: '#e2a8ff', rim: '#9c6ad8', accent: '#e9c7ff', glow: 'rgba(200,120,255,.6)', field: '#c58cff' },
  ecash: { metal: 0.12, rough: 0.34, iridescence: 0.14, sheen: '#a8ecff', rim: '#69c4e0', accent: '#c4f1ff', glow: 'rgba(100,210,240,.55)', field: '#7fe0ff' },
};
const tintFor = network => {
  const base = TINT[chainOf(network)] || TINT.cronos;
  // Test networks: a lilac rim and extra iridescence say "play money" at a glance.
  return /mainnet|one$/.test(network) ? base : { ...base, iridescence: Math.max(base.iridescence, 0.24), rim: '#c9bdf5' };
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
/** A soft round sprite for particles. */
function dotTexture() {
  const c = document.createElement('canvas'); c.width = c.height = 64;
  const g = c.getContext('2d').createRadialGradient(32, 32, 0, 32, 32, 32);
  g.addColorStop(0, 'rgba(255,255,255,1)'); g.addColorStop(0.35, 'rgba(255,255,255,.55)'); g.addColorStop(1, 'rgba(255,255,255,0)');
  const ctx = c.getContext('2d'); ctx.fillStyle = g; ctx.fillRect(0, 0, 64, 64);
  return new THREE.CanvasTexture(c);
}

class WalletCard extends HTMLElement {
  static observedAttributes = ['data-network', 'data-coin', 'data-balance', 'data-kind', 'data-native', 'data-address', 'data-holder', 'data-symbol', 'data-native-symbol', 'data-state', 'data-send'];
  constructor() {
    super();
    this.shown = { balance: 0, native: 0, flip: 0, glow: 0, kick: 0, burst: 0, wave: 0, hue: 0 };
    this.target = { balance: 0, native: 0 };
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
    this.lastSend = this.dataset.send || '';
    this.sync(true);
    // Rise out of the page.
    this.run('enter', this, { opacity: [0, 1], transform: ['translateY(28px) scale(.95)', 'translateY(0) scale(1)'] }, { duration: 1.2, ease });
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
  attributeChangedCallback(name) {
    if (!this.face) return;
    if (name === 'data-send') {
      if (this.dataset.send && this.dataset.send !== this.lastSend) { this.lastSend = this.dataset.send; this.send(); }
      return;
    }
    this.sync(false);
  }
  run(name, subject, keyframes, options) {
    this.controls.get(name)?.stop?.();
    if (reduced.matches) {
      // Reduced motion: land on the end state at once, no movement.
      const last = v => (Array.isArray(v) ? v[v.length - 1] : v);
      if (subject instanceof Element) for (const [k, v] of Object.entries(keyframes)) subject.style[k] = last(v);
      else { for (const [k, v] of Object.entries(keyframes)) subject[k] = last(v); options.onUpdate?.(); }
      options.onComplete?.();
      return Promise.resolve();
    }
    const control = animate(subject, keyframes, options);
    this.controls.set(name, control);
    control.then?.(() => { if (this.controls.get(name) === control) this.controls.delete(name); });
    return control;
  }
  async sync(first) {
    const network = this.dataset.network || 'cronos-mainnet';
    const coin = this.dataset.coin || network;
    const changed = coin !== this.coin;
    this.coin = coin;
    this.network = network;
    this.target = { balance: digits(this.dataset.balance), native: digits(this.dataset.native) };
    const art = await image(artFor(network));
    if (this.coin !== coin) return;
    const swap = () => {
      this.art = art; this.tint = tintFor(network); this.applyTint();
      // Snap the counter to the new coin's balance: counting from another coin's is meaningless.
      this.shown.balance = this.target.balance; this.shown.native = this.target.native;
      this.draw();
    };
    if (changed && !first) {
      // Flip a full turn; the new face is painted edge-on.
      let swapped = false;
      this.shown.flip = 0;
      this.run('flip', this.shown, { flip: [0, 1] }, {
        duration: 1.3, ease,
        onUpdate: () => { if (!swapped && this.shown.flip >= 0.5) { swapped = true; swap(); } },
        onComplete: () => { if (!swapped) swap(); this.shown.flip = 0; this.dataset.shownBalance = money(this.target.balance, this.places()); },
      });
    } else if (first || !this.art) swap();
    if (!changed || first) {
      // Balances count to their new value.
      this.run('balance', this.shown, { balance: this.target.balance, native: this.target.native }, {
        duration: first ? 1.6 : 1.2, ease,
        onUpdate: () => this.draw(),
        onComplete: () => { this.draw(); this.dataset.shownBalance = money(this.target.balance, this.places()); },
      });
    }
    // A soft glow pulses while the wallet is talking to the network.
    const busy = this.dataset.state === 'loading' || this.dataset.state === 'sending';
    this.run('glow', this.shown, { glow: busy ? 1 : 0 }, { duration: 0.8, ease, onUpdate: () => this.draw() });
  }
  /** Dollars read best with cents; every other coin gets four places. */
  places() { return this.dataset.kind === 'usd' ? 2 : 4; }
  applyTint() {
    if (!this.material) return;
    this.material.metalness = this.tint.metal;
    this.material.roughness = this.tint.rough;
    this.material.iridescence = this.tint.iridescence;
    this.material.sheenColor = new THREE.Color(this.tint.sheen);
    this.rim.color = new THREE.Color(this.tint.rim);
    // The orbit and the burst take the chain's colour.
    this.field.material.color = new THREE.Color(this.tint.field);
    // The burst and ring are drawn in the stronger rim colour: additive light
    // vanishes against the light stage.
    this.burst.material.color = new THREE.Color(this.tint.rim);
    this.ring.material.color = new THREE.Color(this.tint.rim);
  }
  /**
   * The send choreography. `data-send` is "<seq>|<amount>|<symbol>"; a new
   * sequence number plays it once. The amount flies out of the element marked
   * `data-flight-origin` (the Confirm button) into the card.
   */
  async send() {
    const [, amount = '', symbol = ''] = (this.dataset.send || '').split('|');
    this.dataset.sent = (Number(this.dataset.sent || 0) + 1).toString();
    if (reduced.matches) return;
    // Where the amount leaves from, in page coordinates: the button is about
    // to fade away and the page may scroll.
    const button = (document.querySelector('[data-flight-origin]') || this).getBoundingClientRect();
    const origin = { x: button.left + button.width / 2 + scrollX, y: button.top + button.height / 2 + scrollY };
    // Bring the card into view first; the send plays where it can be seen.
    const card = this.getBoundingClientRect();
    if (card.top < 0 || card.bottom > innerHeight) {
      const target = Math.max(0, scrollY + card.top - Math.max(16, (innerHeight - card.height) / 2));
      await animate(scrollY, target, { duration: 0.7, ease, onUpdate: y => scrollTo(0, y) });
    }
    // The card gathers itself, then kicks back as the value leaves it.
    this.run('kick', this.shown, { kick: [0, -0.35, 1, 0] }, { duration: 1.6, ease, times: [0, 0.25, 0.55, 1] });
    this.shown.burst = 0; this.shown.wave = 0;
    this.run('burst', this.shown, { burst: [0, 1] }, { duration: 1.9, ease, delay: 0.35 });
    this.run('wave', this.shown, { wave: [0, 1] }, { duration: 1.4, ease, delay: 0.4 });
    this.fly(`−${amount} ${symbol}`.trim(), { x: origin.x - scrollX, y: origin.y - scrollY });
  }
  /** The amount as a glowing label, from the Confirm button into the card and away. */
  fly(text, start) {
    const to = this.getBoundingClientRect();
    const label = document.createElement('div');
    label.className = 'wallet-flight';
    label.setAttribute('aria-hidden', 'true');
    label.textContent = text;
    document.body.append(label);
    const mid = { x: to.left + to.width / 2, y: to.top + to.height * 0.55 };
    const end = { x: to.right - to.width * 0.1, y: to.top - 40 };
    const at = (p, s) => `translate(${p.x}px, ${p.y}px) translate(-50%, -50%) scale(${s})`;
    animate(label, {
      transform: [at(start, 0.6), at(mid, 1.35), at(end, 0.5)],
      opacity: [0, 1, 0],
      filter: ['blur(6px)', 'blur(0px)', 'blur(4px)'],
    }, { duration: 1.8, ease, times: [0, 0.45, 1] }).then(() => label.remove());
  }
  draw() {
    const ctx = this.ctx;
    if (!ctx) return;
    const tint = this.tint || TINT.cronos;
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
    veil.addColorStop(0, 'rgba(8,6,24,.64)');
    veil.addColorStop(0.55, 'rgba(8,6,24,.2)');
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
    ctx.font = font(700, 44); ctx.globalAlpha = 0.92;
    ctx.fillText('INVISIBLE AI', 96, 140);
    const label = networkLabel(this.network);
    ctx.font = font(600, 34);
    const lw = ctx.measureText(label).width + 56;
    roundRect(ctx, W - 96 - lw, 92, lw, 64, 32);
    ctx.fillStyle = 'rgba(12,8,30,.42)'; ctx.fill();
    ctx.strokeStyle = 'rgba(255,255,255,.28)'; ctx.lineWidth = 2; ctx.stroke();
    ctx.fillStyle = tint.accent; ctx.fillText(label, W - 96 - lw + 28, 137);
    // EMV chip and contactless waves.
    const chip = ctx.createLinearGradient(96, 260, 256, 380);
    chip.addColorStop(0, '#f6e7b4'); chip.addColorStop(0.5, '#c9a65a'); chip.addColorStop(1, '#f1dc9c');
    roundRect(ctx, 96, 250, 168, 128, 22); ctx.fillStyle = chip; ctx.fill();
    ctx.strokeStyle = 'rgba(90,64,20,.45)'; ctx.lineWidth = 3;
    for (const y of [292, 336]) { ctx.beginPath(); ctx.moveTo(96, y); ctx.lineTo(264, y); ctx.stroke(); }
    ctx.beginPath(); ctx.moveTo(180, 250); ctx.lineTo(180, 378); ctx.stroke();
    ctx.strokeStyle = 'rgba(255,255,255,.75)'; ctx.lineWidth = 7; ctx.lineCap = 'round';
    for (let i = 0; i < 3; i++) { ctx.beginPath(); ctx.arc(300, 314, 26 + i * 22, -0.75, 0.75); ctx.stroke(); }
    // Balance.
    ctx.fillStyle = '#fff';
    ctx.font = font(500, 36); ctx.globalAlpha = 0.78;
    const symbol = (this.dataset.symbol || '').slice(0, 16);
    ctx.fillText(`${symbol} BALANCE`, 96, 560);
    const amount = money(this.shown.balance, this.places());
    ctx.globalAlpha = 1; ctx.font = font(700, amount.length > 12 ? 96 : 132);
    ctx.fillText(this.dataset.kind === 'usd' ? `$${amount}` : amount, 90, 700);
    ctx.font = font(500, 40); ctx.globalAlpha = 0.8;
    const fee = this.dataset.nativeSymbol || 'CRO';
    ctx.fillText(this.dataset.kind === 'native' ? `${fee} also pays network fees` : `${money(this.shown.native, 4)} ${fee} for fees`, 96, 770);
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
    this.run('tilt', this.tilt, { x: y * 0.22, y: x * 0.32 }, { duration: 0.9, ease });
    if (this.flat) this.flat.style.transform = `perspective(1100px) rotateX(${-this.tilt.x * 40}deg) rotateY(${this.tilt.y * 40}deg)`;
  }
  mount2d() {
    this.flat = this.face;
    this.dataset.renderer = '2d';
    this.append(this.face);
    // The 2D path still applies tint state used by the 3D one.
    this.material = null;
    const loop = () => {
      this.frame = requestAnimationFrame(loop);
      const flip = this.shown.flip * 360;
      this.flat.style.transform = `perspective(1100px) rotateX(${-this.tilt.x * 40 + this.shown.kick * 12}deg) rotateY(${this.tilt.y * 40 + flip}deg)`;
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
    this.renderer = renderer;
    this.append(renderer.domElement);
    const scene = new THREE.Scene();
    const pmrem = new THREE.PMREMGenerator(renderer);
    scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    scene.environmentIntensity = 0.45;
    pmrem.dispose();
    const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 60);
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
    // A moving highlight sweeps the clearcoat: the sheen of a metal card.
    const fill = new THREE.PointLight('#ffffff', 6, 12); fill.position.set(3, -2, 3); scene.add(fill);

    // The orbit: a tilted ring of soft particles circling behind the card.
    const dot = dotTexture();
    // Angle, radius, height and speed drawn independently, so the band reads
    // as a soft galaxy rather than as lines.
    const orbit = 1400, orbitPositions = new Float32Array(orbit * 3);
    const angles = Float32Array.from({ length: orbit }, () => Math.random() * Math.PI * 2);
    const radii = Float32Array.from({ length: orbit }, () => 2.1 + Math.pow(Math.random(), 1.6) * 1.6);
    const heights = Float32Array.from({ length: orbit }, () => (Math.random() - 0.5) * 0.28);
    const speeds = Float32Array.from({ length: orbit }, () => 0.5 + Math.random());
    const orbitGeometry = new THREE.BufferGeometry();
    orbitGeometry.setAttribute('position', new THREE.BufferAttribute(orbitPositions, 3));
    this.field = new THREE.Points(orbitGeometry, new THREE.PointsMaterial({ size: 0.045, map: dot, transparent: true, opacity: 0.7, depthWrite: false, blending: THREE.AdditiveBlending, color: '#f2cf86' }));
    this.field.rotation.x = 1.15;
    scene.add(this.field);

    // The send burst: particles that leave the card centre along arcs.
    const count = 220, burstPositions = new Float32Array(count * 3), seeds = Array.from({ length: count }, () => Math.random());
    const burstGeometry = new THREE.BufferGeometry();
    burstGeometry.setAttribute('position', new THREE.BufferAttribute(burstPositions, 3));
    this.burst = new THREE.Points(burstGeometry, new THREE.PointsMaterial({ size: 0.12, map: dot, transparent: true, opacity: 0, depthWrite: false }));
    scene.add(this.burst);
    // And a shockwave ring that spreads from the card.
    this.ring = new THREE.Mesh(new THREE.RingGeometry(0.97, 1, 128), new THREE.MeshBasicMaterial({ transparent: true, opacity: 0, side: THREE.DoubleSide, depthWrite: false }));
    scene.add(this.ring);

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
      camera.position.z = Math.max(cw / 0.62 / (half * camera.aspect), ch / 0.72 / half);
      camera.updateProjectionMatrix();
    };
    this.resizeObserver = new ResizeObserver(resize);
    this.resizeObserver.observe(this);
    resize();

    const timer = new THREE.Timer();
    const loop = now => {
      this.frame = requestAnimationFrame(loop);
      if (document.hidden) return;
      timer.update(now);
      const t = timer.getElapsed();
      const idle = reduced.matches ? 0 : 1;
      const kick = this.shown.kick;
      card.rotation.x = this.tilt.x + Math.sin(t * 0.7) * 0.04 * idle - kick * 0.35;
      card.rotation.y = this.tilt.y + Math.sin(t * 0.45) * 0.07 * idle + this.shown.flip * Math.PI * 2;
      card.position.y = Math.sin(t * 0.9) * 0.05 * idle + kick * 0.12;
      card.position.z = -kick * 0.6;
      shadow.material.opacity = 0.85 - card.position.y * 2;
      fill.position.x = 3 * Math.cos(t * 0.3);
      // Orbit: each particle on a wobbling ellipse; faster while busy.
      const spin = t * (0.08 + this.shown.glow * 0.25) * idle;
      for (let i = 0; i < orbit; i++) {
        const a = angles[i] + spin * speeds[i] / radii[i] * 2.4;
        const radius = radii[i] + this.shown.wave * 0.7;
        orbitPositions[i * 3] = Math.cos(a) * radius * 1.3;
        orbitPositions[i * 3 + 1] = Math.sin(a) * radius;
        orbitPositions[i * 3 + 2] = heights[i] + Math.sin(angles[i] * 3 + t * 0.4) * 0.05;
      }
      orbitGeometry.attributes.position.needsUpdate = true;
      // Burst: the particles ride their arcs while it plays.
      const b = this.shown.burst;
      this.burst.material.opacity = b > 0 && b < 1 ? Math.sin(Math.PI * b) : 0;
      if (this.burst.material.opacity > 0) {
        for (let i = 0; i < count; i++) {
          const p = flight(b * (0.75 + seeds[i] * 0.25), { x: 0, y: 0 }, { x: 3.4, y: 2.6 }, 0.9, seeds[i]);
          burstPositions[i * 3] = p.x; burstPositions[i * 3 + 1] = p.y; burstPositions[i * 3 + 2] = p.z + 0.1;
        }
        burstGeometry.attributes.position.needsUpdate = true;
      }
      const w = this.shown.wave;
      this.ring.material.opacity = w > 0 && w < 1 ? (1 - w) * 0.8 : 0;
      this.ring.scale.setScalar(0.4 + w * 3.2);
      renderer.render(scene, camera);
    };
    requestAnimationFrame(loop);
  }
}
if (!customElements.get('cwb-wallet-card')) customElements.define('cwb-wallet-card', WalletCard);

// Page motion. Panels marked `data-motion` fade in rising and fade out before
// Yew removes them (Rust marks a leaving panel `is-leaving`); children of
// `data-stagger` arrive one after another.
const seen = new WeakSet();
const enter = el => {
  if (seen.has(el) || reduced.matches) return;
  seen.add(el);
  const delay = Number(el.dataset.motionDelay || 0);
  animate(el, { opacity: [0, 1], transform: ['translateY(18px)', 'translateY(0)'] }, { duration: 0.9, delay, ease });
};
const stagger = list => {
  if (reduced.matches) return;
  [...list.children].forEach((child, i) => {
    if (seen.has(child)) return;
    seen.add(child);
    animate(child, { opacity: [0, 1], transform: ['translateY(10px) scale(.98)', 'translateY(0) scale(1)'] }, { duration: 0.6, delay: Math.min(i, 12) * 0.035, ease });
  });
};
const leave = el => {
  if (reduced.matches) { el.style.opacity = '0'; return; }
  animate(el, { opacity: [1, 0], transform: ['translateY(0) scale(1)', 'translateY(-10px) scale(.98)'] }, { duration: 0.4, ease });
};

// Gliders: a segmented control's highlight slides to the chosen option.
const glide = (group, instant) => {
  const glider = group.querySelector(':scope > .glider');
  const on = group.querySelector(':scope > [aria-pressed="true"], :scope > [aria-selected="true"], :scope > [aria-checked="true"]');
  if (!glider) return;
  if (!on) { glider.style.opacity = '0'; return; }
  const target = { transform: `translate(${on.offsetLeft}px, ${on.offsetTop}px)`, width: `${on.offsetWidth}px`, height: `${on.offsetHeight}px`, opacity: '1' };
  if (instant || reduced.matches) Object.assign(glider.style, target);
  else animate(glider, target, { duration: 0.7, ease });
};

// The amount being typed rolls digit by digit: each changed character slides in.
const roll = el => {
  const value = el.dataset.value || '';
  const previous = el.dataset.rendered ?? '';
  if (value === previous) return;
  el.dataset.rendered = value;
  el.textContent = '';
  [...(value || '0')].forEach((ch, i) => {
    const span = document.createElement('span');
    span.className = 'roll-digit';
    span.textContent = ch;
    el.append(span);
    if (!reduced.matches && ch !== previous[i]) {
      const up = previous.length <= value.length;
      animate(span, { transform: [`translateY(${up ? 60 : -60}%)`, 'translateY(0)'], opacity: [0, 1] }, { duration: 0.5, ease });
    }
  });
  if (!value) el.firstChild?.classList.add('is-placeholder');
};

const scan = root => {
  if (!(root instanceof Element)) return;
  const all = sel => [...(root.matches(sel) ? [root] : []), ...root.querySelectorAll(sel)];
  for (const el of all('[data-motion]')) enter(el);
  for (const el of all('[data-stagger]')) stagger(el);
  for (const el of all('[data-glider]')) glide(el, true);
  for (const el of all('[data-roll]')) roll(el);
};
new MutationObserver(records => {
  for (const record of records) {
    const el = record.target;
    if (record.type === 'attributes') {
      if (record.attributeName === 'class' && el.classList.contains('is-leaving') && !el.dataset.left) { el.dataset.left = '1'; leave(el); }
      else if (record.attributeName === 'data-value' && el.matches('[data-roll]')) roll(el);
      else if (el.parentElement?.matches('[data-glider]')) glide(el.parentElement, false);
    } else {
      if (el instanceof Element && el.matches('[data-stagger]')) stagger(el);
      for (const node of record.addedNodes) scan(node);
    }
  }
}).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'aria-pressed', 'aria-selected', 'aria-checked', 'data-value'] });
addEventListener('resize', () => document.querySelectorAll('[data-glider]').forEach(g => glide(g, true)));
scan(document.body);
