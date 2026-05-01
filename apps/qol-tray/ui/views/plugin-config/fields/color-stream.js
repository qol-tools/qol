const WS_URL = 'ws://127.0.0.1:42710';
let socket = null;
let refCount = 0;

function ensureSocket() {
    if (socket && socket.readyState === WebSocket.OPEN) return socket;
    if (socket && socket.readyState === WebSocket.CONNECTING) return socket;
    socket = new WebSocket(WS_URL);
    socket.onclose = () => { socket = null; };
    socket.onerror = () => { socket = null; };
    return socket;
}

export function openColorStream() {
    refCount++;
    return ensureSocket();
}

export function closeColorStream() {
    refCount--;
    if (refCount <= 0) {
        refCount = 0;
        if (socket) {
            socket.close();
            socket = null;
        }
    }
}

export function streamColorHex(hex) {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    socket.send(JSON.stringify({ type: 'color', hex }));
}

export function streamBrightness(level, hex) {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    socket.send(JSON.stringify({ type: 'brightness', level, hex }));
}
