let results = [];
let selectedIndex = 0;
let debounceTimer = null;

const searchInput = document.getElementById('search');
const resultsContainer = document.getElementById('results');
const actionHint = document.getElementById('action-hint');

function renderResults() {
    if (results.length === 0) {
        resultsContainer.innerHTML = `
            <div class="empty-state">
                <div>Type to search files and directories</div>
                <div class="keyboard-hints">
                    <span><kbd>Enter</kbd> Open</span>
                    <span><kbd>Ctrl+Enter</kbd> Terminal</span>
                    <span><kbd>Shift+Enter</kbd> Folder</span>
                    <span><kbd>Alt+Enter</kbd> Copy</span>
                    <span><kbd>Esc</kbd> Close</span>
                </div>
            </div>
        `;
        return;
    }

    resultsContainer.innerHTML = results.map((result, index) => `
        <div class="result-item ${index === selectedIndex ? 'selected' : ''}" data-index="${index}">
            <span class="result-icon">${result.is_dir ? '📁' : '📄'}</span>
            <div class="result-info">
                <div class="result-name">${escapeHtml(result.name)}</div>
                <div class="result-path">${escapeHtml(result.path)}</div>
            </div>
        </div>
    `).join('');

    const selected = resultsContainer.querySelector('.selected');
    if (selected) {
        selected.scrollIntoView({ block: 'nearest' });
    }
}

function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

function search(query) {
    if (!query) {
        results = [];
        renderResults();
        return;
    }
    window.ipc.postMessage(JSON.stringify({ type: 'search', query }));
}

window.onSearchResults = function(data) {
    results = data;
    selectedIndex = 0;
    renderResults();
};

function executeSelected(action) {
    if (results.length === 0) return;
    const selected = results[selectedIndex];
    window.ipc.postMessage(JSON.stringify({
        type: 'execute',
        path: selected.path,
        action: action
    }));
}

function close() {
    window.ipc.postMessage(JSON.stringify({ type: 'close' }));
}

function updateActionHint(e) {
    if (e.ctrlKey) {
        actionHint.textContent = 'Terminal';
    } else if (e.shiftKey) {
        actionHint.textContent = 'Open Folder';
    } else if (e.altKey) {
        actionHint.textContent = 'Copy Path';
    } else {
        actionHint.textContent = '';
    }
}

searchInput.addEventListener('input', (e) => {
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
        search(e.target.value.trim());
    }, 100);
});

document.addEventListener('keydown', (e) => {
    updateActionHint(e);

    switch (e.key) {
        case 'Escape':
            close();
            break;
        case 'ArrowDown':
            e.preventDefault();
            if (results.length > 0) {
                selectedIndex = (selectedIndex + 1) % results.length;
                renderResults();
            }
            break;
        case 'ArrowUp':
            e.preventDefault();
            if (results.length > 0) {
                selectedIndex = (selectedIndex - 1 + results.length) % results.length;
                renderResults();
            }
            break;
        case 'Enter':
            e.preventDefault();
            if (e.ctrlKey) {
                executeSelected('terminal');
            } else if (e.shiftKey) {
                executeSelected('folder');
            } else if (e.altKey) {
                executeSelected('copy');
            } else {
                executeSelected('open');
            }
            break;
    }
});

document.addEventListener('keyup', updateActionHint);

resultsContainer.addEventListener('click', (e) => {
    const item = e.target.closest('.result-item');
    if (item) {
        selectedIndex = parseInt(item.dataset.index, 10);
        executeSelected('open');
    }
});

renderResults();
