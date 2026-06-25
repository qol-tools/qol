let socket = null;
let refCount = 0;
let wsUrl = null;

function ensureSocket() {
    if (!wsUrl) return null;
    if (socket && socket.readyState === WebSocket.OPEN) return socket;
    if (socket && socket.readyState === WebSocket.CONNECTING) return socket;
    socket = new WebSocket(wsUrl);
    socket.onclose = () => { socket = null; };
    socket.onerror = () => { socket = null; };
    return socket;
}

export function openColorStream(port) {
    if (port) wsUrl = `ws://127.0.0.1:${port}`;
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
