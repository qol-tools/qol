import { useEffect, useState } from 'preact/hooks';

export function useAppBootstrap() {
    const [devEnabled, setDevEnabled] = useState(false);
    const [appVersion, setAppVersion] = useState(null);

    useEffect(() => {
        (async () => {
            let dev = false;
            try {
                const res = await fetch('/api/dev/enabled');
                dev = res.ok && await res.json();
            } catch {}
            setDevEnabled(dev);

            try {
                const res = await fetch('/api/version');
                if (res.ok) {
                    setAppVersion(await res.text());
                }
            } catch {}
        })();
    }, []);

    return { devEnabled, appVersion };
}
