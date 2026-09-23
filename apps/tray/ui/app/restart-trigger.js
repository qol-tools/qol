export async function postRestartTrigger(url, fetchOpts, onHttpError) {
    let response;
    try {
        response = await fetch(url, { ...fetchOpts, qolSuppressErrorToast: true });
    } catch {
        return;
    }
    if (response.ok) return;
    await onHttpError(response);
}
