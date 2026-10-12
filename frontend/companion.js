import * as THREE from 'three';
import { animate } from 'motion';

// The original artwork stays in accessible HTML; Three renders its little world.
// Neither engine reads prompts, messages, or any other private content.
class InvisibleCompanion extends HTMLElement {
  static observedAttributes = ['data-state', 'data-activity'];
  connectedCallback() {
    this.startFrame = requestAnimationFrame(() => this.mount());
  }
  mount() {
    if (!this.isConnected || this.abort) return;
    this.abort = new AbortController();
    this.motion = matchMedia('(prefers-reduced-motion: reduce)');
    this.visible = true;
    this.lost = false;
    this.controls = [];
    this.resources = [];
    this.figure = this.querySelector('.companion-figure');
    this.button = this.querySelector('button');
    const options = { signal: this.abort.signal };
    this.button.addEventListener('click', () => this.greet(), options);
    this.addEventListener('pointermove', e => {
      if (this.motion.matches || e.pointerType === 'touch') return;
      const r = this.getBoundingClientRect();
      this.targetX = (e.clientX - r.left) / r.width - .5;
      this.targetY = (e.clientY - r.top) / r.height - .5;
    }, options);
    this.addEventListener('pointerleave', () => { this.targetX = this.targetY = 0; }, options);
    this.targetX = this.targetY = 0;
    this.motion.addEventListener('change', () => this.refresh(), options);
    document.addEventListener('visibilitychange', () => this.refresh(), options);
    this.intersection = new IntersectionObserver(entries => {
      this.visible = entries[0].isIntersecting;
      this.refresh();
    });
    this.intersection.observe(this);
    try {
      this.renderer = new THREE.WebGLRenderer({ alpha:true, antialias:true, powerPreference:'low-power' });
      this.renderer.setPixelRatio(Math.min(devicePixelRatio, 1.5));
      this.renderer.setClearColor(0,0);
      this.renderer.domElement.setAttribute('aria-hidden','true');
      this.querySelector('.companion-world').append(this.renderer.domElement);
      this.renderer.domElement.addEventListener('webglcontextlost', e => {
        e.preventDefault(); this.lost = true; this.refresh(); this.dataset.render = 'fallback';
      }, options);
      this.renderer.domElement.addEventListener('webglcontextrestored', () => {
        this.lost = false; this.dataset.render = 'webgl'; this.refresh();
      }, options);
      this.scene = new THREE.Scene();
      this.camera = new THREE.PerspectiveCamera(38,1,.1,30);
      this.camera.position.set(0,0,7);
      this.scene.add(new THREE.AmbientLight(0xe4d4ff,2.5));
      const light = new THREE.DirectionalLight(0xffffff,4);
      light.position.set(-3,5,6); this.scene.add(light);
      this.world = new THREE.Group(); this.scene.add(this.world);
      const add = (geometry, color, opacity=1) => {
        const material = new THREE.MeshPhysicalMaterial({color,metalness:.28,roughness:.25,transparent:true,opacity});
        const mesh = new THREE.Mesh(geometry,material);
        this.resources.push(geometry,material); this.world.add(mesh); return mesh;
      };
      const colors = {wiki:0x91c9bd,autonomous:0xd2ac70,coding:0x83b9e4,code:0x83b9e4,skills:0x83b9e4,images:0xe8a3c7,videos:0xe8a3c7,devices:0x91c9bd};
      this.ring = add(new THREE.TorusGeometry(1.52,.014,8,96),colors[this.dataset.scene] || 0xbba0ff,.7);
      this.ring.rotation.set(1.08,.2,-.25);
      this.ring.position.y = -.42;
      this.satellites = Array.from({length:9}, (_,i) => {
        const mesh = add(i%3===0 ? new THREE.OctahedronGeometry(.07) : new THREE.SphereGeometry(.025+(i%3)*.016,12,8), i%2 ? 0xe9b9fa : 0xa2d9da,.85);
        mesh.userData.phase = i * Math.PI*2/9;
        return mesh;
      });
      this.resize = new ResizeObserver(() => this.fit());
      this.resize.observe(this);
      this.dataset.render = 'webgl';
      this.fit();
    } catch {
      this.releaseGPU();
      this.dataset.render = 'fallback';
    }
    this.refresh();
  }
  attributeChangedCallback() { if (this.abort) this.refresh(); }
  get state() { return this.dataset.activity || this.dataset.state; }
  get active() { return this.visible && !document.hidden && !this.motion.matches; }
  stopMotion() {
    for (const control of this.controls) control.stop();
    this.controls = [];
    this.figure?.style.removeProperty('transform');
  }
  refresh() {
    this.stopMotion();
    cancelAnimationFrame(this.frame);
    if (this.greetingTimer) { clearTimeout(this.greetingTimer); this.greetingTimer = null; }
    this.removeAttribute('data-greeting');
    if (this.active) {
      const thinking = this.state === 'thinking';
      const listening = this.state === 'listening';
      this.controls.push(animate(this.figure, {
        y:[0, thinking ? -5 : -9,0], rotate:thinking ? [-3,3,-3] : [0,listening ? -3 : 1.5,0],
        scale:listening ? [1,1.035,1] : [1,1.015,1]
      }, {duration:thinking ? 1.7 : 4.8,repeat:Infinity,ease:'easeInOut'}));
    }
    this.render(performance.now());
  }
  greet() {
    if (!this.active) return;
    this.stopMotion();
    clearTimeout(this.greetingTimer);
    this.dataset.greeting = 'true';
    this.controls.push(animate(this.figure, { y:[0,-24,0,-10,0], rotate:[0,-12,9,-5,0], scale:[1,1.08,.96,1.03,1] }, {duration:1,ease:'easeInOut'}));
    this.greetingTimer = setTimeout(() => this.refresh(),1100);
  }
  fit() {
    if (!this.renderer) return;
    const {width,height} = this.getBoundingClientRect();
    if (!width || !height) return;
    this.renderer.setSize(width,height,false);
    this.camera.aspect = width/height;
    this.camera.updateProjectionMatrix();
    if (!this.active) this.draw(0);
  }
  draw(time) {
    if (!this.renderer || this.lost) return;
    const t = time*.00025*(this.state==='thinking' ? 2.5 : 1);
    this.world.rotation.y += ((this.targetX || 0)*.25-this.world.rotation.y)*.06;
    this.world.rotation.x += ((this.targetY || 0)*.12-this.world.rotation.x)*.06;
    this.satellites.forEach((mesh,i) => {
      const a = mesh.userData.phase+t;
      mesh.position.set(Math.cos(a)*(1.25+i%2*.25), Math.sin(a)*.85, Math.sin(a)*.35);
      mesh.rotation.set(t+i,t,0);
    });
    this.ring.rotation.z = -.25+Math.sin(t)*.12;
    this.renderer.render(this.scene,this.camera);
  }
  render(time) {
    this.draw(this.motion.matches ? 0 : time);
    if (this.active && this.renderer && !this.lost) this.frame = requestAnimationFrame(t => this.render(t));
  }
  releaseGPU() {
    this.resize?.disconnect();
    for (const resource of this.resources || []) resource.dispose();
    this.resources = [];
    this.renderer?.dispose();
    this.renderer?.forceContextLoss();
    this.renderer?.domElement.remove();
    this.renderer = null;
  }
  disconnectedCallback() {
    cancelAnimationFrame(this.startFrame); cancelAnimationFrame(this.frame);
    clearTimeout(this.greetingTimer);
    this.stopMotion();
    this.abort?.abort(); this.abort = null;
    this.intersection?.disconnect();
    this.releaseGPU();
  }
}
customElements.define('invisible-companion', InvisibleCompanion);
