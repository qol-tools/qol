const PLACEHOLDER_SVG = 'data:image/svg+xml,' + encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200">' +
    '<rect fill="#333" width="300" height="200"/>' +
    '<text fill="#666" x="50%" y="50%" text-anchor="middle" dy=".3em" font-family="sans-serif" font-size="14">No Cover</text>' +
    '</svg>'
);

const state = {
    plugins: [],
    selectedIndex: 0,
    columns: 4
};

async function init() {
    try {
        const response = await fetch('/api/installed');
        if (!response.ok) throw new Error('Failed to fetch plugins');
        
        state.plugins = await response.json();
        render();
        updateSelection();
        
        document.addEventListener('keydown', handleKeydown);
        document.getElementById('grid').addEventListener('click', handleClick);
    } catch (error) {
        document.getElementById('grid').innerHTML = 
            `<div class="error">Error loading plugins: ${error.message}</div>`;
    }
}

function render() {
    const grid = document.getElementById('grid');
    
    if (state.plugins.length === 0) {
        grid.innerHTML = '<div class="empty">No plugins installed. Press Tab to open the store.</div>';
        return;
    }
    
    grid.innerHTML = state.plugins.map((plugin, index) => {
        const coverUrl = plugin.has_cover ? `/api/cover/${plugin.id}` : PLACEHOLDER_SVG;
        const noUiClass = plugin.has_ui ? '' : 'no-ui';
        
        return `
            <div class="plugin-card ${noUiClass}" data-index="${index}">
                <img src="${coverUrl}" alt="${plugin.name}" onerror="this.src='${PLACEHOLDER_SVG}'">
                <div class="plugin-name">${plugin.name}</div>
            </div>
        `;
    }).join('');
}

function updateSelection() {
    document.querySelectorAll('.plugin-card').forEach((card, i) => {
        card.classList.toggle('selected', i === state.selectedIndex);
    });
    
    const selected = document.querySelector('.plugin-card.selected');
    if (selected) {
        selected.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
}

function handleClick(e) {
    const card = e.target.closest('.plugin-card');
    if (!card) return;
    
    const index = parseInt(card.dataset.index, 10);
    if (index !== state.selectedIndex) {
        state.selectedIndex = index;
        updateSelection();
    } else {
        openSelected();
    }
}

function handleKeydown(e) {
    const handlers = {
        ArrowUp: () => navigate(-state.columns),
        ArrowDown: () => navigate(state.columns),
        ArrowLeft: () => navigate(-1),
        ArrowRight: () => navigate(1),
        Enter: openSelected,
        Tab: () => !e.shiftKey && (window.location.href = '/store.html')
    };
    
    const handler = handlers[e.key];
    if (handler) {
        e.preventDefault();
        handler();
    }
}

function navigate(delta) {
    const total = state.plugins.length;
    if (total === 0) return;
    
    const newIndex = Math.max(0, Math.min(total - 1, state.selectedIndex + delta));
    
    if (newIndex !== state.selectedIndex) {
        state.selectedIndex = newIndex;
        updateSelection();
    }
}

function openSelected() {
    if (state.plugins.length === 0) return;
    
    const plugin = state.plugins[state.selectedIndex];
    if (plugin.has_ui) {
        window.location.href = `/plugins/${plugin.id}/`;
    }
}

init();
