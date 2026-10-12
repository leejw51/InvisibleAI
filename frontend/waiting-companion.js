import { animate } from 'motion';

// Enhance existing, truthful waiting indicators; never invent progress or delays.
const selector = '.spinner, .chat-waiting, .coding-stage, .cloud-now.is-running, .chat-upload-progress';
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const buddies = new Map();
function update(item) {
  item.control?.stop(); item.control = null;
  item.image.style.removeProperty('transform');
  if (!reduced.matches && !document.hidden && item.visible) {
    item.control = animate(item.image, {
      y:[0,-5,0,0,-3,0], rotate:[0,-8,8,0,0,0], scale:[1,1.04,.98,1,1.03,1]
    }, {duration:4.5,repeat:Infinity,ease:'easeInOut'});
  }
}
const visibility = new IntersectionObserver(entries => {
  for (const entry of entries) {
    const item = buddies.get(entry.target);
    if (item) { item.visible = entry.isIntersecting; update(item); }
  }
});
let scheduled;
function reconcile() {
  scheduled = null;
  for (const [element,item] of buddies) {
    if (!element.isConnected || !element.matches(selector)) {
      item.control?.stop(); visibility.unobserve(element);
      item.image.remove(); element.removeAttribute('data-buddy-wait'); buddies.delete(element);
    }
  }
  const companion=document.querySelector('invisible-companion[data-page-companion]');
  if (companion) {
    const pending=[...document.querySelectorAll('.spinner, .cloud-now.is-thinking, .coding-progress, .wiki-refresh.is-busy, .chat-upload-progress, [aria-busy="true"]')].some(el=>el.getClientRects().length && getComputedStyle(el).visibility !== 'hidden');
    const watching=!!document.querySelector('.cloud-session-state.is-active');
    const state=pending ? 'thinking' : watching ? 'listening' : '';
    if ((companion.dataset.activity || '') !== state) {
      if (state) companion.dataset.activity=state; else companion.removeAttribute('data-activity');
    }
  }
  for (const element of document.querySelectorAll(selector)) {
    if (buddies.has(element) || element.parentElement.closest(selector)) continue;
    const image = new Image(); image.src='/assets/everyday-companion.png';
    image.alt=''; image.className='wait-buddy'; image.setAttribute('aria-hidden','true');
    element.append(image); element.setAttribute('data-buddy-wait','');
    const item={image,visible:false}; buddies.set(element,item); visibility.observe(element);
  }
}
new MutationObserver(() => { if (!scheduled) scheduled=requestAnimationFrame(reconcile); })
  .observe(document.body,{subtree:true,childList:true,attributes:true,attributeFilter:['class','aria-busy']});
reduced.addEventListener('change',()=>buddies.forEach(update));
document.addEventListener('visibilitychange',()=>buddies.forEach(update));
reconcile();
