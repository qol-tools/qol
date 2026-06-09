const BG_VERT = `#version 300 es
in vec2 a_pos;
out vec2 v_uv;
void main() {
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}`;

const BG_FRAG = `#version 300 es
precision highp float;
uniform sampler2D u_order;
uniform float u_progress;
uniform vec3 u_bg;
in vec2 v_uv;
out vec4 o;
void main() {
    float t = texture(u_order, v_uv).r;
    if (t < u_progress) discard;
    o = vec4(u_bg, 1.0);
}`;

const PT_VERT = `#version 300 es
in vec2 a_pos;
in vec4 a_col;
uniform vec2 u_res;
out vec4 v_col;
void main() {
    vec2 c = (a_pos / u_res) * 2.0 - 1.0;
    c.y = -c.y;
    gl_Position = vec4(c, 0.0, 1.0);
    gl_PointSize = 1.0;
    v_col = a_col;
}`;

const PT_FRAG = `#version 300 es
precision lowp float;
in vec4 v_col;
out vec4 o;
void main() { o = v_col; }`;

function compile(gl, type, src) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    return s;
}

function link(gl, vs, fs) {
    const p = gl.createProgram();
    gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs));
    gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
    gl.linkProgram(p);
    return p;
}

export function initGPU(canvas, W, H, shuffleIndices, total, bgColor) {
    const gl = canvas.getContext('webgl2', { alpha: true, premultipliedAlpha: false, antialias: false });
    if (!gl) return null;

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    const bgProg = link(gl, BG_VERT, BG_FRAG);
    const ptProg = link(gl, PT_VERT, PT_FRAG);

    const quadBuf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, quadBuf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, -1,1, -1,1, 1,-1, 1,1]), gl.STATIC_DRAW);

    const orderData = new Float32Array(W * H);
    const invTotal = 1 / total;
    for (let i = 0; i < total; i++) {
        orderData[shuffleIndices[i]] = i * invTotal;
    }
    const orderTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, orderTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R32F, W, H, 0, gl.RED, gl.FLOAT, orderData);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

    const ptBuf = gl.createBuffer();

    const bgLocs = {
        aPos: gl.getAttribLocation(bgProg, 'a_pos'),
        uOrder: gl.getUniformLocation(bgProg, 'u_order'),
        uProgress: gl.getUniformLocation(bgProg, 'u_progress'),
        uBg: gl.getUniformLocation(bgProg, 'u_bg'),
    };
    const ptLocs = {
        aPos: gl.getAttribLocation(ptProg, 'a_pos'),
        aCol: gl.getAttribLocation(ptProg, 'a_col'),
        uRes: gl.getUniformLocation(ptProg, 'u_res'),
    };

    return {
        gl, bgProg, ptProg, quadBuf, ptBuf, orderTex,
        bgLocs, ptLocs, W, H, total,
        bgR: bgColor[0] / 255, bgG: bgColor[1] / 255, bgB: bgColor[2] / 255,
        particleData: new Float32Array(total * 6),
    };
}

export function renderFrame(gpu, progress, particleCount, particleData) {
    const { gl, W, H } = gpu;
    gl.viewport(0, 0, W, H);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);

    gl.useProgram(gpu.bgProg);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, gpu.orderTex);
    gl.uniform1i(gpu.bgLocs.uOrder, 0);
    gl.uniform1f(gpu.bgLocs.uProgress, progress);
    gl.uniform3f(gpu.bgLocs.uBg, gpu.bgR, gpu.bgG, gpu.bgB);
    gl.bindBuffer(gl.ARRAY_BUFFER, gpu.quadBuf);
    gl.enableVertexAttribArray(gpu.bgLocs.aPos);
    gl.vertexAttribPointer(gpu.bgLocs.aPos, 2, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.TRIANGLES, 0, 6);

    if (particleCount > 0) {
        gl.useProgram(gpu.ptProg);
        gl.uniform2f(gpu.ptLocs.uRes, W, H);
        gl.bindBuffer(gl.ARRAY_BUFFER, gpu.ptBuf);
        gl.bufferData(gl.ARRAY_BUFFER, particleData.subarray(0, particleCount * 6), gl.STREAM_DRAW);
        gl.enableVertexAttribArray(gpu.ptLocs.aPos);
        gl.vertexAttribPointer(gpu.ptLocs.aPos, 2, gl.FLOAT, false, 24, 0);
        gl.enableVertexAttribArray(gpu.ptLocs.aCol);
        gl.vertexAttribPointer(gpu.ptLocs.aCol, 4, gl.FLOAT, false, 24, 8);
        gl.drawArrays(gl.POINTS, 0, particleCount);
    }
}
