// Draws Clutch's built-in alert animations: an emblem per kind that pops in with a spark burst
// and a shockwave, then a banner for the alert's text (the alerts page writes the text over it).
// Frames are SVG, rasterised with sharp and encoded to VP9 WebM with alpha by FFmpeg.
// usage: node make-animations.mjs <out dir>   (needs `sharp` resolvable, e.g. NODE_PATH, and ffmpeg)
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const sharp = createRequire(join(process.cwd(), "x.js"))("sharp");
const OUT = process.argv[2] || ".";
const W = 960, H = 540, FPS = 30;

// Emblem and banner placement; the text box in alert.json is the banner's inside.
const EM = { x: 480, y: 168, r: 76 };
const BANNER = { x: 100, y: 292, w: 760, h: 112, rx: 24 };
const TEXT_BOX = {
  x: round((BANNER.x + 36) / W), y: round((BANNER.y + 8) / H),
  w: round((BANNER.w - 72) / W), h: round((BANNER.h - 16) / H),
};

// Icons in a 100×100 box around (50, 50).
const star = (cx, cy, ro, ri) => {
  const pts = [];
  for (let i = 0; i < 10; i++) {
    const a = -Math.PI / 2 + (i * Math.PI) / 5;
    const r = i % 2 ? ri : ro;
    pts.push(`${(cx + r * Math.cos(a)).toFixed(2)},${(cy + r * Math.sin(a)).toFixed(2)}`);
  }
  return `<polygon points="${pts.join(" ")}" fill="url(#silver)"/>`;
};
const ICONS = {
  follow: `<path fill="url(#silver)" d="M50 84 C22 64 11 48 11 34 C11 21 20 13 31 13 C40 13 46 18 50 25 C54 18 60 13 69 13 C80 13 89 21 89 34 C89 48 78 64 50 84 Z"/>`,
  sub: star(50, 53, 42, 18),
  resub: `${star(50, 52, 27, 12)}
    <path d="M24 70 A33 33 0 1 1 76 70" fill="none" stroke="url(#silver)" stroke-width="8" stroke-linecap="round"/>
    <polygon points="68,74 84,64 84,82" fill="url(#silver)"/>`,
  gift: `<rect x="17" y="44" width="66" height="40" rx="5" fill="url(#silver)"/>
    <rect x="12" y="32" width="76" height="16" rx="4" fill="url(#silver)"/>
    <rect x="45" y="32" width="10" height="52" fill="ACCENT"/>
    <path d="M50 32 C40 14 22 16 28 26 C31 31 42 32 50 32 C58 32 69 31 72 26 C78 16 60 14 50 32 Z" fill="none" stroke="url(#silver)" stroke-width="6" stroke-linejoin="round"/>`,
  bits: `<polygon points="50,12 82,38 50,90 18,38" fill="url(#silver)"/>
    <polygon points="34,38 50,12 66,38" fill="#ffffff" opacity="0.55"/>
    <polygon points="18,38 82,38 50,90" fill="ACCENT" opacity="0.35"/>`,
  raid: `<polygon points="58,8 22,56 46,56 40,92 78,40 54,40" fill="url(#silver)"/>`,
  paid: `<circle cx="50" cy="50" r="38" fill="none" stroke="url(#silver)" stroke-width="7"/>
    <path d="M62 37 C59 32 55 30 49 30 C42 30 37 34 37 40 C37 52 63 47 63 60 C63 66 57 70 50 70 C43 70 38 67 36 62" fill="none" stroke="url(#silver)" stroke-width="7" stroke-linecap="round"/>
    <path d="M50 22 V78" stroke="url(#silver)" stroke-width="7" stroke-linecap="round"/>`,
};

// Per kind: accent colour, length, how big the burst is, and whether it uses confetti.
const KINDS = {
  follow: { accent: "#5AA9FF", secs: 4.0, sparks: 18, rings: 1 },
  sub: { accent: "#A27BFF", secs: 5.0, sparks: 26, rings: 1 },
  resub: { accent: "#D17BFF", secs: 5.0, sparks: 26, rings: 2 },
  gift: { accent: "#39D9A8", secs: 5.0, sparks: 34, rings: 1, confetti: true },
  bits: { accent: "#FFC247", secs: 4.5, sparks: 28, rings: 1 },
  raid: { accent: "#FF6A4D", secs: 6.0, sparks: 40, rings: 3 },
  paid: { accent: "#4CE08A", secs: 5.0, sparks: 26, rings: 1 },
};
const CONFETTI = ["#FF6A4D", "#FFC247", "#39D9A8", "#5AA9FF", "#A27BFF", "#FFFFFF"];

function round(v) { return Math.round(v * 1000) / 1000; }
const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
const span = (t, a, b) => clamp((t - a) / (b - a));
const easeOut = (x) => 1 - Math.pow(1 - x, 3);
const easeIn = (x) => x * x * x;
const backOut = (x) => { const c = 1.9; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); };

// A small seeded random, so every build draws the same frames.
function rng(seed) {
  return () => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 4294967296;
  };
}

function particles(kind, k) {
  const rand = rng([...kind].reduce((a, c) => a * 31 + c.charCodeAt(0), 7));
  return Array.from({ length: k.sparks }, (_, i) => ({
    angle: (i / k.sparks) * Math.PI * 2 + (rand() - 0.5) * 0.5,
    dist: 120 + rand() * 230,
    size: 3 + rand() * 5,
    life: 0.9 + rand() * 0.7,
    delay: rand() * 0.12,
    spin: (rand() - 0.5) * 8,
    color: k.confetti ? CONFETTI[i % CONFETTI.length] : rand() < 0.35 ? "#FFFFFF" : k.accent,
  }));
}

function frame(kind, k, sparks, t) {
  const end = k.secs;
  const exit = easeIn(span(t, end - 0.5, end));
  const pop = span(t, 0.05, 0.55);
  const emScale = backOut(pop) * (1 - exit);
  const float = Math.sin(t * 2.2) * 3 * span(t, 0.6, 1.2);
  const emY = EM.y + float;
  const glow = 0.55 + 0.25 * Math.sin(t * 3.1);
  const bannerOpen = easeOut(span(t, 0.3, 0.75)) * (1 - easeIn(span(t, end - 0.55, end - 0.1)));
  const bw = BANNER.w * bannerOpen;
  const bx = W / 2 - bw / 2;
  const parts = [];

  // Shockwave rings from the emblem.
  for (let i = 0; i < k.rings; i++) {
    const p = span(t, 0.18 + i * 0.16, 0.95 + i * 0.16);
    if (p > 0 && p < 1) {
      const r = EM.r + easeOut(p) * (170 + i * 40);
      parts.push(`<circle cx="${EM.x}" cy="${emY}" r="${r.toFixed(1)}" fill="none" stroke="${k.accent}" stroke-width="${(5 * (1 - p) + 1).toFixed(2)}" opacity="${(0.75 * (1 - p)).toFixed(3)}"/>`);
    }
  }

  // Banner: dark glass with an accent edge, a soft glow and one light sweep.
  if (bw > 2) {
    const op = clamp(bannerOpen * 1.4);
    parts.push(`<g opacity="${op.toFixed(3)}">
      <rect x="${bx}" y="${BANNER.y}" width="${bw}" height="${BANNER.h}" rx="${BANNER.rx}" fill="${k.accent}" opacity="${(0.35 * glow).toFixed(3)}" filter="url(#soft)"/>
      <rect x="${bx}" y="${BANNER.y}" width="${bw}" height="${BANNER.h}" rx="${BANNER.rx}" fill="url(#glass)"/>
      <rect x="${bx + 1.5}" y="${BANNER.y + 1.5}" width="${Math.max(0, bw - 3)}" height="${BANNER.h - 3}" rx="${BANNER.rx - 1.5}" fill="none" stroke="url(#edge)" stroke-width="3"/>
    </g>`);
    const sweep = span(t, 0.85, 1.6);
    if (sweep > 0 && sweep < 1) {
      const sx = bx - 160 + sweep * (bw + 320);
      parts.push(`<g clip-path="url(#bannerclip)"><polygon points="${sx},${BANNER.y} ${sx + 70},${BANNER.y} ${sx + 30},${BANNER.y + BANNER.h} ${sx - 40},${BANNER.y + BANNER.h}" fill="#ffffff" opacity="0.16"/></g>`);
    }
  }

  // Sparks (or confetti) bursting from the emblem.
  for (const s of sparks) {
    const p = span(t, 0.2 + s.delay, 0.2 + s.delay + s.life);
    if (p <= 0 || p >= 1) continue;
    const d = easeOut(p) * s.dist;
    const x = EM.x + Math.cos(s.angle) * d;
    const y = emY + Math.sin(s.angle) * d + (k.confetti ? p * p * 90 : 0);
    const o = (1 - p) * (1 - exit);
    if (k.confetti) {
      parts.push(`<rect x="${(x - s.size).toFixed(1)}" y="${(y - s.size / 2).toFixed(1)}" width="${(s.size * 2).toFixed(1)}" height="${s.size.toFixed(1)}" fill="${s.color}" opacity="${o.toFixed(3)}" transform="rotate(${(s.spin * p * 90).toFixed(1)} ${x.toFixed(1)} ${y.toFixed(1)})"/>`);
    } else {
      const r = s.size * (1 - p * 0.6);
      parts.push(`<circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="${(r * 2.2).toFixed(2)}" fill="${s.color}" opacity="${(o * 0.25).toFixed(3)}"/>
        <circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="${r.toFixed(2)}" fill="${s.color}" opacity="${o.toFixed(3)}"/>`);
    }
  }

  // The emblem: glow, slate badge with an accent ring, the kind's icon.
  if (emScale > 0.01) {
    const icon = ICONS[kind].replaceAll("ACCENT", k.accent);
    parts.push(`<g transform="translate(${EM.x} ${emY.toFixed(2)}) scale(${emScale.toFixed(4)})">
      <circle r="${EM.r + 18}" fill="${k.accent}" opacity="${(0.45 * glow).toFixed(3)}" filter="url(#glow)"/>
      <circle r="${EM.r}" fill="url(#badge)"/>
      <circle r="${EM.r - 4}" fill="none" stroke="${k.accent}" stroke-width="7"/>
      <circle r="${EM.r - 11}" fill="none" stroke="#ffffff" stroke-opacity="0.12" stroke-width="2"/>
      <g transform="translate(-50 -50)">${icon}</g>
    </g>`);
  }

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
  <defs>
    <radialGradient id="badge" cx="0.7" cy="0.25" r="0.95">
      <stop offset="0" stop-color="#5B7590"/><stop offset="0.5" stop-color="#2A3644"/><stop offset="1" stop-color="#161D25"/>
    </radialGradient>
    <linearGradient id="silver" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="100" y2="100">
      <stop offset="0" stop-color="#FFFFFF"/><stop offset="0.45" stop-color="#DCE3EA"/><stop offset="1" stop-color="#8FA1B3"/>
    </linearGradient>
    <linearGradient id="glass" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#2A3644" stop-opacity="0.94"/><stop offset="1" stop-color="#161D25" stop-opacity="0.94"/>
    </linearGradient>
    <linearGradient id="edge" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="${k.accent}" stop-opacity="0.2"/><stop offset="0.5" stop-color="${k.accent}"/><stop offset="1" stop-color="${k.accent}" stop-opacity="0.2"/>
    </linearGradient>
    <clipPath id="bannerclip"><rect x="${bx}" y="${BANNER.y}" width="${Math.max(0, bw)}" height="${BANNER.h}" rx="${BANNER.rx}"/></clipPath>
    <filter id="glow" x="-1" y="-1" width="3" height="3"><feGaussianBlur stdDeviation="16"/></filter>
    <filter id="soft" x="-0.2" y="-0.8" width="1.4" height="2.6"><feGaussianBlur stdDeviation="14"/></filter>
  </defs>
  ${parts.join("\n  ")}
</svg>`;
}

async function encode(kind, k) {
  const dir = join(OUT, kind);
  mkdirSync(dir, { recursive: true });
  const ff = spawn("ffmpeg", [
    "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", `${W}x${H}`, "-r", String(FPS), "-i", "-",
    "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", "36", "-row-mt", "1",
    "-deadline", "good", "-cpu-used", "2", "-auto-alt-ref", "0", "-an", join(dir, "alert.webm"),
  ], { stdio: ["pipe", "inherit", "inherit"] });
  const done = new Promise((ok, fail) => ff.on("close", (c) => (c ? fail(new Error(`ffmpeg ${c}`)) : ok())));
  const sparks = particles(kind, k);
  const frames = Math.round(k.secs * FPS);
  for (let i = 0; i < frames; i++) {
    const raw = await sharp(Buffer.from(frame(kind, k, sparks, i / FPS))).ensureAlpha().raw().toBuffer();
    if (!ff.stdin.write(raw)) await new Promise((r) => ff.stdin.once("drain", r));
  }
  ff.stdin.end();
  await done;
  writeFileSync(join(dir, "alert.json"), JSON.stringify({
    text_box: TEXT_BOX,
    text_in: 0.75,
    text_out: round(k.secs - 0.75),
    duration: k.secs,
  }, null, 2) + "\n");
  console.log(`${kind.padEnd(7)} ${frames} frames`);
}

for (const [kind, k] of Object.entries(KINDS)) await encode(kind, k);
