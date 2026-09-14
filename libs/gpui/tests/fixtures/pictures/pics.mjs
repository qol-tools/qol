const n = (v) => +v.toFixed(2);
const svg = (inner) => `<svg class="art" viewBox="0 0 96 60" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;
const rect = (x, y, w, h, r = 3, more = '') => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}"${more}/>`;
const wash = (x, y, w, h, r = 3, o = 0.14) => rect(x, y, w, h, r, ` fill="currentColor" fill-opacity="${o}" stroke="none"`);
const line = (x1, y1, x2, y2, more = '') => `<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}"${more}/>`;
const path = (d, more = '') => `<path d="${d}"${more}/>`;
const circle = (cx, cy, r, more = '') => `<circle cx="${cx}" cy="${cy}" r="${r}"${more}/>`;
const dot = (cx, cy, r) => circle(cx, cy, r, ' fill="currentColor" stroke="none"');
const text = (x, y, t, size = 9, anchor = 'middle', family = 'IBM Plex Mono, monospace', weight = 500) =>
  `<text x="${x}" y="${y}" font-size="${size}" font-family="${family}" font-weight="${weight}" text-anchor="${anchor}" fill="currentColor" stroke="none">${t}</text>`;
const SOFT = ' stroke-opacity=".5"';
const FILL = ' fill="currentColor" stroke="none"';
const shrink = (inner, s, [fx, fy], [tx, ty] = [48, 28]) => `<g transform="translate(${tx} ${ty}) scale(${s}) translate(${-fx} ${-fy})" stroke-width="${n(1.5 / s)}">${inner}</g>`;

const strip = (sel, y) => [10, 38, 66].map((x, i) => wash(x, y, 20, 14, 2) + rect(x, y, 20, 14, 2, i === sel ? ' stroke-width="2.4"' : SOFT)).join('');
const key = (x, y, w, label, held) => (held ? wash(x, y, w, 15, 3, 0.35) : '') + rect(x, y, w, 15, 3) + text(x + w / 2, y + 10.5, label, 8);
const hopRight = path('M20 22 Q33 10 46 21') + path('M41.5 20.5 L46 21 L45 16.5');
const hopLeft = path('M76 22 Q63 10 50 21') + path('M54.5 20.5 L50 21 L51 16.5');
const pin =(x, y, angle) => `<g transform="translate(${x} ${y}) rotate(${angle}) translate(0 -15)" stroke-width="1.3">${path('M-3.45 0 H3.45 V1.38 H2.53 V5.29 Q2.53 7.59 4.83 8.28 V9.66 H-4.83 V8.28 Q-2.53 7.59 -2.53 5.29 V1.38 H-3.45 Z', ' fill="currentColor" fill-opacity=".35"')}${line(0, 9.66, 0, 15)}</g>`;

const appWindow = (p) => rect(10, 6, 76, 48, 4, ` fill="${p.pane}" stroke="${p.edge}"`)
  + rect(10.75, 6.75, 20, 46.5, 3.25, ` fill="${p.rail}" stroke="none"`)
  + [14, 20, 26].map((y, i) => line(15, y, 25 - i, y, ` stroke="${p.soft}"`)).join('')
  + rect(31, 21, 54.25, 10, 0, ` fill="${p.band}" stroke="none"`)
  + line(32, 21.5, 32, 30.5, ` stroke="${p.acc}" stroke-width="2"`)
  + line(37, 14, 66, 14, ` stroke="${p.ink}"`) + line(37, 26, 62, 26, ` stroke="${p.ink}"`)
  + line(37, 38, 70, 38, ` stroke="${p.soft}"`) + line(37, 46, 58, 46, ` stroke="${p.soft}"`);

const webWindow = (p) => rect(8, 6, 80, 48, 4, ` fill="${p.bg}" stroke="${p.border}"`)
  + rect(8.75, 6.75, 78.5, 9, 3.25, ` fill="${p.surface}" stroke="none"`)
  + [14, 19, 24].map((x) => circle(x, 11.2, 1.2, ` fill="${p.muted}" stroke="none"`)).join('')
  + rect(32, 9, 40, 4.5, 2.25, ` fill="${p.raised}" stroke="none"`)
  + rect(8.75, 15.75, 17, 37.5, 0, ` fill="${p.surface}" stroke="none"`)
  + rect(30, 20, 25, 14, 2, ` fill="${p.raised}" stroke="none"`) + rect(58, 20, 26, 14, 2, ` fill="${p.raised}" stroke="none"`)
  + rect(30, 37, 54, 13, 2, ` fill="${p.raised}" stroke="none"`)
  + line(34, 26, 48, 26, ` stroke="${p.text}"`) + line(62, 26, 76, 26, ` stroke="${p.text}"`) + line(34, 43, 66, 43, ` stroke="${p.text}"`);

const screen = () => rect(10, 6, 76, 44, 4) + line(40, 56, 56, 56) + line(48, 50, 48, 56);
const toast = (x, y) => wash(x, y, 30, 11, 3, 0.3) + rect(x, y, 30, 11, 3) + dot(x + 5, y + 5.5, 1.6) + line(x + 10, y + 5.5, x + 25, y + 5.5);
const bubble = (x, y) => rect(x, y, 30, 11, 5.5) + circle(x + 6, y + 5.5, 2.2) + line(x + 12, y + 5.5, x + 25, y + 5.5, SOFT);
const letters = (t) => wash(30, 12, 36, 36, 8, 0.2) + rect(30, 12, 36, 36, 8) + text(48, 35, t, 13, 'middle', 'IBM Plex Sans, sans-serif', 600);
const mic = (dx = 0) => rect(42 + dx, 8, 12, 24, 6) + path(`M${36 + dx} 24 Q${36 + dx} 38 ${48 + dx} 38 Q${60 + dx} 38 ${60 + dx} 24`) + line(48 + dx, 38, 48 + dx, 48) + line(40 + dx, 48, 56 + dx, 48);
const deskMic = (dx) => rect(34 + dx, 6, 16, 28, 8) + [14, 19, 24].map((y) => line(38 + dx, y, 46 + dx, y, SOFT)).join('')
  + path(`M${34 + dx} 20 H${30 + dx} V24 Q${30 + dx} 38 ${42 + dx} 38 Q${54 + dx} 38 ${54 + dx} 24 V20 H${50 + dx}`)
  + line(42 + dx, 38, 42 + dx, 47) + rect(31 + dx, 47, 22, 5, 2.5);
const usbLogo = (x) => line(x, 16, x, 43) + path(`M${x - 3.5} 19 L${x} 13 L${x + 3.5} 19 Z`, FILL) + dot(x, 45.5, 2.8)
  + path(`M${x} 37 L${x - 7} 31 V27.5`) + circle(x - 7, 25.5, 2) + path(`M${x} 32 L${x + 7} 26 V23`) + rect(x + 5, 18.5, 4, 4, 0.5);
const speaker = () => path('M22 24 H30 L42 14 V46 L30 36 H22 Z') + path('M50 22 Q56 30 50 38') + path('M56 16 Q66 30 56 44', SOFT);
const cans = () => path('M23.5 30 V27 Q23.5 8 48 8 Q72.5 8 72.5 27 V30') + [17, 66].map((x) => wash(x, 30, 13, 22, 5, 0.2) + rect(x, 30, 13, 22, 5)).join('');
const bar = (y, fill) => rect(28, y, 56, 8, 4) + wash(28, y, 56 * fill, 8, 4, 0.55);
const terminal = (inner) => rect(10, 8, 76, 44, 4) + line(10, 17, 86, 17, SOFT) + [16, 21, 26].map((x) => dot(x, 12.5, 1.2)).join('') + inner;
const folder = (more = '') => path('M12 16 H36 L40 20 H84 V48 H12 Z', more);
const miniFolder = (x, y) => path(`M${x} ${y} H${x + 4.5} L${x + 6} ${y + 1.5} H${x + 11} V${y + 9} H${x} Z`);
const chevron = (x, cy) => path(`M${x} ${cy - 2.5} L${x + 2.5} ${cy} L${x} ${cy + 2.5}`);
const chipFrame = rect(30, 14, 36, 32, 4) + [22, 30, 38].map((y) => line(24, y, 30, y) + line(66, y, 72, y)).join('');
const chip = (label) => chipFrame + text(48, 33, label, 7);
const rune = (cx, cy, k) => path(`M${n(cx - 5 * k)} ${n(cy - 5 * k)} L${n(cx + 5 * k)} ${n(cy + 5 * k)} L${cx} ${n(cy + 10 * k)} V${n(cy - 10 * k)} L${n(cx + 5 * k)} ${n(cy - 5 * k)} L${n(cx - 5 * k)} ${n(cy + 5 * k)}`);
const globe = (cx, cy, r) => circle(cx, cy, r) + path(`M${cx} ${cy - r} Q${cx + r} ${cy} ${cx} ${cy + r} Q${cx - r} ${cy} ${cx} ${cy - r}`) + line(cx - r, cy, cx + r, cy);
const moon = (cx, cy, s, more = '') => {
  const x = n(cx + 5.3 * s);
  const y = n(cy - 6 * s);
  return path(`M${x} ${y} A${n(7 * s)} ${n(7 * s)} 0 1 0 ${x} ${n(y + 12 * s)} A${n(9 * s)} ${n(9 * s)} 0 0 1 ${x} ${y} Z`, FILL + more);
};
const dial = (night, moonMore = '') => circle(48, 30, 21, ' stroke-width="5" stroke-opacity=".16"')
  + [[48, 13, 48, 16], [65, 30, 62, 30], [48, 47, 48, 44], [31, 30, 34, 30]].map(([x1, y1, x2, y2]) => line(x1, y1, x2, y2, SOFT)).join('')
  + (night ? path('M29.81 19.5 A21 21 0 0 1 69 30', ' stroke-width="5"') : '') + moon(48, 30, 1.05, moonMore);
let masks = 0;
const masked = (inner, cutout) => {
  masks += 1;
  const id = `cut-${masks}`;
  return `<mask id="${id}" maskUnits="userSpaceOnUse" x="0" y="0" width="96" height="60"><rect width="96" height="60" fill="#fff" stroke="none"/>${cutout}</mask><g mask="url(#${id})">${inner}</g>`;
};
const slashed = (inner, x1, y1, x2, y2) => masked(inner, line(x1, y1, x2, y2, ' stroke="#000" stroke-width="6.5"')) + line(x1, y1, x2, y2, ' stroke-width="2.2"');
const copies = (front) => masked(rect(28, 8, 44, 32, 3, SOFT), rect(20, 16, 44, 32, 3, ' fill="#000" stroke="#000" stroke-width="3"')) + wash(20, 16, 44, 32, 3, 0.22) + rect(20, 16, 44, 32, 3) + front;

export const SLATE = { pane: '#16171a', rail: '#101114', edge: '#2a2b31', ink: '#f3f2f0', soft: '#8b8880', band: '#464a79', acc: '#8a93f7' };
export const BONE = { pane: '#fffefb', rail: '#f5f2eb', edge: '#e2ded4', ink: '#1a1815', soft: '#78736a', band: '#bbb2d1', acc: '#6f5da8' };
const WEB_SLATE = { bg: '#111317', surface: '#1a1e26', raised: '#272d38', border: '#3e485b', text: '#8a97ae', muted: '#55627a' };
const WEB_MIDNIGHT = { bg: '#090b19', surface: '#141626', raised: '#1b1e30', border: '#34374d', text: '#b2b6cc', muted: '#555974' };

export const pic = {
  holdToSwitch: () => svg(rect(3, 22, 19, 16, 2.5, SOFT) + rect(74, 22, 19, 16, 2.5, SOFT) + wash(26, 12, 44, 36, 3.5, 0.22) + rect(26, 12, 44, 36, 3.5, ' stroke-width="2.2"') + line(26, 19, 70, 19, SOFT)),
  sticky: () => svg(rect(3, 18, 86, 32, 4.5) + [9, 36, 63].map((x, i) => (i === 1 ? wash(x, 24, 20, 20, 2.5, 0.22) : '') + rect(x, 24, 20, 20, 2.5, i === 1 ? ' stroke-width="2.2"' : SOFT)).join('') + pin(86, 24, 20)),
  cycleOnce: () => svg(hopRight + strip(1, 28)),
  showOnly: () => svg(path('M11 16 Q20 8 29 16 Q20 24 11 16 Z') + dot(20, 16, 2.4) + strip(0, 28)),
  iconCorner: (right) => svg(wash(18, 8, 60, 44, 4) + rect(18, 8, 60, 44, 4) + line(24, 44, 56, 44, SOFT) + `<rect x="${right ? 63 : 24}" y="13" width="9" height="9" rx="2" fill="currentColor" stroke="none"/>`),
  aa: (px) => svg(text(48, 30 + px * 0.36, 'Aa', px, 'middle', 'IBM Plex Sans, sans-serif', 600)),
  desktopTheme: (palette) => svg(appWindow(palette)),
  webTheme: (name) => svg(webWindow(name === 'midnight' ? WEB_MIDNIGHT : WEB_SLATE)),
  swatch: (hex) => svg(circle(48, 30, 16, ` fill="${hex}" stroke="none"`)),
  profile: () => svg(circle(48, 20, 7) + path('M33 48 Q33 33 48 33 Q63 33 63 48')),
  qolToasts: () => svg(screen() + toast(50, 34)),
  systemBubbles: () => svg(screen() + bubble(50, 10)),
  bothNotices: () => svg(screen() + bubble(50, 10) + toast(50, 34)),
  launchApp: () => svg([[26, 14], [44, 14], [26, 32], [44, 32]].map(([x, y], i) => (i === 0 ? wash(x, y, 14, 14, 3, 0.35) : '') + rect(x, y, 14, 14, 3)).join('') + path('M66 36 L80 22 M72 22 H80 V30')),
  openUrl: () => svg(rect(10, 8, 76, 44, 4) + line(10, 18, 86, 18) + rect(22, 10.5, 52, 5, 2.5, SOFT) + text(48, 38, 'https://', 10)),
  bundleRef: (t) => svg(path('M10 20 H74 L84 30 L74 40 H10 Z') + text(43, 32.5, t, 7)),
  pathRef: (t) => svg(folder() + text(48, 38, t, 7)),
  nameRef: (t) => svg(rect(12, 20, 72, 20, 4) + text(45, 33.5, t, 9) + line(76, 24, 76, 36)),
  browserBundleRef: () => svg(rect(16, 12, 64, 38, 4) + rect(42, 16, 12, 3, 1.5) + wash(22, 23, 20, 20, 3, 0.2) + globe(32, 33, 7)
    + line(48, 27, 72, 27) + line(48, 34, 68, 34, SOFT) + line(48, 41, 62, 41, SOFT)),
  browserPathRef: () => svg(rect(8, 19, 80, 22, 5) + miniFolder(18, 25.5) + chevron(34, 30) + miniFolder(41, 25.5) + chevron(57, 30) + globe(72, 30, 6)),
  browserNameRef: () => svg(rect(10, 19, 76, 22, 5) + globe(23, 30, 6) + text(34, 33.5, 'Firefox', 10, 'start', 'IBM Plex Sans, sans-serif', 500) + line(68, 24.5, 68, 35.5)),
  plugin: (t) => svg(letters(t)),
  nextWindow: () => svg(hopRight + strip(1, 28)),
  previousWindow: () => svg(hopLeft + strip(1, 28)),
  gear: () => {
    const at = (deg, radius) => `${n(48 + Math.cos((deg * Math.PI) / 180) * radius)} ${n(30 + Math.sin((deg * Math.PI) / 180) * radius)}`;
    const outline = [0, 45, 90, 135, 180, 225, 270, 315].flatMap((a) => [at(a - 12, 12.5), at(a - 7, 17), at(a + 7, 17), at(a + 12, 12.5)]);
    return svg(path(`M${outline.join(' L')} Z`) + circle(48, 30, 5.5));
  },
  adapterAuto: () => svg(screen() + rune(48, 28, 1.3)),
  adapterChip: () => svg(chipFrame + rune(48, 30, 1.05)),
  scheduleOff: () => svg(dial(false, ' fill-opacity=".45"')),
  scheduleDaily: () => svg(dial(true)),
  micDefault: () => svg(screen() + shrink(mic(), 0.62, [48, 28])),
  usbMic: () => svg(deskMic(-9) + usbLogo(66)),
  webcam: () => svg(rect(26, 12, 44, 28, 6) + circle(48, 26, 8) + dot(48, 26, 2.6) + line(48, 40, 48, 48) + line(38, 48, 58, 48)),
  headset: () => svg(cans() + masked(path('M68 43 Q62 54 51 55.2'), rect(66, 30, 13, 22, 5, ' fill="#000" stroke="none"') + rect(45, 52.5, 9, 5.5, 2.75, ' fill="#000" stroke="none"'))
    + rect(45, 52.5, 9, 5.5, 2.75, ' fill="currentColor" fill-opacity=".35"')),
  speakerDefault: () => svg(screen() + shrink(speaker(), 0.62, [44, 30])),
  speakers: () => svg(rect(16, 12, 20, 38, 3) + circle(26, 36, 6) + dot(26, 21, 2) + rect(60, 12, 20, 38, 3) + circle(70, 36, 6) + dot(70, 21, 2)),
  hdmi: () => svg(rect(12, 10, 52, 32, 3) + line(32, 48, 44, 48) + line(38, 42, 38, 48) + path('M72 22 Q77 28 72 34') + path('M78 17 Q86 28 78 39', SOFT)),
  headphones: () => svg(cans()),
  preset: (speed, size) => svg(path('M18 7 L11 18 H16 L14 26 L22 14 H17 Z') + bar(12, speed) + path('M12 34 H18 L22 38 V48 H12 Z') + bar(37, size)),
  format: (ext) => svg(path('M28 5 H58 L68 15 V55 H28 Z') + path('M58 5 V15 H68') + path('M43 20 L43 32 L53 26 Z', ' fill="currentColor" stroke="none"') + text(48, 46, ext, 9)),
  copyImage: () => svg(copies(path('M24 44 L34 32 L42 40 L48 34 L60 44') + circle(53, 24, 3))),
  copyPath: () => svg(copies(text(42, 29, '~/Pictures', 7) + text(42, 40, 'shot.png', 7))),
  noTerminal: () => svg(slashed(terminal(text(18, 38, '&gt;_', 11, 'start')), 18, 5, 78, 55)),
  terminalSession: (name) => svg(terminal(text(16, 30, '&gt; ' + name, 8, 'start') + line(16, 38, 62, 38, SOFT) + line(16, 45, 48, 45, SOFT))),
  insertOnly: () => svg(terminal(text(16, 34, '&gt; hello', 9, 'start') + rect(57, 26, 2, 11, 0, FILL))),
  insertSubmit: () => svg(terminal(text(16, 34, '&gt; hello', 9, 'start') + path('M73 24 V28.5 Q73 32 69.5 32 H60') + path('M63.5 28.5 L60 32 L63.5 35.5'))),
  preferLocal: () => svg(dot(12, 30, 2.4) + path('M14 30 H24 L33 18 H50') + path('M46.5 14.5 L50 18 L46.5 21.5') + path('M24 30 L33 42 H50', ' stroke-dasharray="2.5 3.5"' + SOFT)
    + rect(55, 9, 24, 18, 3) + [61, 67, 73].map((x) => line(x, 5.5, x, 9) + line(x, 27, x, 30.5)).join('')
    + path('M60 50 H75 A5 5 0 0 0 75.5 40 A7 7 0 0 0 62.5 40.5 A4.75 4.75 0 0 0 60 50 Z', SOFT)),
  localEngine: (label) => svg(chip(label)),
  remoteEngine: () => svg(rect(12, 16, 24, 28, 3) + line(17, 24, 31, 24, SOFT) + line(17, 30, 31, 30, SOFT) + rect(60, 16, 24, 28, 3) + line(65, 24, 79, 24, SOFT) + line(65, 30, 79, 30, SOFT) + path('M38 30 H58', ' stroke-dasharray="3 3"') + path('M53 26 L58 30 L53 34')),
  noModel: () => svg(folder(' stroke-dasharray="3 3"')),
  model: () => svg(folder() + path('M24 34 L28 28 L32 40 L36 24 L40 36 L44 30 L48 34 L52 29 L56 38 L60 32 L64 34')),
  family: (t) => svg(letters(t)),
  familyAuto: () => svg(folder(SOFT) + circle(50, 32, 10) + line(57.5, 39.5, 65, 47, ' stroke-width="2.6"') + path('M42.5 32 L45 29 L47.5 35.5 L50 26.5 L52.5 36 L55 30 L57.5 32')),
  fixedSize: () => svg(rect(8, 6, 80, 46, 4) + wash(28, 14, 40, 30, 2) + rect(28, 14, 40, 30, 2) + text(48, 32, '1152×892', 7)),
  relativeSize: () => svg(rect(8, 6, 80, 46, 4) + wash(22.5, 10.5, 51, 37, 2) + rect(22.5, 10.5, 51, 37, 2) + text(48, 32.5, '64%', 10))
};
