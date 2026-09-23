import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { BONE, SLATE, pic } from './pics.mjs';

const here = dirname(fileURLToPath(import.meta.url));

const renumberMasks = (markup) => {
  const names = new Map();
  return markup.replace(/cut-\d+/g, (name) => {
    if (!names.has(name)) {
      names.set(name, `cut-${names.size + 1}`);
    }
    return names.get(name);
  });
};

const golden = (markup) =>
  renumberMasks(markup.replace(' class="art"', '')).replace(
    '<svg ',
    '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="60" ',
  );

const fileFor = (spec) => `${spec.replace(/:/g, '-').replace(/,/g, '-')}.svg`;

const goldens = [
  ['hold-to-switch', () => pic.holdToSwitch()],
  ['sticky', () => pic.sticky()],
  ['cycle-once', () => pic.cycleOnce()],
  ['show-only', () => pic.showOnly()],
  ['icon-corner:right', () => pic.iconCorner(true)],
  ['icon-corner:left', () => pic.iconCorner(false)],
  ['aa:15', () => pic.aa(15)],
  ['aa:22', () => pic.aa(22)],
  ['aa:30', () => pic.aa(30)],
  ['desktop-theme:bone', () => pic.desktopTheme(BONE)],
  ['desktop-theme:slate', () => pic.desktopTheme(SLATE)],
  ['web-theme:slate', () => pic.webTheme('slate')],
  ['web-theme:midnight', () => pic.webTheme('midnight')],
  ['swatch:amber', () => pic.swatch('#e0ac3f')],
  ['swatch:green', () => pic.swatch('#46e08a')],
  ['swatch:cyan', () => pic.swatch('#56d6e0')],
  ['swatch:magenta', () => pic.swatch('#e879c6')],
  ['swatch:blue', () => pic.swatch('#4a9eff')],
  ['swatch:violet', () => pic.swatch('#8a93f7')],
  ['qol-toasts', () => pic.qolToasts()],
  ['system-bubbles', () => pic.systemBubbles()],
  ['both-notices', () => pic.bothNotices()],
  ['launch-app', () => pic.launchApp()],
  ['open-url', () => pic.openUrl()],
  ['bundle-ref', () => pic.bundleRef('com.example.App')],
  ['path-ref', () => pic.pathRef('\u2026/App.app')],
  ['name-ref', () => pic.nameRef('App Name')],
  ['browser-bundle-ref', () => pic.browserBundleRef()],
  ['browser-path-ref', () => pic.browserPathRef()],
  ['browser-name-ref', () => pic.browserNameRef()],
  ['letters:D', () => pic.plugin('D')],
  ['letters:WL', () => pic.plugin('WL')],
  ['letters:AT', () => pic.plugin('AT')],
  ['letters:B', () => pic.plugin('B')],
  ['letters:CS', () => pic.plugin('CS')],
  ['letters:C', () => pic.plugin('C')],
  ['letters:IC', () => pic.plugin('IC')],
  ['letters:KR', () => pic.plugin('KR')],
  ['letters:L', () => pic.plugin('L')],
  ['letters:Li', () => pic.plugin('Li')],
  ['letters:M', () => pic.plugin('M')],
  ['letters:OT', () => pic.plugin('OT')],
  ['letters:PZ', () => pic.plugin('PZ')],
  ['letters:QM', () => pic.plugin('QM')],
  ['letters:QS', () => pic.plugin('QS')],
  ['letters:QV', () => pic.plugin('QV')],
  ['letters:WA', () => pic.plugin('WA')],
  ['letters:ZT', () => pic.plugin('ZT')],
  ['letters:PK', () => pic.plugin('PK')],
  ['letters:W', () => pic.plugin('W')],
  ['letters:SV', () => pic.plugin('SV')],
  ['letters:PF', () => pic.plugin('PF')],
  ['letters:NC', () => pic.plugin('NC')],
  ['letters:MS', () => pic.plugin('MS')],
  ['next-window', () => pic.nextWindow()],
  ['previous-window', () => pic.previousWindow()],
  ['gear', () => pic.gear()],
  ['adapter-auto', () => pic.adapterAuto()],
  ['adapter-chip', () => pic.adapterChip()],
  ['schedule-off', () => pic.scheduleOff()],
  ['schedule-daily', () => pic.scheduleDaily()],
  ['mic-default', () => pic.micDefault()],
  ['usb-mic', () => pic.usbMic()],
  ['webcam', () => pic.webcam()],
  ['headset', () => pic.headset()],
  ['speaker-default', () => pic.speakerDefault()],
  ['hdmi', () => pic.hdmi()],
  ['headphones', () => pic.headphones()],
  ['preset:100,100', () => pic.preset(1, 1)],
  ['preset:88,92', () => pic.preset(0.88, 0.92)],
  ['preset:76,84', () => pic.preset(0.76, 0.84)],
  ['preset:64,78', () => pic.preset(0.64, 0.78)],
  ['preset:54,72', () => pic.preset(0.54, 0.72)],
  ['preset:44,66', () => pic.preset(0.44, 0.66)],
  ['preset:33,60', () => pic.preset(0.33, 0.6)],
  ['preset:22,56', () => pic.preset(0.22, 0.56)],
  ['preset:12,52', () => pic.preset(0.12, 0.52)],
  ['format:MKV', () => pic.format('MKV')],
  ['format:MP4', () => pic.format('MP4')],
  ['format:MOV', () => pic.format('MOV')],
  ['format:WEBM', () => pic.format('WEBM')],
  ['copy-image', () => pic.copyImage()],
  ['copy-path', () => pic.copyPath()],
  ['no-terminal', () => pic.noTerminal()],
  ['terminal-session:claude', () => pic.terminalSession('claude')],
  ['insert-only', () => pic.insertOnly()],
  ['insert-submit', () => pic.insertSubmit()],
  ['prefer-local', () => pic.preferLocal()],
  ['local-engine:onnx', () => pic.localEngine('onnx')],
  ['local-engine:candle', () => pic.localEngine('candle')],
  ['remote-engine', () => pic.remoteEngine()],
  ['no-model', () => pic.noModel()],
  ['model', () => pic.model()],
  ['family-auto', () => pic.familyAuto()],
  ['fixed-size', () => pic.fixedSize()],
  ['relative-size', () => pic.relativeSize()],
];

mkdirSync(here, { recursive: true });
for (const [spec, render] of goldens) {
  writeFileSync(join(here, fileFor(spec)), golden(render()));
}
