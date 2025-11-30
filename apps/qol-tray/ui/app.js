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
            <button
                class="${plugin.installed ? 'installed' : 'install'}"
                onclick="${plugin.installed ? '' : `installPlugin('${plugin.id}')`}"
                ${plugin.installed ? 'disabled' : ''}
            >
                ${plugin.installed ? 'Installed' : 'Install'}
            </button>
        </div>
    `).join('');
}

async function installPlugin(id) {
    try {
        const response = await fetch(`/api/install/${id}`, {
            method: 'POST'
        });

        if (!response.ok) throw new Error('Installation failed');

        const plugin = allPlugins.find(p => p.id === id);
        if (plugin) {
            plugin.installed = true;
            renderPlugins(allPlugins);
        }

        alert(`Plugin ${id} installed successfully! Restart QoL Tray to see changes.`);
    } catch (error) {
        alert(`Failed to install plugin: ${error.message}`);
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
