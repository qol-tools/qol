(() => {
  'use strict';

  const data = JSON.parse(document.getElementById('architecture-data').textContent);
  const $ = selector => document.querySelector(selector);
  const $$ = selector => [...document.querySelectorAll(selector)];
  const rowsById = new Map(data.rows.map(row => [row.id, row]));
  const rowsByPackage = new Map();
  for (const row of data.rows) if (row.package && !rowsByPackage.has(row.package)) rowsByPackage.set(row.package, row);
  const categoryNames = { core: 'Core owners', plugins: 'Plugin domains', libraries: 'Shared crates', surfaces: 'Apps & tooling' };
  const escape = value => String(value).replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);
  const plain = value => value.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1').replace(/[*`]/g, '');
  const inline = value => escape(value)
    .replace(/\[([^\]]+)\]\((\.\.\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1 ↗</a>')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');

  const paths = {
    monitor: '<rect x="3" y="3" width="18" height="13" rx="2"/><path d="M8 21h8m-4-5v5"/>',
    laptop: '<path d="M5 16V5a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v11M2 16h20l-1 4H3Z"/>',
    layers: '<path d="m12 3 9 5-9 5-9-5Zm-9 9 9 5 9-5M3 16l9 5 9-5"/>',
    core: '<rect x="7" y="7" width="10" height="10" rx="2"/><path d="M10 2v5m4-5v5m-4 10v5m4-5v5M2 10h5m-5 4h5m10-4h5m-5 4h5"/><path d="M10 10h4v4h-4z"/>',
    database: '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 4 16 4 16 0V5M4 12c0 4 16 4 16 0"/>',
    plugin: '<path d="M8 3v4m8-4v4M6 7h12v5a6 6 0 0 1-12 0Zm6 11v4"/>',
    cpu: '<path d="M4 5h16v14H4Z"/><path d="M8 9h8v6H8Zm0-8v4m8-4v4M8 19v4m8-4v4M0 9h4m-4 6h4m16-6h4m-4 6h4"/>',
    link: '<path d="m10 13 4-4m-6 7-1 1a4 4 0 0 1-6-6l4-4a4 4 0 0 1 6 0m2 10a4 4 0 0 0 6 0l4-4a4 4 0 0 0-6-6l-1 1" transform="translate(1 0) scale(.92)"/>',
    check: '<path d="m5 12 4 4L19 6"/>',
    search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
    headphones: '<path d="M4 14v-3a8 8 0 0 1 16 0v3"/><rect x="3" y="12" width="5" height="9" rx="2"/><rect x="16" y="12" width="5" height="9" rx="2"/>',
    phone: '<rect x="6" y="2" width="12" height="20" rx="2"/><path d="M10 5h4m-3 14h2"/>',
    play: '<path d="m8 4 12 8-12 8Z"/>',
    pause: '<path d="M8 4v16M16 4v16"/>',
  };
  const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name] ?? ''}</svg>`;
  $$('[data-icon]').forEach(element => { element.innerHTML = icon(element.dataset.icon); });

  let currentView = 'status';
  let category = 'core';
  let selectedOwner = data.nodes.core;
  let selectedNode = 'core';
  let step = 0;
  let timer = null;
  const stepTitles = ['Find the connection', 'Release on the laptop', 'Connect on the desktop', 'Verify and reconcile'];
  const viewCopy = {
    status: { eyebrow: '01 / IMPLEMENTATION STATUS', title: 'Before this work.<br><span>Current worktree.</span>', intro: 'Compare implementation, fixture evidence, and remaining work from the design document. Test results do not mean a feature is deployed.' },
    overview: { eyebrow: '02 / THE FOUNDATION', title: 'Your devices.<br><span>One shared foundation.</span>', intro: 'Link once in core. Let every plugin use the same connection. Keep one owner for every fact.' },
    owners: { eyebrow: '03 / THE OWNERSHIP MAP', title: 'Every fact.<br><span>One place to own it.</span>', intro: 'Explore the full monorepo. Follow each fact to its authority, its consumers, and its source.' },
    handoff: { eyebrow: '04 / ILLUSTRATIVE HANDOFF', title: 'An intended handoff.<br><span>Illustration only.</span>', intro: 'Explore the intended phone, laptop, and desktop sequence. These controls only advance an illustration; they never connect to or change real devices.' },
  };

  function stopPlayback() {
    if (timer !== null) clearInterval(timer);
    timer = null;
    renderPlayButton();
  }

  function showView(view, updateHash = true) {
    if (!viewCopy[view]) view = 'status';
    if (view !== 'handoff') stopPlayback();
    currentView = view;
    $$('.view').forEach(element => { element.hidden = element.id !== view; });
    $$('[data-view]').forEach(button => {
      if (button.dataset.view === view) button.setAttribute('aria-current', 'page');
      else button.removeAttribute('aria-current');
    });
    $('#eyebrow').textContent = viewCopy[view].eyebrow;
    $('#page-title').innerHTML = viewCopy[view].title;
    $('#page-intro').textContent = viewCopy[view].intro;
    document.title = `${view === 'status' ? 'Before / Worktree status' : view === 'overview' ? 'Linked devices' : view === 'owners' ? 'Sources of truth' : 'Illustrative handoff'} — QoL architecture`;
    if (updateHash && location.hash !== `#${view}`) history.replaceState(null, '', `#${view}`);
  }

  function packageChips(names) {
    if (!names.length) return '<span class="dependency-empty">None declared in this workspace.</span>';
    return `<div class="dependency-chips">${names.map(name => {
      const row = rowsByPackage.get(name);
      return row ? `<button type="button" data-owner="${escape(row.id)}">${escape(name)}</button>` : `<span class="dependency-empty">${escape(name)}</span>`;
    }).join('')}</div>`;
  }

  function inspect(row, target, overview = false) {
    target.innerHTML = `<div class="inspector-kicker"><span class="eyebrow">${escape(categoryNames[row.category])}</span>${row.proposed ? '<span class="status-tag">PROPOSED</span>' : ''}</div>
      <h2>${escape(row.name)}</h2>
      ${row.fields.map(field => `<section class="fact"><h3>${escape(field.label)}</h3><p>${inline(field.text)}</p></section>`).join('')}
      ${row.sources.map(source => `<a class="source-link" href="${escape(source.url)}" target="_blank" rel="noopener"><span>Open source · ${escape(source.label)}</span><span aria-hidden="true">↗</span></a>`).join('')}
      ${row.package ? `<details class="dependencies"><summary>Explore dependency relationships</summary><h3>Declared workspace dependencies</h3>${packageChips(row.dependencies)}<h3>Direct workspace consumers</h3>${packageChips(row.consumers)}<p class="inspector-note">Derived from Cargo manifests. Includes build, development, and platform dependencies; this is not a runtime access list.</p></details>` : ''}
      ${overview ? `<button type="button" class="inspector-explore" data-owner="${escape(row.id)}">Find this in the ownership map <span aria-hidden="true">↗</span></button>` : ''}
      ${row.proposed ? '<p class="inspector-note">One core service per user and runtime namespace. Each device owns its identity, grants, and live sessions.</p>' : ''}`;
  }

  function selectNode(node) {
    selectedNode = node;
    $$('[data-node]').forEach(button => {
      const active = button.dataset.node === selectedNode;
      button.classList.toggle('selected', active);
      button.setAttribute('aria-pressed', String(active));
    });
    inspect(rowsById.get(data.nodes[node]), $('#overview-detail'), true);
  }

  function matchingRows() {
    const terms = $('#owner-search').value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return data.rows.filter(row => row.category === category && terms.every(term => `${row.name} ${row.package ?? ''} ${row.source ?? ''} ${row.fields.map(field => plain(field.text)).join(' ')}`.toLowerCase().includes(term)));
  }

  function renderOwners() {
    const visible = matchingRows();
    if (!visible.some(row => row.id === selectedOwner)) selectedOwner = visible[0]?.id ?? null;
    $$('[data-category]').forEach(button => { button.setAttribute('aria-pressed', String(button.dataset.category === category)); });
    $('#list-title').textContent = categoryNames[category].toUpperCase();
    $('#result-count').textContent = `${visible.length} of ${data.counts[category]}`;
    $('#owner-list').innerHTML = visible.length ? visible.map(row => `<button type="button" class="owner-row${row.id === selectedOwner ? ' selected' : ''}" data-row="${escape(row.id)}" aria-pressed="${row.id === selectedOwner}"><div><strong>${escape(row.name)}${row.proposed ? '<span class="row-proposed">PROPOSED</span>' : ''}</strong><p>${escape(plain(row.fields[0].text))}</p></div><span class="row-arrow" aria-hidden="true">↗</span></button>`).join('') : '<p class="empty-state">No matching owners in this category.<br>Try another term or category.</p>';
    if (selectedOwner) inspect(rowsById.get(selectedOwner), $('#owner-detail'));
    else $('#owner-detail').innerHTML = '<div class="inspector-kicker"><span class="eyebrow">OWNERSHIP MAP</span></div><h2>Keep exploring.</h2><section class="fact"><p>Search by a plugin, a fact, a shared crate, or a source path. Select an entry to see its ownership boundary.</p></section>';
  }

  function openOwner(id) {
    const row = rowsById.get(id);
    if (!row) return;
    selectedOwner = id;
    category = row.category;
    $('#owner-search').value = '';
    renderOwners();
    showView('owners');
    $('#owner-detail').scrollIntoView({ block: 'nearest' });
    $('#owner-detail').setAttribute('tabindex', '-1');
    $('#owner-detail').focus({ preventScroll: true });
  }

  function scenarioLimit() {
    return $('#scenario').value === 'success' ? 4 : 1;
  }

  function renderPlayButton() {
    const button = $('#step-play');
    button.innerHTML = `${icon(timer === null ? 'play' : 'pause')} ${timer !== null ? 'Pause' : step >= scenarioLimit() ? 'Replay' : step === 0 ? 'Play walkthrough' : 'Continue'}`;
    button.setAttribute('aria-label', timer !== null ? 'Pause walkthrough' : step >= scenarioLimit() ? 'Replay walkthrough' : 'Play walkthrough');
  }

  function renderHandoff() {
    const scenario = $('#scenario').value;
    const blocked = scenario !== 'success' && step >= 1;
    const offline = scenario === 'offline' && blocked;
    const laptopConnected = step < 2 || blocked;
    const desktopConnected = step >= 3 && !blocked;
    const states = {
      phone: ['connected', 'Connected'],
      laptop: offline ? ['unknown', 'Status unknown'] : laptopConnected ? ['connected', 'Connected'] : ['', 'Released'],
      desktop: desktopConnected ? ['connected', 'Connected'] : step === 2 ? ['pending', 'Ready to connect'] : ['', 'Not connected'],
    };
    $$('[data-device]').forEach(element => {
      const [status, label] = states[element.dataset.device];
      element.className = `sim-device ${status}`;
      element.querySelector('small').textContent = label;
    });
    $('#slot-count').textContent = offline ? 'Laptop connection cannot be confirmed' : `${1 + Number(laptopConnected) + Number(desktopConnected)} of 2 slots in use`;
    const messages = [
      ['Two slots are already occupied.', 'The phone and laptop are connected. This example assumes the devices are linked in QoL and both have previously paired with the earbuds.'],
      ['The desktop finds an authorized connection.', 'Core obtains a fresh observation from the laptop. Bluetooth matches the peripheral by its domain identity; the device name is not enough.'],
      ['The laptop makes a slot available.', 'Laptop Bluetooth verifies the release and holds its own automatic reconnect. The hold is owned by the handoff operation and expires if the workflow stops.'],
      ['The desktop connects and checks audio.', 'Desktop Bluetooth uses its existing local operation, then verifies link and audio readiness. The phone stays connected in this illustration.'],
      ['Bluetooth reports the verified result.', 'The temporary hold is cleared according to the handoff result. Core returns that result; a delivered request alone never counts as success.'],
    ];
    const message = blocked ? scenario === 'offline'
      ? ['The handoff stops: laptop unavailable.', 'Core cannot obtain a fresh authenticated observation. The laptop connection is unknown, the release is not attempted, and the desktop does not claim success.']
      : ['The handoff stops: permission missing.', 'The laptop’s core denies the required Bluetooth access. Pairing devices does not grant every plugin action. No release is requested.']
      : messages[step];
    $('#simulation-message').classList.toggle('blocked', blocked);
    $('#simulation-message').innerHTML = `<strong>${escape(message[0])}</strong><p>${escape(message[1])}</p>`;
    $('#handoff-steps').innerHTML = data.handoff.map((description, index) => {
      const number = index + 1;
      const completed = number < step;
      return `<li><button type="button" class="walkthrough-step${completed ? ' completed' : ''}" data-step="${number}"${number === step ? ' aria-current="step"' : ''}${number > scenarioLimit() ? ' disabled' : ''}><span class="step-number">${completed ? '✓' : number.toString().padStart(2, '0')}</span><span><strong>${escape(stepTitles[index])}</strong><p>${inline(description)}</p></span></button></li>`;
    }).join('');
    $('#step-prev').disabled = step === 0;
    $('#step-next').disabled = step >= scenarioLimit();
    $('#step-counter').textContent = blocked ? 'Stopped at step 1' : `Step ${step} of 4`;
    renderPlayButton();
  }

  function setStep(next, manual = true) {
    if (manual) stopPlayback();
    step = Math.max(0, Math.min(scenarioLimit(), next));
    if (step >= scenarioLimit()) stopPlayback();
    renderHandoff();
  }

  $('#plugin-count').textContent = data.counts.plugins;
  $('#library-count').textContent = data.counts.libraries;
  $('#design-source').href = data.design;
  $('#status-source').href = `${data.design}#implementation-status`;
  $('#status-head').innerHTML = `<tr><th scope="col">Capability</th>${data.implementation.rows[0].fields.map(field => `<th scope="col">${escape(field.label)}</th>`).join('')}</tr>`;
  $('#status-rows').innerHTML = data.implementation.rows.map(row => `<tr><th scope="row">${escape(row.name)}</th>${row.fields.map(field => `<td>${inline(field.text)}</td>`).join('')}</tr>`).join('');
  const delivery = data.implementation.rows.find(row => row.id === data.implementation.delivery);
  $('#delivery-status').innerHTML = `${escape(delivery.name)}: ${inline(delivery.fields[1].text)}. ${inline(delivery.fields[2].text)}`;
  $('#change-count').textContent = `${data.gaps.length} ownership boundaries`;
  $$('[data-category]').forEach(button => { button.querySelector('span').textContent = data.counts[button.dataset.category]; });
  $('#change-list').innerHTML = `${data.gaps.map(gap => `<p>${inline(gap)}</p>`).join('')}<section class="mission-constraints"><h3>Portable and Resident follow the same ownership rules.</h3><ul>${data.mission.map(item => `<li>${inline(item)}</li>`).join('')}</ul></section>`;

  document.addEventListener('click', event => {
    const button = event.target.closest('button');
    if (!button || button.disabled) return;
    if (button.dataset.view) showView(button.dataset.view);
    if (button.dataset.node) selectNode(button.dataset.node);
    if (button.dataset.owner) openOwner(button.dataset.owner);
    if (button.dataset.row) {
      selectedOwner = button.dataset.row;
      renderOwners();
      $(`[data-row="${selectedOwner}"]`).focus({ preventScroll: true });
    }
    if (button.dataset.category || button.dataset.categoryLink) {
      category = button.dataset.category ?? button.dataset.categoryLink;
      if (button.dataset.categoryLink) $('#owner-search').value = '';
      selectedOwner = null;
      renderOwners();
      showView('owners');
    }
    if (button.dataset.step) setStep(Number(button.dataset.step));
  });
  $('#owner-search').addEventListener('input', renderOwners);
  $('#scenario').addEventListener('change', () => setStep(0));
  $('#step-reset').addEventListener('click', () => setStep(0));
  $('#step-prev').addEventListener('click', () => setStep(step - 1));
  $('#step-next').addEventListener('click', () => setStep(step + 1));
  $('#step-play').addEventListener('click', () => {
    if (timer !== null) {
      stopPlayback();
      return;
    }
    if (step >= scenarioLimit()) step = 0;
    setStep(step + 1, false);
    if (step < scenarioLimit()) timer = setInterval(() => setStep(step + 1, false), 2400);
    renderPlayButton();
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && document.activeElement === $('#owner-search')) {
      $('#owner-search').value = '';
      renderOwners();
    }
    if (event.key !== '/' || event.ctrlKey || event.metaKey || event.altKey || /INPUT|TEXTAREA|SELECT/.test(document.activeElement?.tagName) || document.activeElement?.isContentEditable) return;
    event.preventDefault();
    showView('owners');
    $('#owner-search').focus();
  });
  window.addEventListener('hashchange', () => {
    const view = location.hash.slice(1);
    if (viewCopy[view]) showView(view, false);
  });
  document.addEventListener('visibilitychange', () => { if (document.hidden) stopPlayback(); });

  selectNode('core');
  renderOwners();
  renderHandoff();
  showView(location.hash.slice(1) || 'status', false);
})();
