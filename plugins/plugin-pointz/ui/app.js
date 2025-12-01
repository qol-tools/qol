import QRCode from 'https://esm.sh/qrcode@1.5.4';

const STATUS_URL = 'http://127.0.0.1:45460/status';

async function fetchStatus() {
    try {
        const response = await fetch(STATUS_URL);
        if (!response.ok) throw new Error('Server error');
        return await response.json();
    } catch (error) {
        console.error('Failed to fetch status:', error);
        return null;
    }
}

function showStatus(status) {
    document.getElementById('status').classList.add('hidden');
    document.getElementById('error-section').classList.add('hidden');
    document.getElementById('qr-section').classList.remove('hidden');

    document.getElementById('hostname').textContent = status.hostname;
    document.getElementById('ip').textContent = status.ip || 'Not available';
    document.getElementById('discovery-port').textContent = status.discovery_port;
    document.getElementById('command-port').textContent = status.command_port;
    document.getElementById('download-link').href = status.app_download_url;

    const qrContainer = document.getElementById('qr-code');
    qrContainer.innerHTML = '';

    QRCode.toDataURL(status.app_download_url, { width: 200, margin: 1 })
        .then(url => {
            const img = document.createElement('img');
            img.src = url;
            img.alt = 'QR Code';
            qrContainer.appendChild(img);
        })
        .catch(err => console.error('QR generation failed:', err));
}

function showError() {
    document.getElementById('status').classList.add('hidden');
    document.getElementById('qr-section').classList.add('hidden');
    document.getElementById('error-section').classList.remove('hidden');
}

async function init() {
    const status = await fetchStatus();
    if (status) {
        showStatus(status);
    } else {
        showError();
    }
}

init();

setInterval(async () => {
    const status = await fetchStatus();
    if (status) {
        showStatus(status);
    }
}, 5000);
