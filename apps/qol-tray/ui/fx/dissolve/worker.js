import {
    createEvaporateState, evaporateFrame,
} from './engine.js';

let state = null;
let buffer = null;

self.onmessage = (e) => {
    const { type } = e.data;
    if (type === 'start') init(e.data);
    if (type === 'buffer') resumeWithBuffer(e.data.buffer);
    if (type === 'stop') { state = null; buffer = null; }
};

function init({ bgColor, targetColor, opts, pixelBuffer }) {
    const canvas = new OffscreenCanvas(1, 1);
    state = createEvaporateState(canvas, bgColor, targetColor, opts);
    buffer = pixelBuffer;
    tick();
}

function resumeWithBuffer(pixelBuffer) {
    buffer = pixelBuffer;
    if (state) tick();
}

function tick() {
    if (!state || !buffer) return;
    const done = evaporateFrame(state);
    const src = state.d;
    const dst = new Uint8Array(buffer);
    dst.set(src);
    self.postMessage({ type: done ? 'done' : 'frame', buffer }, [buffer]);
    buffer = null;
}
