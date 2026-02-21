function buildError(response, message) {
    const error = new Error(message || `Request failed (${response.status})`);
    error.status = response.status;
    error.response = response;
    return error;
}

async function readErrorMessage(response) {
    try {
        const text = (await response.text())?.trim();
        if (text) return text;
    } catch {}
    return `Request failed (${response.status})`;
}

export async function apiJson(url, options) {
    const response = await fetch(url, options);
    if (!response.ok) {
        throw buildError(response, await readErrorMessage(response));
    }
    if (response.status === 204) return null;
    return response.json();
}

export async function apiText(url, options) {
    const response = await fetch(url, options);
    if (!response.ok) {
        throw buildError(response, await readErrorMessage(response));
    }
    return response.text();
}

export async function apiResponse(url, options) {
    return fetch(url, options);
}
