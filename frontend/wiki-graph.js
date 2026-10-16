import * as THREE from 'three';

// A small force-directed 3D map of the Wiki: tags are the hubs, messages hang
// off their tags, pictures show their own thumbnail. Titles are drawn as HTML
// labels next to the canvas, so screen readers and search get real text; WebGL
// only shows shapes and lines. Everything renders from the JSON the backend
// already gives the page; nothing is fetched from elsewhere but the backend's
// own thumbnails.
const TAG = 0xa98ac7, MESSAGE = 0x7aaea1, NOTE = 0x8fb38a, PHOTO = 0xd9b26b, VIDEO = 0xc98aa6, CODE = 0x6f8fbf, LINK = 0xc9bddb, RELATED = 0xb7a2d3, ACTIVE = 0x7760cf;
const colorOf = kind => kind === 'tag' ? TAG : kind === 'photo' ? PHOTO : kind === 'video' ? VIDEO : kind === 'code' ? CODE : kind === 'note' ? NOTE : MESSAGE;
class WikiGraph extends HTMLElement {
  static observedAttributes = ['data-graph', 'data-selected'];
  connectedCallback() {
    if (this.renderer) return;
    this.motion = matchMedia('(prefers-reduced-motion: reduce)');
    this.resources = [];
    this.nodes = []; this.edges = [];
    this.rotation = {x: .35, y: 0}; this.drag = null; this.distance = 26;
    try {
      this.renderer = new THREE.WebGLRenderer({alpha: true, antialias: true, powerPreference: 'low-power'});
      this.renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
      this.renderer.setClearColor(0, 0);
      this.renderer.domElement.setAttribute('aria-hidden', 'true');
      this.append(this.renderer.domElement);
      this.labels = document.createElement('div'); this.labels.className = 'wiki-graph-labels'; this.append(this.labels);
      this.scene = new THREE.Scene();
      this.camera = new THREE.PerspectiveCamera(42, 1, .1, 200);
      this.scene.add(new THREE.AmbientLight(0xffffff, 1.8));
      const light = new THREE.DirectionalLight(0xffffff, 2.4); light.position.set(-5, 8, 10); this.scene.add(light);
      this.group = new THREE.Group(); this.scene.add(this.group);
      this.raycaster = new THREE.Raycaster(); this.pointer = new THREE.Vector2();
      this.loader = new THREE.TextureLoader();
      this.resizeObserver = new ResizeObserver(() => this.resize()); this.resizeObserver.observe(this);
      // Touch is handled here, not by the browser (touch-action: none in CSS):
      // one finger drags, two fingers pinch to zoom, a tap selects.
      this.pointers = new Map();
      this.onDown = e => {
        this.pressed = true;
        this.renderer.domElement.style.cursor = '';
        this.pointers.set(e.pointerId, {x: e.clientX, y: e.clientY});
        if (this.pointers.size === 2) { this.drag = null; this.pinch = this.span(); return; }
        this.drag = {x: e.clientX, y: e.clientY, moved: false};
      };
      this.onCancel = e => { this.pointers.delete(e.pointerId); if (this.pointers.size < 2) this.pinch = 0; this.drag = null; this.pressed = false; };
      this.onMove = e => {
        if (this.pointers.has(e.pointerId)) this.pointers.set(e.pointerId, {x: e.clientX, y: e.clientY});
        if (this.pointers.size >= 2) { const d = this.span(); if (this.pinch) this.zoomBy((this.pinch - d) * .06); this.pinch = d; return; }
        // A node under a resting pointer shows a hand, so a small dot reads as a button.
        if (!this.drag) { if (e.target === this.renderer.domElement) this.renderer.domElement.style.cursor = this.hit(e) ? 'pointer' : ''; return; }
        const dx = e.clientX - this.drag.x, dy = e.clientY - this.drag.y; if (Math.abs(dx) + Math.abs(dy) > 3) this.drag.moved = true; this.rotation.y += dx * .006; this.rotation.x = Math.max(-1.2, Math.min(1.2, this.rotation.x + dy * .006)); this.drag.x = e.clientX; this.drag.y = e.clientY; };
      // A tap selects; a second tap on the same spot within 350 ms (iPad has no
      // dblclick for touch) opens the node's page.
      // Only a press that began on the canvas can pick: pointerup is heard on
      // the window, and a tap on a reader or a lightbox above the map must not
      // reach the node under it.
      this.onUp = e => {
        if (!this.pressed) return;
        this.pressed = false;
        const pinching = this.pointers.size >= 2;
        this.pointers.delete(e.pointerId);
        if (pinching) { this.pinch = 0; this.drag = null; return; }
        const moved = this.drag?.moved; this.drag = null; if (moved) return;
        const now = performance.now(), last = this.lastTap;
        this.lastTap = {t: now, x: e.clientX, y: e.clientY};
        const double = last && now - last.t < 350 && Math.abs(e.clientX - last.x) < 24 && Math.abs(e.clientY - last.y) < 24;
        this.pick(e, double);
        if (double) this.lastTap = null;
      };
      this.onWheel = e => { e.preventDefault(); this.zoomBy(e.deltaY * .02); };
      // The map holds still while the pointer rests over it, so a dot stays
      // where the eye found it until the tap lands.
      this.onEnter = () => { this.hover = true; }; this.onLeave = () => { this.hover = false; this.renderer.domElement.style.cursor = ''; this.schedule(); };
      this.renderer.domElement.addEventListener('pointerenter', this.onEnter); this.renderer.domElement.addEventListener('pointerleave', this.onLeave);
      this.renderer.domElement.addEventListener('pointerdown', this.onDown);
      addEventListener('pointermove', this.onMove); addEventListener('pointerup', this.onUp); addEventListener('pointercancel', this.onCancel);
      this.renderer.domElement.addEventListener('wheel', this.onWheel, {passive: false});
      this.intersection = new IntersectionObserver(([entry]) => { this.visible = entry.isIntersecting; this.schedule(); }); this.intersection.observe(this);
      this.addEventListener('webglcontextlost', e => { e.preventDefault(); cancelAnimationFrame(this.frame); this.setAttribute('data-renderer', 'unavailable'); }, true);
      this.visible = true;
      this.build(); this.resize(); this.setAttribute('data-renderer', 'three'); this.schedule();
    } catch {
      this.setAttribute('data-renderer', 'unavailable');
      this.cleanup();
    }
  }
  attributeChangedCallback(name) {
    if (!this.renderer) return;
    if (name === 'data-graph') this.build();
    // The selected label is marked at once: a phone's tag panel can push the
    // canvas out of view, and an offscreen map does not draw.
    if (name === 'data-selected') for (const n of this.nodes) n.label?.classList.toggle('is-selected', n.id === (this.dataset.selected || ''));
    this.schedule();
  }
  track(r) { this.resources.push(r); return r; }
  sphere(kind, size) {
    const radius = kind === 'tag' ? .34 + Math.min(.6, (size || 1) * .05) : .2;
    const color = colorOf(kind);
    return new THREE.Mesh(this.track(new THREE.SphereGeometry(radius, 12, 12)),
      this.track(new THREE.MeshStandardMaterial({color, roughness: .4, metalness: .1})));
  }
  build() {
    for (const child of [...this.group.children]) this.group.remove(child);
    for (const r of this.resources) r.dispose?.(); this.resources = [];
    this.labels.replaceChildren();
    let data = {nodes: [], edges: []};
    try { data = JSON.parse(this.dataset.graph || '{}'); } catch {}
    const nodes = (data.nodes || []).slice(0, 450);
    const index = new Map(nodes.map((n, i) => [n.id, i]));
    const edges = (data.edges || []).filter(e => index.has(e.from) && index.has(e.to));
    const seed = n => { const t = n * 9301 + 49297; return ((t % 233280) / 233280) - .5; };
    this.nodes = nodes.map((n, i) => ({
      id: n.id, kind: n.kind, title: n.title || '', url: n.url, document: n.document || '',
      p: new THREE.Vector3(seed(i * 3 + 1) * 14, seed(i * 3 + 2) * 10, seed(i * 3 + 3) * 14), v: new THREE.Vector3(),
      mesh: this.sphere(n.kind, n.size),
    }));
    for (const n of this.nodes) { n.mesh.userData.id = n.id; this.group.add(n.mesh); }
    // A picture becomes a small photo card once its thumbnail arrives; until
    // then, and if the backend has no frame for it, it stays a sphere.
    for (const [i, n] of nodes.entries()) {
      if ((n.kind !== 'photo' && n.kind !== 'video') || !n.thumbnail) continue;
      const node = this.nodes[i];
      this.loader.load(n.thumbnail, texture => {
        if (!this.renderer || !this.nodes.includes(node)) { texture.dispose(); return; }
        texture.colorSpace = THREE.SRGBColorSpace;
        const sprite = new THREE.Sprite(this.track(new THREE.SpriteMaterial({map: this.track(texture)})));
        const aspect = texture.image && texture.image.width ? texture.image.width / texture.image.height : 1;
        sprite.scale.set(aspect >= 1 ? 1.1 : 1.1 * aspect, aspect >= 1 ? 1.1 / aspect : 1.1, 1);
        sprite.userData.id = node.id;
        this.group.remove(node.mesh); node.mesh = sprite; node.sprite = true; node.base = sprite.scale.clone(); this.group.add(sprite);
        this.schedule();
      }, undefined, () => {});
    }
    this.edges = edges.map(e => {
      const line = new THREE.Line(this.track(new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(), new THREE.Vector3()])),
        this.track(new THREE.LineBasicMaterial({color: e.kind === 'related' ? RELATED : LINK, transparent: true, opacity: e.kind === 'related' ? .4 : .5})));
      this.group.add(line);
      return {a: index.get(e.from), b: index.get(e.to), w: e.weight || 1, kind: e.kind, line};
    });
    for (const n of this.nodes) {
      const label = document.createElement('button'); label.type = 'button'; label.className = 'wiki-graph-label';
      label.classList.toggle('is-tag', n.kind === 'tag');
      label.textContent = n.kind === 'tag' ? '#' + n.title : n.title; label.dataset.id = n.id; label.dataset.kind = n.kind;
      label.addEventListener('click', () => this.select(n));
      this.labels.append(label); n.label = label;
    }
    this.steps = 0;
    this.layout(60);
  }
  layout(iterations) {
    const nodes = this.nodes; if (!nodes.length) return;
    for (let it = 0; it < iterations; it++) {
      for (let i = 0; i < nodes.length; i++) for (let j = i + 1; j < nodes.length; j++) {
        const a = nodes[i], b = nodes[j]; const d = a.p.clone().sub(b.p); const dist = Math.max(.3, d.length());
        const f = 2.2 / (dist * dist); d.normalize().multiplyScalar(f); a.v.add(d); b.v.sub(d);
      }
      for (const e of this.edges) {
        const a = nodes[e.a], b = nodes[e.b]; const d = b.p.clone().sub(a.p); const dist = d.length();
        const rest = e.kind === 'related' ? Math.max(2.4, 5 - e.w) : 2.6;
        const f = (dist - rest) * .03 * Math.min(3, e.w); d.normalize().multiplyScalar(f); a.v.add(d); b.v.sub(d);
      }
      for (const n of nodes) { n.v.sub(n.p.clone().multiplyScalar(.004)); n.p.add(n.v.multiplyScalar(.6)); n.v.multiplyScalar(.55); }
    }
    this.steps += iterations;
  }
  select(node) {
    this.setAttribute('data-selected', node.id);
    this.dispatchEvent(new CustomEvent('wiki-select', {detail: {id: node.id, kind: node.kind, url: node.url}, bubbles: true}));
  }
  // Opens the node's page: a message's source, a tag's document.
  open(node) {
    this.dispatchEvent(new CustomEvent('wiki-open', {detail: {id: node.id, kind: node.kind, url: node.url, document: node.document}, bubbles: true}));
  }
  pick(e, double = false) {
    const node = this.hit(e);
    if (node) { if (double) this.open(node); else this.select(node); }
  }
  // The node under a pointer: what the ray through it meets first, or else the
  // nearest node within a fingertip (about 20 px) of it. A message is a dot a
  // few pixels wide, so a tap that lands just beside it still counts.
  hit(e, reach = 20) {
    const rect = this.renderer.domElement.getBoundingClientRect();
    if (!rect.width || !rect.height) return null;
    this.pointer.set(((e.clientX - rect.left) / rect.width) * 2 - 1, -((e.clientY - rect.top) / rect.height) * 2 + 1);
    this.raycaster.setFromCamera(this.pointer, this.camera);
    const hit = this.raycaster.intersectObjects(this.nodes.map(n => n.mesh))[0];
    if (hit) { const node = this.nodes.find(n => n.mesh === hit.object); if (node) return node; }
    let best = null, bestDistance = reach;
    for (const n of this.nodes) {
      const v = n.p.clone().applyMatrix4(this.group.matrixWorld).project(this.camera);
      if (v.z >= 1) continue;
      const d = Math.hypot(rect.left + ((v.x + 1) / 2) * rect.width - e.clientX, rect.top + ((1 - v.y) / 2) * rect.height - e.clientY);
      if (d < bestDistance) { best = n; bestDistance = d; }
    }
    return best;
  }
  // Distance between the two touch points while pinching.
  span() { const [a, b] = [...this.pointers.values()]; return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0; }
  // Camera distance, bounded; the toolbar's zoom buttons, the wheel and a pinch share it.
  zoomBy(delta) { this.distance = Math.max(10, Math.min(60, this.distance + delta)); this.schedule(); }
  zoomIn() { this.zoomBy(-6); }
  zoomOut() { this.zoomBy(6); }
  resize() {
    const w = Math.max(1, this.clientWidth), h = Math.max(1, this.clientHeight);
    this.renderer.setSize(w, h, false); this.camera.aspect = w / h; this.camera.updateProjectionMatrix(); this.schedule();
  }
  schedule() { if (!this.renderer || this.frame) return; this.frame = requestAnimationFrame(() => { this.frame = 0; this.draw(); }); }
  draw() {
    if (!this.renderer || !this.visible || document.hidden) return;
    if (this.steps < 240) this.layout(4);
    const selected = this.dataset.selected || '';
    for (const n of this.nodes) {
      n.mesh.position.copy(n.p);
      const active = n.id === selected;
      // The node the reader is on stays marked: larger, violet, and lit from within.
      if (n.sprite) {
        n.mesh.scale.copy(n.base).multiplyScalar(active ? 1.5 : 1);
        n.mesh.material.color.set(active ? 0xffffff : 0xdddddd);
      } else {
        n.mesh.scale.setScalar(active ? 1.7 : 1);
        n.mesh.material.color.set(active ? ACTIVE : colorOf(n.kind));
        n.mesh.material.emissive.set(active ? ACTIVE : 0x000000); n.mesh.material.emissiveIntensity = active ? .45 : 0;
      }
    }
    for (const e of this.edges) { const pos = e.line.geometry.attributes.position; pos.setXYZ(0, ...this.nodes[e.a].p.toArray()); pos.setXYZ(1, ...this.nodes[e.b].p.toArray()); pos.needsUpdate = true; }
    if (!this.drag && !this.hover && !this.motion.matches) this.rotation.y += .0015;
    this.group.rotation.set(this.rotation.x, this.rotation.y, 0);
    this.camera.position.set(0, 0, this.distance); this.camera.lookAt(0, 0, 0);
    this.renderer.render(this.scene, this.camera);
    const rect = this.renderer.domElement.getBoundingClientRect();
    for (const n of this.nodes) {
      if (!n.label) continue;
      const v = n.p.clone().applyMatrix4(this.group.matrixWorld).project(this.camera);
      // Tags are always named; a message shows its title when it is the one selected.
      const shown = v.z < 1 && (n.kind === 'tag' || n.id === selected);
      n.label.style.transform = `translate(${((v.x + 1) / 2) * rect.width}px, ${((1 - v.y) / 2) * rect.height}px)`;
      // Hidden labels leave the tab order too, so the keyboard never walks hundreds of invisible buttons.
      n.label.style.opacity = shown ? '1' : '0'; n.label.hidden = !shown;
      n.label.classList.toggle('is-selected', n.id === selected);
    }
    if (this.steps < 240 || this.drag || !this.motion.matches) this.schedule();
  }
  cleanup() {
    cancelAnimationFrame(this.frame); this.frame = 0;
    this.resizeObserver?.disconnect(); this.intersection?.disconnect();
    removeEventListener('pointermove', this.onMove); removeEventListener('pointerup', this.onUp); removeEventListener('pointercancel', this.onCancel);
    this.renderer?.domElement.removeEventListener('pointerenter', this.onEnter); this.renderer?.domElement.removeEventListener('pointerleave', this.onLeave);
    for (const r of this.resources) r.dispose?.(); this.resources = [];
    this.renderer?.dispose(); this.renderer = null;
  }
  disconnectedCallback() { this.cleanup(); }
}
if (!customElements.get('wiki-graph')) customElements.define('wiki-graph', WikiGraph);
