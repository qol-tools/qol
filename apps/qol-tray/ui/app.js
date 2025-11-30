let allPlugins = [];

async function loadPlugins() {
    const container = document.getElementById('plugins-list');

    try {
        const response = await fetch('/api/plugins');
        if (!response.ok) throw new Error('Failed to fetch plugins');

        allPlugins = await response.json();
        renderPlugins(allPlugins);
    } catch (error) {
        container.innerHTML = `<div class="error">Error loading plugins: ${error.message}</div>`;
    }
}

function renderPlugins(plugins) {
    const container = document.getElementById('plugins-list');

    if (plugins.length === 0) {
        container.innerHTML = '<div class="loading">No plugins found</div>';
        return;
    }

    container.innerHTML = plugins.map(plugin => `
        <div class="plugin-card">
            <h3>${plugin.name}</h3>
            <div class="version">v${plugin.version}</div>
            <div class="description">${plugin.description}</div>
            <div class="button-group">
                ${plugin.installed ? `
                    <button class="uninstall" onclick="uninstallPlugin('${plugin.id}')">
                        Uninstall
                    </button>
                ` : `
                    <button class="install" onclick="installPlugin('${plugin.id}')">
                        Install
                    </button>
                `}
            </div>
        </div>
    `).join('');
}

async function installPlugin(id) {
    try {
        const response = await fetch(`/api/install/${id}`, { method: 'POST' });
        if (!response.ok) throw new Error('Installation failed');

        const plugin = allPlugins.find(p => p.id === id);
        if (plugin) {
            plugin.installed = true;
            renderPlugins(allPlugins);
        }
    } catch (error) {
        console.error(`Failed to install plugin: ${error.message}`);
    }
}

async function uninstallPlugin(id) {
    try {
        const response = await fetch(`/api/uninstall/${id}`, { method: 'POST' });
        const result = await response.json();

        if (!result.success) throw new Error(result.message);

        const plugin = allPlugins.find(p => p.id === id);
        if (plugin) {
            plugin.installed = false;
            renderPlugins(allPlugins);
        }
    } catch (error) {
        console.error(`Failed to uninstall plugin: ${error.message}`);
    }
}

document.getElementById('search').addEventListener('input', (e) => {
    const query = e.target.value.toLowerCase();
    const filtered = allPlugins.filter(p =>
        p.name.toLowerCase().includes(query) ||
        p.description.toLowerCase().includes(query)
    );
    renderPlugins(filtered);
});

loadPlugins();
