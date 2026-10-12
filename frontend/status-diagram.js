import * as THREE from 'three';

// Coordinates are shared with the accessible HTML labels. No private content enters WebGL.
const anchors = [[.19,.25],[.19,.73],[.82,.19],[.86,.5],[.82,.81]];
const exponentialInOut = t => t === 0 || t === 1 ? t : t < .5 ? 2 ** (20*t-10)/2 : (2-2**(-20*t+10))/2;
class WorkspaceDiagram extends HTMLElement {
  static observedAttributes = ['data-state'];
  connectedCallback() {
    if (this.renderer) return;
    this.motion = matchMedia('(prefers-reduced-motion: reduce)');
    this.visible = true;
    this.resources = [];
    try {
      this.renderer = new THREE.WebGLRenderer({alpha:true, antialias:true, powerPreference:'low-power'});
      this.renderer.setPixelRatio(Math.min(devicePixelRatio,2));
      this.renderer.setClearColor(0,0);
      this.renderer.domElement.setAttribute('aria-hidden','true');
      this.append(this.renderer.domElement);
      this.scene = new THREE.Scene();
      this.camera = new THREE.OrthographicCamera(-6,6,4,-4,.1,100);
      this.camera.position.set(0,0,20);
      this.scene.add(new THREE.AmbientLight(0xffffff,2.1));
      const light = new THREE.DirectionalLight(0xffffff,3); light.position.set(-4,6,8); this.scene.add(light);
      this.group = new THREE.Group(); this.scene.add(this.group);
      const material = new THREE.MeshPhysicalMaterial({color:0xab91dd,metalness:.16,roughness:.32,transparent:true,opacity:.64});
      this.core = new THREE.Mesh(this.track(new THREE.IcosahedronGeometry(.6,1)),this.track(material));
      this.group.add(this.core);
      this.rings = [];
      for (let i=0;i<3;i++) {
        const ring = new THREE.Mesh(this.track(new THREE.TorusGeometry(.86+i*.12,.009,8,100)),this.track(new THREE.MeshBasicMaterial({color:0xb7a2d3,transparent:true,opacity:.5-i*.09})));
        ring.rotation.set(.3+i*.4,.15+i*.5,i*.8); this.group.add(ring); this.rings.push(ring);
      }
      this.points=[];this.paths=[];this.nodes=[];
      const dotGeometry=this.track(new THREE.SphereGeometry(.035,8,8));
      anchors.forEach((_,i)=>{
        const line=new THREE.Line(this.track(new THREE.BufferGeometry()),this.track(new THREE.LineBasicMaterial({color:0xc9bddb,transparent:true,opacity:.62})));
        this.scene.add(line);this.paths.push(line);
        const node=new THREE.Mesh(this.track(new THREE.TorusGeometry(.35,.012,8,60)),this.track(new THREE.MeshBasicMaterial({color:i<2?0x7aaea1:0xb29acd,transparent:true,opacity:.55})));
        this.scene.add(node);this.nodes.push(node);
        for(let n=0;n<3;n++){
          const dot=new THREE.Mesh(dotGeometry,this.track(new THREE.MeshBasicMaterial({color:i<2?0x86b6a7:0xa98ac7,transparent:true,opacity:.8})));
          this.scene.add(dot);this.points.push({mesh:dot,index:i,offset:n/3});
        }
      });
      this.resizeObserver=new ResizeObserver(()=>this.resize());this.resizeObserver.observe(this);
      this.themeObserver=new MutationObserver(()=>this.theme());this.themeObserver.observe(document.documentElement,{attributes:true,attributeFilter:['data-theme']});
      this.intersection=new IntersectionObserver(([entry])=>{this.visible=entry.isIntersecting;this.schedule();});this.intersection.observe(this);
      this.onVisibility=()=>this.schedule();document.addEventListener('visibilitychange',this.onVisibility);
      this.onMotion=()=>this.schedule();this.motion.addEventListener('change',this.onMotion);
      this.addEventListener('webglcontextlost',this.contextLost=event=>{event.preventDefault();cancelAnimationFrame(this.frame);this.setAttribute('data-renderer','unavailable');},true);
      this.addEventListener('webglcontextrestored',this.contextRestored=()=>{this.setAttribute('data-renderer','three');this.schedule();},true);
      this.theme();this.resize();this.update();this.setAttribute('data-renderer','three');
    } catch {
      this.setAttribute('data-renderer','unavailable');
      this.cleanup();
    }
  }
  track(resource){this.resources.push(resource);return resource;}
  attributeChangedCallback(){this.update();}
  update(){
    try{this.state=JSON.parse(this.getAttribute('data-state')||'{}');}catch{this.state={};}
    this.tween={from:this.scale??1,to:1+Math.min(Math.log2(1+(this.state.counts||[]).reduce((sum,v)=>sum+v,0))*.013,.16),start:performance.now()};
    if(this.renderer)this.schedule();
  }
  resize(){
    if(!this.renderer)return;
    const {width,height}=this.getBoundingClientRect();if(!width||!height)return;
    const span=12*height/width;this.camera.top=span/2;this.camera.bottom=-span/2;this.camera.updateProjectionMatrix();this.renderer.setSize(width,height,false);
    this.curves=anchors.map(([x,y],i)=>{
      const end=new THREE.Vector3((x-.5)*12,(.5-y)*span,0);
      this.nodes[i].position.copy(end);
      const curve=new THREE.CubicBezierCurve3(new THREE.Vector3(0,0,0),new THREE.Vector3(end.x*.35,end.y*.04,.45),new THREE.Vector3(end.x*.8,end.y*.85,-.4),end);
      this.paths[i].geometry.setFromPoints(curve.getPoints(64));return curve;
    });
    this.schedule();
  }
  theme(){
    const dark=document.documentElement.dataset.theme==='dark';
    this.paths?.forEach(p=>p.material.color.set(dark?0x635376:0xc9bddb));
    if(this.core)this.core.material.color.set(dark?0x9a7cc9:0xab91dd);
    this.schedule();
  }
  schedule(){
    cancelAnimationFrame(this.frame);
    if(this.renderer&&this.visible&&!document.hidden)this.frame=requestAnimationFrame(time=>this.render(time));
  }
  render(time){
    if(!this.renderer||!this.curves)return;
    const reduced=this.motion.matches;
    const progress=reduced?1:Math.min(1,(time-(this.tween?.start??time))/950);
    this.scale=THREE.MathUtils.lerp(this.tween?.from??1,this.tween?.to??1,exponentialInOut(progress));
    this.group.scale.setScalar(this.scale);
    const seconds=reduced?0:time/1000;
    this.core.rotation.set(.4+seconds*.045,.5+seconds*.075,.2);
    this.rings.forEach((ring,i)=>{ring.rotation.z=i*.8+seconds*(i%2?-.055:.04);});
    this.nodes.forEach((node,i)=>node.scale.setScalar(1+.035*(1-Math.cos(seconds*1.3+i))));
    this.points.forEach(({mesh,index,offset})=>{
      mesh.visible=!!this.state?.live && (this.state.counts?.[index]??0)>0;
      const t=reduced?offset:(seconds*.085+offset)%1;
      mesh.position.copy(this.curves[index].getPoint(t));
      mesh.material.opacity=.8*Math.sin(Math.PI*t);
    });
    this.renderer.render(this.scene,this.camera);
    if(!reduced&&this.visible&&!document.hidden)this.frame=requestAnimationFrame(t=>this.render(t));
  }
  cleanup(){
    cancelAnimationFrame(this.frame);this.resizeObserver?.disconnect();this.themeObserver?.disconnect();this.intersection?.disconnect();
    document.removeEventListener('visibilitychange',this.onVisibility);this.motion?.removeEventListener('change',this.onMotion);
    this.removeEventListener('webglcontextlost',this.contextLost,true);this.removeEventListener('webglcontextrestored',this.contextRestored,true);
    this.resources?.forEach(resource=>resource.dispose());this.renderer?.dispose();this.renderer?.domElement.remove();this.renderer=null;
  }
  disconnectedCallback(){this.cleanup();}
}
customElements.define('cwb-status-diagram',WorkspaceDiagram);
