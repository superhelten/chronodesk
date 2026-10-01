'use strict';
(() => {
const d = document, R = d.documentElement, $ = id => d.getElementById(id);
R.className = R.className.replace(/\bno-js\b/, 'js');
const reduce = matchMedia('(prefers-reduced-motion: reduce)');

// Skin picker; sizes are natural / 2
const DIM = {sans: [331, 168], digital: [375, 145], matrix: [420, 145]};
const FACE = {sans: ['Typeface', 'typeface digits'], digital: ['Seven-segment', 'seven-segment digits'],
  matrix: ['Dot matrix', 'dot-matrix digits']};
const COL = {default: ['Default', 'Default white'], warm: ['Warm', 'Warm'], cool: ['Cool', 'Cool blue'],
  amber: ['Amber', 'Amber'], green: ['Green', 'Green'], red: ['Red', 'Red'], yellow: ['Yellow', 'Yellow'],
  studio: ['Studio', 'Studio colours, green time and red ring']};

const form = $('picker'), stage = $('stage');
if (form && stage) {
  let front = stage.querySelector('img'), back = null, seq = 0;
  const seen = {};
  const pick = n => (form.querySelector(`input[name="${n}"]:checked`) || {}).value;
  const url = (f, c) => `img/skin-${f}-${c}.webp`;
  const hide = img => { img.alt = ''; img.setAttribute('aria-hidden', 'true'); };
  form.addEventListener('submit', e => e.preventDefault());
  form.addEventListener('change', async () => {
    const f = pick('face'), c = pick('colour'), s = COL[c] && DIM[f], n = ++seq;
    if (!s) return;
    $('skin-live').textContent = `${FACE[f][0]}, ${COL[c][0]}`;
    if (!back) {
      back = d.createElement('img'); hide(back); back.decoding = 'async';
      stage.insertBefore(back, front.nextSibling);
    }
    const img = back;
    img.className = 'back wait';
    [img.width, img.height] = s;
    img.src = url(f, c);
    try { await img.decode(); } catch (e) { return; }
    if (n !== seq) return;
    img.alt = `ChronoDesk clock, ${FACE[f][1]}, ${COL[c][1]}: 10:09:37 and TUE 22 SEP, framed by the sixty-LED seconds ring`;
    img.removeAttribute('aria-hidden');
    hide(front);
    img.className = 'front'; front.className = 'back';
    [back, front] = [front, img];
  });
  const preload = i => {
    const f = i.name === 'face' ? i.value : pick('face'), c = i.name === 'colour' ? i.value : pick('colour');
    const u = url(f, c);
    if (!seen[u] && COL[c] && DIM[f]) { seen[u] = 1; new Image().src = u; }
  };
  form.querySelectorAll('input').forEach(i => {
    i.addEventListener('focus', () => preload(i));
    const l = form.querySelector(`label[for="${i.id}"]`);
    if (l) l.addEventListener('pointerenter', () => preload(i));
  });
}

// Hero: animated ring after load
const hero = $('hero-img'), tpl = $('pause-tpl'), STILL = 'img/hero-ring.webp', ANIM = 'img/hero-ring-anim.webp';
let btn = null;
if (hero && tpl) {
  addEventListener('load', () => {
    const cn = navigator.connection;
    if (reduce.matches || (cn && cn.saveData)) return;
    (window.requestIdleCallback || (f => setTimeout(f, 200)))(async () => {
      const a = new Image(); a.src = ANIM;
      try { await a.decode(); } catch (e) { return; }
      if (reduce.matches) return;
      hero.src = a.src;
      btn = tpl.content.firstElementChild.cloneNode(true);
      btn.addEventListener('click', () => {
        const p = btn.getAttribute('aria-pressed') === 'true';
        hero.src = p ? ANIM : STILL;
        btn.setAttribute('aria-pressed', String(!p));
      });
      hero.parentNode.appendChild(btn);
    });
  });
  if (reduce.addEventListener) reduce.addEventListener('change', e => {
    if (!e.matches) return;
    hero.src = STILL;
    if (btn) { btn.remove(); btn = null; }
  });
}

// Star count from 10 up; silent on failure
(async () => {
  try {
    let n, s = null;
    try { s = JSON.parse(sessionStorage.getItem('cd-stars')); } catch (e) {}
    if (s && Date.now() - s.t < 600000) n = s.n;
    else {
      const r = await fetch('https://api.github.com/repos/superhelten/chronodesk', {headers: {Accept: 'application/vnd.github+json'}});
      if (!r.ok) return;
      n = (await r.json()).stargazers_count;
      try { sessionStorage.setItem('cd-stars', JSON.stringify({n, t: Date.now()})); } catch (e) {}
    }
    if (typeof n !== 'number' || n < 10) return;
    const t = n.toLocaleString('en');
    d.querySelectorAll('a.star').forEach(a => {
      const c = d.createElement('span'); c.className = 'count'; c.textContent = t;
      a.appendChild(c);
      a.setAttribute('aria-label', `Star ChronoDesk on GitHub, ${t} stars`);
    });
  } catch (e) {}
})();

// Copy button
const cp = $('copy'), code = $('cmd'), st = $('copy-status');
let tm = 0;
if (cp && code && st) cp.addEventListener('click', () => {
  const ok = () => {
    cp.textContent = st.textContent = 'Copied';
    clearTimeout(tm);
    tm = setTimeout(() => { cp.textContent = 'Copy'; st.textContent = ''; }, 1500);
  };
  const no = () => {
    try { const r = d.createRange(), s = getSelection(); r.selectNodeContents(code); s.removeAllRanges(); s.addRange(r); } catch (e) {}
    st.textContent = 'Press Ctrl+C to copy';
  };
  const cb = navigator.clipboard;
  if (cb && cb.writeText) cb.writeText(code.textContent).then(ok, no); else no();
});
})();
